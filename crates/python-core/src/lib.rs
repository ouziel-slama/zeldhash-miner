#![allow(dead_code)]

pub use zeldhash_miner_core as core;

#[cfg(feature = "gpu")]
pub use zeldhash_miner_gpu as gpu;

/// Placeholder for the future pyo3-based Python bindings.
pub fn placeholder() -> &'static str {
    "python bindings not yet implemented"
}
