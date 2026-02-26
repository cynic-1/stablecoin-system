use crate::{
    backpressure::{BackpressureController, BlockExecStats},
    config::LeapConfig,
    errors::*,
    outcome_array::OutcomeArray,
    scheduler::{Scheduler, SchedulerTask, TaskGuard, TxnIndex, Version},
    task::{ExecutionStatus, ExecutorTask, Transaction, TransactionOutput},
    txn_last_input_output::{ReadDescriptor, TxnLastInputOutput},
};
use mvhashmap::MVHashMap;
use rayon::scope;
use std::{
    collections::HashSet,
    hash::Hash,
    marker::PhantomData,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
        Arc,
    },
    thread::spawn,
};

// Re-export mvmemory as the mvhashmap module for internal use.
use crate::mvmemory as mvhashmap;

/// Per-thread view into the multi-version data structure.
/// Resolves reads from the versioned map and tracks the read set.
pub struct MVHashMapView<'a, K, V> {
    versioned_map: &'a MVHashMap<K, V>,
    txn_idx: TxnIndex,
    scheduler: &'a Scheduler,
    read_dependency: AtomicBool,
    captured_reads: Mutex<Vec<ReadDescriptor<K>>>,
}

impl<'a, K: PartialOrd + Send + Clone + Hash + Eq, V: Send + Sync> MVHashMapView<'a, K, V> {
    pub fn take_reads(&self) -> Vec<ReadDescriptor<K>> {
        let mut reads = self.captured_reads.lock().unwrap();
        std::mem::take(&mut reads)
    }

    pub fn read(&self, key: &K) -> anyhow::Result<Option<Arc<V>>> {
        loop {
            match self.versioned_map.read(key, self.txn_idx) {
                Ok((version, v)) => {
                    let (txn_idx, incarnation) = version;
                    self.captured_reads.lock().unwrap().push(
                        ReadDescriptor::from(key.clone(), txn_idx, incarnation),
                    );
                    return Ok(Some(v));
                }
                Err(None) => {
                    self.captured_reads
                        .lock()
                        .unwrap()
                        .push(ReadDescriptor::from_storage(key.clone()));
                    return Ok(None);
                }
                Err(Some(dep_idx)) => {
                    if self.scheduler.try_add_dependency(self.txn_idx, dep_idx) {
                        self.read_dependency.store(true, Ordering::Relaxed);
                        anyhow::bail!("Read dependency not computed, retry later")
                    }
                    // Dependency resolved, retry read.
                }
            }
        }
    }

    pub fn txn_idx(&self) -> TxnIndex {
        self.txn_idx
    }

    pub fn read_dependency(&self) -> bool {
        self.read_dependency.load(Ordering::Relaxed)
    }
}

pub struct ParallelTransactionExecutor<T: Transaction, E: ExecutorTask> {
    num_cpus: usize,
    /// Persistent backpressure controller (adjusted between blocks).
    bp_controller: Mutex<Option<BackpressureController>>,
    /// Domain-aware segment boundaries for the scheduler.
    segment_bounds: Vec<(usize, usize, bool)>,
    /// O(1) txn-to-segment lookup array.
    txn_to_segment: Vec<usize>,
    phantom: PhantomData<(T, E)>,
}

impl<T, E> ParallelTransactionExecutor<T, E>
where
    T: Transaction,
    E: ExecutorTask<T = T>,
{
    pub fn new() -> Self {
        Self {
            num_cpus: num_cpus::get(),
            bp_controller: Mutex::new(None),
            segment_bounds: Vec::new(),
            txn_to_segment: Vec::new(),
            phantom: PhantomData,
        }
    }

    pub fn with_config(config: LeapConfig) -> Self {
        let num_workers = if config.num_workers > 0 {
            config.num_workers
        } else {
            num_cpus::get()
        };
        let bp = if config.enable_backpressure {
            // Scale window with thread count: threads need enough headroom to
            // stay busy while validation (sequential) catches up.
            let w_init = std::cmp::max(config.w_initial, num_workers * 8);
            let w_max = std::cmp::max(config.w_max, num_workers * 32);
            Some(BackpressureController::new(w_init, config.w_min, w_max))
        } else {
            None
        };
        Self {
            num_cpus: num_workers,
            bp_controller: Mutex::new(bp),
            segment_bounds: Vec::new(),
            txn_to_segment: Vec::new(),
            phantom: PhantomData,
        }
    }

    /// Set domain-aware segment boundaries for the scheduler.
    pub fn set_segment_bounds(&mut self, bounds: Vec<(usize, usize, bool)>, txn_to_segment: Vec<usize>) {
        self.segment_bounds = bounds;
        self.txn_to_segment = txn_to_segment;
    }

    fn execute_one<'a>(
        &self,
        version_to_execute: Version,
        guard: TaskGuard<'a>,
        block: &[T],
        last_io: &TxnLastInputOutput<
            <T as Transaction>::Key,
            <E as ExecutorTask>::Output,
            <E as ExecutorTask>::Error,
        >,
        versioned_data: &MVHashMap<<T as Transaction>::Key, <T as Transaction>::Value>,
        scheduler: &'a Scheduler,
        executor: &E,
    ) -> SchedulerTask<'a> {
        let (idx, incarnation) = version_to_execute;
        let txn = &block[idx];

        // Pre-check read dependencies from previous incarnation.
        if let Some(read_set) = last_io.read_set(idx) {
            if read_set.iter().any(
                |r| match versioned_data.read(r.path(), idx) {
                    Err(Some(dep_idx)) => scheduler.try_add_dependency(idx, dep_idx),
                    Ok(_) | Err(None) => false,
                },
            ) {
                return SchedulerTask::NoTask;
            }
        }

        let state_view = MVHashMapView {
            versioned_map: versioned_data,
            txn_idx: idx,
            scheduler,
            read_dependency: AtomicBool::new(false),
            captured_reads: Mutex::new(Vec::new()),
        };

        let execute_result = executor.execute_transaction(&state_view, txn);

        if state_view.read_dependency() {
            return SchedulerTask::NoTask;
        }

        let mut prev_write_set: HashSet<T::Key> = last_io.write_set(idx);
        let mut writes_outside = false;

        let mut apply_writes = |output: &<E as ExecutorTask>::Output| {
            let write_version = (idx, incarnation);
            for (k, v) in output.get_writes().into_iter() {
                if !prev_write_set.remove(&k) {
                    writes_outside = true;
                }
                versioned_data.write(&k, write_version, v);
            }
        };

        let result = match execute_result {
            ExecutionStatus::Success(output) => {
                apply_writes(&output);
                ExecutionStatus::Success(output)
            }
            ExecutionStatus::SkipRest(output) => {
                apply_writes(&output);
                scheduler.set_stop_idx(idx + 1);
                ExecutionStatus::SkipRest(output)
            }
            ExecutionStatus::Abort(err) => {
                scheduler.set_stop_idx(idx + 1);
                ExecutionStatus::Abort(Error::UserError(err))
            }
        };

        for k in &prev_write_set {
            versioned_data.delete(k, idx);
        }

        last_io.record(idx, state_view.take_reads(), result);
        scheduler.finish_execution(idx, incarnation, writes_outside, guard)
    }

    fn validate_one<'a>(
        &self,
        version_to_validate: Version,
        guard: TaskGuard<'a>,
        last_io: &TxnLastInputOutput<
            <T as Transaction>::Key,
            <E as ExecutorTask>::Output,
            <E as ExecutorTask>::Error,
        >,
        versioned_data: &MVHashMap<<T as Transaction>::Key, <T as Transaction>::Value>,
        scheduler: &'a Scheduler,
    ) -> SchedulerTask<'a> {
        let (idx, incarnation) = version_to_validate;
        let read_set = last_io
            .read_set(idx)
            .expect("Prior read-set must be recorded");

        let valid = read_set.iter().all(|r| {
            match versioned_data.read(r.path(), idx) {
                Ok((version, _)) => r.validate_version(version),
                Err(Some(_)) => false,
                Err(None) => r.validate_storage(),
            }
        });

        let aborted = !valid && scheduler.try_abort(idx, incarnation);

        if aborted {
            for k in &last_io.write_set(idx) {
                versioned_data.mark_estimate(k, idx);
            }
            scheduler.finish_abort(idx, incarnation, guard)
        } else {
            SchedulerTask::NoTask
        }
    }

    pub fn execute_transactions_parallel(
        &self,
        executor_args: E::Argument,
        block: Vec<T>,
    ) -> Result<Vec<E::Output>, E::Error> {
        if block.is_empty() {
            return Ok(vec![]);
        }

        let num_txns = block.len();
        let versioned_data = MVHashMap::new();
        let outcomes = OutcomeArray::new(num_txns);
        let compute_cpus = self.num_cpus;
        let last_io = TxnLastInputOutput::new(num_txns);

        let bp_window = {
            let ctrl = self.bp_controller.lock().unwrap();
            ctrl.as_ref().map_or(0, |c| c.window())
        };
        let scheduler = if self.segment_bounds.is_empty() {
            Scheduler::with_backpressure(num_txns, bp_window)
        } else {
            Scheduler::with_domain_plan(num_txns, bp_window, self.segment_bounds.clone(), self.txn_to_segment.clone())
        };

        scope(|s| {
            for _ in 0..compute_cpus {
                s.spawn(|_| {
                    let executor = E::init(executor_args.clone());
                    let mut task = SchedulerTask::NoTask;
                    loop {
                        task = match task {
                            SchedulerTask::ValidationTask(version, guard) => self
                                .validate_one(version, guard, &last_io, &versioned_data, &scheduler),
                            SchedulerTask::ExecutionTask(version, guard) => self
                                .execute_one(
                                    version, guard, &block, &last_io, &versioned_data,
                                    &scheduler, &executor,
                                ),
                            SchedulerTask::NoTask => scheduler.next_task(),
                            SchedulerTask::Done => break,
                        }
                    }
                });
            }
        });

        // Adaptive backpressure: adjust window based on this block's stats.
        {
            let mut ctrl = self.bp_controller.lock().unwrap();
            if let Some(ref mut controller) = *ctrl {
                let (executions, aborts, waits) = scheduler.exec_stats();
                let stats = BlockExecStats {
                    total_executions: executions,
                    total_aborts: aborts,
                    total_waits: waits,
                };
                controller.adjust(&stats);
            }
        }

        let valid_results = scheduler.num_txn_to_execute();
        let chunk_size = std::cmp::max(1, (valid_results + 4 * compute_cpus - 1) / (4 * compute_cpus));
        use rayon::prelude::*;
        (0..valid_results)
            .collect::<Vec<TxnIndex>>()
            .par_chunks(chunk_size)
            .for_each(|chunk| {
                for idx in chunk {
                    outcomes.set_result(*idx, last_io.take_output(*idx));
                }
            });

        spawn(move || {
            drop(last_io);
            drop(block);
            drop(versioned_data);
            drop(scheduler);
        });

        outcomes.get_all_results(valid_results)
    }
}
