use crate::executor::MVHashMapView;
use std::{fmt::Debug, hash::Hash};

/// Execution result of a transaction.
#[derive(Debug)]
pub enum ExecutionStatus<T, E> {
    Success(T),
    Abort(E),
    SkipRest(T),
}

/// A transaction that can be parallel-executed by the scheduler.
pub trait Transaction: Sync + Send + 'static {
    type Key: PartialOrd + Send + Sync + Clone + Hash + Eq + Debug;
    type Value: Send + Sync + Clone;
}

/// Single-threaded transaction executor (one per worker thread).
pub trait ExecutorTask: Sync {
    type T: Transaction;
    type Output: TransactionOutput<T = Self::T> + 'static;
    type Error: Clone + Send + Sync + Debug + 'static;
    type Argument: Sync + Send + Clone;

    fn init(args: Self::Argument) -> Self;

    fn execute_transaction(
        &self,
        view: &MVHashMapView<<Self::T as Transaction>::Key, <Self::T as Transaction>::Value>,
        txn: &Self::T,
    ) -> ExecutionStatus<Self::Output, Self::Error>;
}

/// Trait for execution output — provides the write set.
pub trait TransactionOutput: Send + Sync {
    type T: Transaction;

    fn get_writes(
        &self,
    ) -> Vec<(
        <Self::T as Transaction>::Key,
        <Self::T as Transaction>::Value,
    )>;

    fn skip_output() -> Self;
}
