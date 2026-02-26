use std::cmp::{max, min};

/// Statistics from executing one block, used to tune backpressure.
#[derive(Debug, Clone, Default)]
pub struct BlockExecStats {
    pub total_executions: usize,
    pub total_aborts: usize,
    pub total_waits: usize,
}

impl BlockExecStats {
    pub fn abort_rate(&self) -> f64 {
        if self.total_executions == 0 {
            0.0
        } else {
            self.total_aborts as f64 / self.total_executions as f64
        }
    }

    pub fn wait_rate(&self) -> f64 {
        if self.total_executions == 0 {
            0.0
        } else {
            self.total_waits as f64 / self.total_executions as f64
        }
    }
}

/// Adaptive backpressure controller.
///
/// Dynamically adjusts the speculative execution window W = exec_idx - val_idx.
/// When abort/wait rates are high, shrinks the window to reduce wasted work.
/// When rates are low, expands to increase parallelism.
#[derive(Debug, Clone)]
pub struct BackpressureController {
    pub w: usize,
    pub w_min: usize,
    pub w_max: usize,
    pub gamma_down: f64,
    pub gamma_up: f64,
    pub eta_abort: f64,
    pub eta_wait: f64,
}

impl BackpressureController {
    pub fn new(w_initial: usize, w_min: usize, w_max: usize) -> Self {
        Self {
            w: w_initial,
            w_min,
            w_max,
            gamma_down: 0.8,
            gamma_up: 1.2,
            eta_abort: 0.1,
            eta_wait: 0.1,
        }
    }

    /// Adjust the window size based on block execution statistics.
    pub fn adjust(&mut self, stats: &BlockExecStats) {
        let abort_rate = stats.abort_rate();
        let wait_rate = stats.wait_rate();

        if abort_rate > self.eta_abort || wait_rate > self.eta_wait {
            // High contention: shrink window.
            self.w = max(self.w_min, (self.w as f64 * self.gamma_down) as usize);
        } else if abort_rate < self.eta_abort / 2.0 && wait_rate < self.eta_wait / 2.0 {
            // Low contention: expand window.
            self.w = min(self.w_max, (self.w as f64 * self.gamma_up) as usize);
        }
    }

    /// Current window size.
    pub fn window(&self) -> usize {
        self.w
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shrink_on_high_abort() {
        let mut ctrl = BackpressureController::new(32, 4, 64);
        let stats = BlockExecStats {
            total_executions: 100,
            total_aborts: 20, // 20% abort rate > 10% threshold
            total_waits: 0,
        };
        ctrl.adjust(&stats);
        assert!(ctrl.w < 32);
    }

    #[test]
    fn test_expand_on_low_contention() {
        let mut ctrl = BackpressureController::new(32, 4, 64);
        let stats = BlockExecStats {
            total_executions: 100,
            total_aborts: 1, // 1% < 5% (half of eta_abort)
            total_waits: 1,
        };
        ctrl.adjust(&stats);
        assert!(ctrl.w > 32);
    }

    #[test]
    fn test_min_bound() {
        let mut ctrl = BackpressureController::new(4, 4, 64);
        let stats = BlockExecStats {
            total_executions: 100,
            total_aborts: 50,
            total_waits: 0,
        };
        ctrl.adjust(&stats);
        assert_eq!(ctrl.w, 4); // Can't go below w_min
    }

    #[test]
    fn test_max_bound() {
        let mut ctrl = BackpressureController::new(60, 4, 64);
        let stats = BlockExecStats {
            total_executions: 100,
            total_aborts: 0,
            total_waits: 0,
        };
        ctrl.adjust(&stats);
        assert!(ctrl.w <= 64); // Can't exceed w_max
    }
}
