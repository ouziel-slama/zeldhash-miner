fn main() {
    // GPU builds stay on stable by sticking to the WGSL path only.
    let target_arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    if target_arch != "wasm32" {
        println!("cargo:warning=Native GPU backend disabled; build uses WGSL");
    }
}
