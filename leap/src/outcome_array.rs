use crate::{
    errors::{Error, Result},
    task::{ExecutionStatus, TransactionOutput},
};
use once_cell::sync::OnceCell;

pub struct OutcomeArray<T, E> {
    results: Vec<OnceCell<ExecutionStatus<T, Error<E>>>>,
}

impl<T: TransactionOutput, E: Send> OutcomeArray<T, E> {
    pub fn new(len: usize) -> Self {
        OutcomeArray {
            results: (0..len).map(|_| OnceCell::new()).collect(),
        }
    }

    pub fn set_result(&self, idx: usize, res: ExecutionStatus<T, Error<E>>) {
        let entry = &self.results[idx];
        assert!(entry.set(res).is_ok());
    }

    pub fn get_all_results(self, stop_at: usize) -> Result<Vec<T>, E> {
        let len = self.results.len();
        let mut final_results = Vec::with_capacity(stop_at);
        for (idx, status) in self.results.into_iter().take(stop_at).enumerate() {
            let t = match status.into_inner() {
                Some(ExecutionStatus::Success(t)) => t,
                Some(ExecutionStatus::SkipRest(t)) if idx == stop_at - 1 => t,
                Some(ExecutionStatus::SkipRest(_)) => return Err(Error::InvariantViolation),
                Some(ExecutionStatus::Abort(err)) => return Err(err),
                None => return Err(Error::InvariantViolation),
            };
            final_results.push(t)
        }
        assert!(final_results.len() == stop_at);
        final_results.resize_with(len, T::skip_output);
        Ok(final_results)
    }
}
