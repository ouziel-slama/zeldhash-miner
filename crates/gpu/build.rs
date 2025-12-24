fn main() {
    // Skip SPIR-V generation unless the native-spirv feature is enabled.
    #[cfg(feature = "native-spirv")]
    build_spirv();

    #[cfg(not(feature = "native-spirv"))]
    {
        let target_arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
        if target_arch != "wasm32" {
            println!(
                "cargo:warning=Skipping SPIR-V build (native-spirv feature disabled or wasm target)"
            );
        }
    }
}

#[cfg(feature = "native-spirv")]
fn build_spirv() {
    use spirv_builder::{Capability, MetadataPrintout, ModuleResult, SpirvBuilder};

    let target_arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    if target_arch == "wasm32" {
        println!("cargo:warning=Skipping SPIR-V build for wasm target");
        return;
    }

    // Kernel crate lives under crates/kernel after repo layout move.
    let result = SpirvBuilder::new("../kernel", "spirv-unknown-spv1.5")
        .capability(Capability::Int8)
        .capability(Capability::Int64)
        .capability(Capability::VariablePointers)
        .capability(Capability::VariablePointersStorageBuffer)
        .print_metadata(MetadataPrintout::Full)
        .build()
        .expect("failed to build zeldhash-miner-kernel spirv");

    let spv_path_ref = match &result.module {
        ModuleResult::SingleModule(path) => path,
        ModuleResult::MultiModule(map) => map
            .get("main")
            .or_else(|| map.values().next())
            .expect("spirv-builder did not produce a module"),
    };

    let spv_path = spv_path_ref
        .to_str()
        .expect("spirv path is not valid utf-8")
        .to_string();

    println!("cargo:rustc-env=ZELD_KERNEL_SPV={spv_path}");
}
