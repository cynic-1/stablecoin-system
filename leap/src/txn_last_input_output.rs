use crate::{
    errors::Error,
    scheduler::{Incarnation, TxnIndex, Version},
    task::{ExecutionStatus, Transaction, TransactionOutput},
};
use arc_swap::ArcSwapOption;
use crossbeam::utils::CachePadded;
use std::{collections::HashSet, sync::Arc};

type TxnInput<K> = Vec<ReadDescriptor<K>>;
type TxnOutput<T, E> = ExecutionStatus<T, Error<E>>;

#[derive(Clone, PartialEq)]
enum ReadKind {
    MVHashMap(TxnIndex, Incarnation),
    Storage,
}

#[derive(Clone)]
pub struct ReadDescriptor<K> {
    access_path: K,
    kind: ReadKind,
}

impl<K> ReadDescriptor<K> {
    pub fn from(access_path: K, txn_idx: TxnIndex, incarnation: Incarnation) -> Self {
        Self {
            access_path,
            kind: ReadKind::MVHashMap(txn_idx, incarnation),
        }
    }

    pub fn from_storage(access_path: K) -> Self {
        Self {
            access_path,
            kind: ReadKind::Storage,
        }
    }

    pub fn path(&self) -> &K {
        &self.access_path
    }

    pub fn validate_version(&self, version: Version) -> bool {
        let (txn_idx, incarnation) = version;
        self.kind == ReadKind::MVHashMap(txn_idx, incarnation)
    }

    pub fn validate_storage(&self) -> bool {
        self.kind == ReadKind::Storage
    }
}

pub struct TxnLastInputOutput<K, T, E> {
    inputs: Vec<CachePadded<ArcSwapOption<TxnInput<K>>>>,
    outputs: Vec<CachePadded<ArcSwapOption<TxnOutput<T, E>>>>,
}

impl<K, T: TransactionOutput, E: Send + Clone> TxnLastInputOutput<K, T, E> {
    pub fn new(num_txns: usize) -> Self {
        Self {
            inputs: (0..num_txns)
                .map(|_| CachePadded::new(ArcSwapOption::empty()))
                .collect(),
            outputs: (0..num_txns)
                .map(|_| CachePadded::new(ArcSwapOption::empty()))
                .collect(),
        }
    }

    pub fn record(
        &self,
        txn_idx: TxnIndex,
        input: Vec<ReadDescriptor<K>>,
        output: ExecutionStatus<T, Error<E>>,
    ) {
        self.inputs[txn_idx].store(Some(Arc::new(input)));
        self.outputs[txn_idx].store(Some(Arc::new(output)));
    }

    pub fn read_set(&self, txn_idx: TxnIndex) -> Option<Arc<Vec<ReadDescriptor<K>>>> {
        self.inputs[txn_idx].load_full()
    }

    pub fn write_set(
        &self,
        txn_idx: TxnIndex,
    ) -> HashSet<<<T as TransactionOutput>::T as Transaction>::Key> {
        match &self.outputs[txn_idx].load_full() {
            None => HashSet::new(),
            Some(txn_output) => match txn_output.as_ref() {
                ExecutionStatus::Success(t) | ExecutionStatus::SkipRest(t) => {
                    t.get_writes().into_iter().map(|(k, _)| k).collect()
                }
                ExecutionStatus::Abort(_) => HashSet::new(),
            },
        }
    }

    pub fn take_output(&self, txn_idx: TxnIndex) -> ExecutionStatus<T, Error<E>> {
        let owning_ptr = self.outputs[txn_idx]
            .swap(None)
            .expect("Output must be recorded after execution");
        if let Ok(output) = Arc::try_unwrap(owning_ptr) {
            output
        } else {
            unreachable!("Output should be uniquely owned after execution");
        }
    }
}
