use crossbeam::utils::CachePadded;
use std::{
    cmp::min,
    hint,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Mutex,
    },
};

pub type TxnIndex = usize;
pub type Incarnation = usize;
pub type Version = (TxnIndex, Incarnation);

/// RAII guard that tracks active tasks.
pub struct TaskGuard<'a> {
    counter: &'a AtomicUsize,
}

impl<'a> TaskGuard<'a> {
    pub fn new(counter: &'a AtomicUsize) -> Self {
        counter.fetch_add(1, Ordering::SeqCst);
        Self { counter }
    }
}

impl Drop for TaskGuard<'_> {
    fn drop(&mut self) {
        assert!(self.counter.fetch_sub(1, Ordering::SeqCst) > 0);
    }
}

/// Task returned from the scheduler.
pub enum SchedulerTask<'a> {
    ExecutionTask(Version, TaskGuard<'a>),
    ValidationTask(Version, TaskGuard<'a>),
    NoTask,
    Done,
}

#[derive(PartialEq)]
enum TransactionStatus {
    ReadyToExecute(Incarnation),
    Executing(Incarnation),
    Executed(Incarnation),
    Aborting(Incarnation),
}

pub struct Scheduler {
    execution_idx: AtomicUsize,
    validation_idx: AtomicUsize,
    decrease_cnt: AtomicUsize,
    num_active_tasks: AtomicUsize,
    done_marker: AtomicBool,
    stop_idx: AtomicUsize,
    txn_dependency: Vec<CachePadded<Mutex<Vec<TxnIndex>>>>,
    txn_status: Vec<CachePadded<Mutex<TransactionStatus>>>,
    /// Backpressure window limit (0 = disabled).
    backpressure_window: usize,
    /// Domain-aware segment boundaries: (start, end_exclusive, par_bound_with_prev).
    /// Empty = no domain-aware scheduling.
    segment_bounds: Vec<(usize, usize, bool)>,
    /// O(1) txn-to-segment lookup. txn_to_segment[txn_idx] = segment index.
    /// Replaces binary search in find_segment() for better performance.
    txn_to_segment: Vec<usize>,
    /// Counters for adaptive backpressure stats.
    abort_count: AtomicUsize,
    wait_count: AtomicUsize,
}

impl Scheduler {
    pub fn new(num_txns: usize) -> Self {
        Self::with_backpressure(num_txns, 0)
    }

    pub fn with_backpressure(num_txns: usize, window: usize) -> Self {
        Self {
            execution_idx: AtomicUsize::new(0),
            validation_idx: AtomicUsize::new(0),
            decrease_cnt: AtomicUsize::new(0),
            num_active_tasks: AtomicUsize::new(0),
            done_marker: AtomicBool::new(false),
            stop_idx: AtomicUsize::new(num_txns),
            txn_dependency: (0..num_txns)
                .map(|_| CachePadded::new(Mutex::new(Vec::new())))
                .collect(),
            txn_status: (0..num_txns)
                .map(|_| CachePadded::new(Mutex::new(TransactionStatus::ReadyToExecute(0))))
                .collect(),
            backpressure_window: window,
            segment_bounds: Vec::new(),
            txn_to_segment: Vec::new(),
            abort_count: AtomicUsize::new(0),
            wait_count: AtomicUsize::new(0),
        }
    }

    pub fn with_domain_plan(
        num_txns: usize,
        window: usize,
        segment_bounds: Vec<(usize, usize, bool)>,
        txn_to_segment: Vec<usize>,
    ) -> Self {
        Self {
            segment_bounds,
            txn_to_segment,
            ..Self::with_backpressure(num_txns, window)
        }
    }

    /// Return execution statistics: (total_executions, total_aborts, total_waits).
    pub fn exec_stats(&self) -> (usize, usize, usize) {
        let executions = self.stop_idx.load(Ordering::Relaxed);
        let aborts = self.abort_count.load(Ordering::Relaxed);
        let waits = self.wait_count.load(Ordering::Relaxed);
        (executions, aborts, waits)
    }

    pub fn set_stop_idx(&self, stop_idx: TxnIndex) {
        self.stop_idx.fetch_min(stop_idx, Ordering::Relaxed);
    }

    pub fn num_txn_to_execute(&self) -> usize {
        self.stop_idx.load(Ordering::Relaxed)
    }

    /// Try to abort a transaction version. Returns true if successfully transitioned
    /// Executed(incarnation) → Aborting(incarnation).
    pub fn try_abort(&self, txn_idx: TxnIndex, incarnation: Incarnation) -> bool {
        let mut status = self.txn_status[txn_idx].lock().unwrap();
        if *status == TransactionStatus::Executed(incarnation) {
            *status = TransactionStatus::Aborting(incarnation);
            true
        } else {
            false
        }
    }

    /// Return the next task for the calling thread.
    pub fn next_task(&self) -> SchedulerTask<'_> {
        loop {
            if self.done() {
                return SchedulerTask::Done;
            }

            let idx_to_validate = self.validation_idx.load(Ordering::SeqCst);
            let idx_to_execute = self.execution_idx.load(Ordering::SeqCst);

            // Backpressure: if exec is too far ahead of validation, wait.
            if self.backpressure_window > 0
                && idx_to_execute > idx_to_validate
                && idx_to_execute - idx_to_validate > self.backpressure_window
            {
                self.wait_count.fetch_add(1, Ordering::Relaxed);
                // Only try validation tasks when under backpressure.
                if let Some((version, guard)) = self.try_validate_next_version() {
                    return SchedulerTask::ValidationTask(version, guard);
                }
                hint::spin_loop();
                continue;
            }

            // Domain-aware: throttle execution at non-parallel segment boundaries.
            if !self.segment_bounds.is_empty() {
                let exec_seg = self.find_segment(idx_to_execute);
                let val_seg = self.find_segment(idx_to_validate);

                if exec_seg > val_seg {
                    // Execution is in a later segment than validation.
                    if exec_seg < self.segment_bounds.len() && !self.segment_bounds[exec_seg].2 {
                        // Non-parallel boundary: prefer validation to catch up.
                        if let Some((version, guard)) = self.try_validate_next_version() {
                            return SchedulerTask::ValidationTask(version, guard);
                        }
                        // No validation available, fall through to normal scheduling.
                    }
                }
            }

            if idx_to_validate < idx_to_execute {
                if let Some((version, guard)) = self.try_validate_next_version() {
                    return SchedulerTask::ValidationTask(version, guard);
                }
            } else if let Some((version, guard)) = self.try_execute_next_version() {
                return SchedulerTask::ExecutionTask(version, guard);
            }
        }
    }

    /// Add txn_idx as a dependency of dep_txn_idx. Returns true if dependency
    /// was recorded, false if dep_txn_idx already executed.
    pub fn try_add_dependency(&self, txn_idx: TxnIndex, dep_txn_idx: TxnIndex) -> bool {
        let mut stored_deps = self.txn_dependency[dep_txn_idx].lock().unwrap();
        if self.is_executed(dep_txn_idx).is_some() {
            return false;
        }
        stored_deps.push(txn_idx);
        true
    }

    /// After execution finishes, resolve dependencies and schedule validation.
    pub fn finish_execution<'a>(
        &self,
        txn_idx: TxnIndex,
        incarnation: Incarnation,
        revalidate_suffix: bool,
        guard: TaskGuard<'a>,
    ) -> SchedulerTask<'a> {
        self.set_executed_status(txn_idx, incarnation);

        let txn_deps: Vec<TxnIndex> = {
            let mut stored_deps = self.txn_dependency[txn_idx].lock().unwrap();
            std::mem::take(&mut stored_deps)
        };

        let min_dep = txn_deps
            .into_iter()
            .map(|dep| {
                self.resume(dep);
                dep
            })
            .min();

        if let Some(target) = min_dep {
            self.decrease_execution_idx(target);
        }

        if self.validation_idx.load(Ordering::SeqCst) > txn_idx {
            if revalidate_suffix {
                self.decrease_validation_idx(txn_idx);
            } else {
                return SchedulerTask::ValidationTask((txn_idx, incarnation), guard);
            }
        }

        SchedulerTask::NoTask
    }

    /// After a successful abort, reset transaction for re-execution.
    pub fn finish_abort<'a>(
        &self,
        txn_idx: TxnIndex,
        incarnation: Incarnation,
        guard: TaskGuard<'a>,
    ) -> SchedulerTask<'a> {
        self.abort_count.fetch_add(1, Ordering::Relaxed);
        self.set_aborted_status(txn_idx, incarnation);
        self.decrease_validation_idx(txn_idx + 1);

        if self.execution_idx.load(Ordering::SeqCst) > txn_idx {
            if let Some(new_incarnation) = self.try_incarnate(txn_idx) {
                return SchedulerTask::ExecutionTask((txn_idx, new_incarnation), guard);
            }
        }

        SchedulerTask::NoTask
    }

    // --- Private helpers ---

    fn decrease_validation_idx(&self, target_idx: TxnIndex) {
        if self.validation_idx.fetch_min(target_idx, Ordering::SeqCst) > target_idx {
            self.decrease_cnt.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn decrease_execution_idx(&self, target_idx: TxnIndex) {
        if self.execution_idx.fetch_min(target_idx, Ordering::SeqCst) > target_idx {
            self.decrease_cnt.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn try_incarnate(&self, txn_idx: TxnIndex) -> Option<Incarnation> {
        if txn_idx >= self.txn_status.len() {
            return None;
        }
        let mut status = self.txn_status[txn_idx].lock().unwrap();
        if let TransactionStatus::ReadyToExecute(incarnation) = *status {
            *status = TransactionStatus::Executing(incarnation);
            Some(incarnation)
        } else {
            None
        }
    }

    fn is_executed(&self, txn_idx: TxnIndex) -> Option<Incarnation> {
        if txn_idx >= self.txn_status.len() {
            return None;
        }
        let status = self.txn_status[txn_idx].lock().unwrap();
        if let TransactionStatus::Executed(incarnation) = *status {
            Some(incarnation)
        } else {
            None
        }
    }

    fn try_validate_next_version(&self) -> Option<(Version, TaskGuard<'_>)> {
        let idx = self.validation_idx.load(Ordering::SeqCst);
        let num_txns = self.num_txn_to_execute();
        if idx >= num_txns {
            if !self.check_done(num_txns) {
                hint::spin_loop();
            }
            return None;
        }
        let guard = TaskGuard::new(&self.num_active_tasks);
        let idx = self.validation_idx.fetch_add(1, Ordering::SeqCst);
        self.is_executed(idx)
            .map(|incarnation| ((idx, incarnation), guard))
    }

    fn try_execute_next_version(&self) -> Option<(Version, TaskGuard<'_>)> {
        let idx = self.execution_idx.load(Ordering::SeqCst);
        let num_txns = self.num_txn_to_execute();
        if idx >= num_txns {
            if !self.check_done(num_txns) {
                hint::spin_loop();
            }
            return None;
        }
        let guard = TaskGuard::new(&self.num_active_tasks);
        let idx = self.execution_idx.fetch_add(1, Ordering::SeqCst);
        self.try_incarnate(idx)
            .map(|incarnation| ((idx, incarnation), guard))
    }

    fn resume(&self, txn_idx: TxnIndex) {
        let mut status = self.txn_status[txn_idx].lock().unwrap();
        match *status {
            TransactionStatus::Executing(incarnation) => {
                // Normal case: txn bailed out due to read dependency,
                // hasn't written yet. Safe to re-schedule.
                *status = TransactionStatus::ReadyToExecute(incarnation + 1);
            }
            TransactionStatus::Executed(_) => {
                // Txn already completed. The decrease_execution_idx will cause
                // re-validation; if stale, the abort path handles re-execution.
            }
            TransactionStatus::Aborting(_) => {
                // Already being aborted; finish_abort handles re-scheduling.
            }
            TransactionStatus::ReadyToExecute(_) => {
                // Already queued for re-execution.
            }
        }
    }

    fn set_executed_status(&self, txn_idx: TxnIndex, incarnation: Incarnation) {
        let mut status = self.txn_status[txn_idx].lock().unwrap();
        debug_assert!(*status == TransactionStatus::Executing(incarnation));
        *status = TransactionStatus::Executed(incarnation);
    }

    fn set_aborted_status(&self, txn_idx: TxnIndex, incarnation: Incarnation) {
        let mut status = self.txn_status[txn_idx].lock().unwrap();
        debug_assert!(*status == TransactionStatus::Aborting(incarnation));
        *status = TransactionStatus::ReadyToExecute(incarnation + 1);
    }

    fn check_done(&self, num_txns: usize) -> bool {
        let observed_cnt = self.decrease_cnt.load(Ordering::SeqCst);
        let val_idx = self.validation_idx.load(Ordering::SeqCst);
        let exec_idx = self.execution_idx.load(Ordering::SeqCst);
        let num_tasks = self.num_active_tasks.load(Ordering::SeqCst);
        if min(exec_idx, val_idx) < num_txns || num_tasks > 0 {
            return false;
        }
        if observed_cnt == self.decrease_cnt.load(Ordering::SeqCst) {
            self.done_marker.store(true, Ordering::Release);
            true
        } else {
            false
        }
    }

    fn done(&self) -> bool {
        self.done_marker.load(Ordering::Acquire)
    }

    /// Find which segment index a transaction belongs to. O(1) via precomputed lookup.
    fn find_segment(&self, idx: usize) -> usize {
        if idx < self.txn_to_segment.len() {
            self.txn_to_segment[idx]
        } else {
            self.segment_bounds.len() // beyond last segment
        }
    }
}
