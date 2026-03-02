/// CADO ordering mode.
#[derive(Clone, Debug, PartialEq)]
pub enum CadoMode {
    /// No CADO ordering (baseline/Block-STM).
    Disabled,
    /// Group same-domain transactions together (original CADO).
    Concatenate,
    /// Round-robin interleave across domains (OCC-friendly).
    Interleave,
}

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
    pub cado_mode: CadoMode,
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
            cado_mode: CadoMode::Interleave,
            enable_domain_aware: false, // auto-disabled for Interleave
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
            cado_mode: CadoMode::Disabled,
            enable_domain_aware: false,
            enable_hot_delta: false,
            enable_backpressure: false,
            ..Self::default()
        }
    }

    /// All optimizations enabled with interleave CADO (new default).
    /// Domain-aware scheduling is auto-disabled since interleaving
    /// spreads domains — no consecutive same-domain segments exist.
    pub fn full() -> Self {
        Self::default()
    }

    /// All optimizations enabled with concatenate CADO (original behavior).
    pub fn full_concat() -> Self {
        Self {
            cado_mode: CadoMode::Concatenate,
            enable_domain_aware: true,
            enable_hot_delta: true,
            enable_backpressure: true,
            ..Self::default()
        }
    }

    /// Only domain-aware scheduling (requires concatenate CADO).
    pub fn domain_only() -> Self {
        Self {
            cado_mode: CadoMode::Concatenate,
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

    /// Whether CADO ordering should be applied.
    pub fn use_cado(&self) -> bool {
        self.cado_mode != CadoMode::Disabled
    }
}
