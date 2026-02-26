/// LEAP configuration parameters.
#[derive(Clone, Debug)]
pub struct LeapConfig {
    pub num_workers: usize,
    pub w_initial: usize,
    pub w_min: usize,
    pub w_max: usize,
    pub l_max: usize,
    pub w_scan: usize,
    pub theta_1: usize,
    pub theta_2: usize,
    pub p_max: usize,
    pub enable_domain_aware: bool,
    pub enable_hot_delta: bool,
    pub enable_backpressure: bool,
}

impl Default for LeapConfig {
    fn default() -> Self {
        Self {
            num_workers: 0, // 0 = use num_cpus
            w_initial: 32,
            w_min: 4,
            w_max: 64,
            l_max: 256,
            w_scan: 8,
            theta_1: 10,
            theta_2: 50,
            p_max: 8,
            enable_domain_aware: true,
            enable_hot_delta: true,
            enable_backpressure: true,
        }
    }
}

impl LeapConfig {
    /// All optimizations disabled (LEAP-base: same core algorithm as Block-STM
    /// but not the official Block-STM implementation).
    pub fn baseline() -> Self {
        Self {
            enable_domain_aware: false,
            enable_hot_delta: false,
            enable_backpressure: false,
            ..Self::default()
        }
    }

    /// All optimizations enabled.
    pub fn full() -> Self {
        Self::default()
    }

    /// Only domain-aware scheduling.
    pub fn domain_only() -> Self {
        Self {
            enable_domain_aware: true,
            enable_hot_delta: false,
            enable_backpressure: false,
            ..Self::default()
        }
    }

    /// Only Hot-Delta.
    pub fn hot_delta_only() -> Self {
        Self {
            enable_domain_aware: false,
            enable_hot_delta: true,
            enable_backpressure: false,
            ..Self::default()
        }
    }

    /// Only backpressure.
    pub fn backpressure_only() -> Self {
        Self {
            enable_domain_aware: false,
            enable_hot_delta: false,
            enable_backpressure: true,
            ..Self::default()
        }
    }
}
