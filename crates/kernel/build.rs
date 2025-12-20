use spirv_builder::{Capability, MetadataPrintout, SpirvBuilder};
use std::env;

fn main() {
    // When cargo builds the SPIR-V target, it would re-run this build script.
    // Guard against that or we'd recursively invoke SpirvBuilder forever.
    if env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("spirv") {
        return;
    }

    // Build the rust-gpu kernel to SPIR-V so zeldhash-miner-gpu can embed it.
    SpirvBuilder::new(".", "spirv-unknown-spv1.5")
        .capability(Capability::Int8)
        .capability(Capability::Int64)
        .capability(Capability::VariablePointers)
        .capability(Capability::VariablePointersStorageBuffer)
        .print_metadata(MetadataPrintout::Full)
        .build()
        .expect("failed to build zeldhash-miner-kernel spirv");
}
