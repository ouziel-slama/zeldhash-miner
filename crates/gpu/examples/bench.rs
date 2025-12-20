use std::time::Instant;

use zeldhash_miner_gpu::{
    calibrate_batch_size, dispatch_mining_batch, GpuContext, GpuError, MiningBatch,
};

const START_NONCE: u64 = 1 << 32; // keep a stable nonce byte-length across batch sizes

struct BenchSample {
    elapsed_ms: f64,
    hashes_per_sec: f64,
}

async fn bench_batch(ctx: &GpuContext, batch_size: u32) -> Result<BenchSample, GpuError> {
    let batch = MiningBatch {
        tx_prefix: b"bench-prefix",
        tx_suffix: b"bench-suffix",
        start_nonce: START_NONCE,
        batch_size,
        target_zeros: 64, // effectively impossible target; keeps the kernel busy
        use_cbor_nonce: false,
    };

    let start = Instant::now();
    // Ignore results; we only care about throughput.
    let _ = dispatch_mining_batch(ctx, &batch).await?;
    let elapsed = start.elapsed().as_secs_f64().max(1e-9);

    Ok(BenchSample {
        elapsed_ms: elapsed * 1_000.0,
        hashes_per_sec: batch_size as f64 / elapsed,
    })
}

async fn memory_pressure_check(ctx: &GpuContext, batch_size: u32) -> Result<(), GpuError> {
    // Use large buffers to validate allocation/dispatch under heavier pressure.
    let blob = vec![0u8; 512 * 1024];
    for i in 0..3 {
        let batch = MiningBatch {
            tx_prefix: &blob,
            tx_suffix: &blob,
            start_nonce: START_NONCE + (i as u64) * batch_size as u64,
            batch_size,
            target_zeros: 64,
            use_cbor_nonce: false,
        };
        let _ = dispatch_mining_batch(ctx, &batch).await?;
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = pollster::block_on(GpuContext::init())?;
    let summary = ctx.adapter_summary();
    println!(
        "Adapter: {} | backend={} | type={}",
        summary.name, summary.backend, summary.device_type
    );

    let calibrated = pollster::block_on(calibrate_batch_size(&ctx))?;
    println!("Calibrated batch size: {}", calibrated);

    for size in [10_000u32, 100_000, 1_000_000] {
        let sample = pollster::block_on(bench_batch(&ctx, size))?;
        println!(
            "Batch {:>7}: {:>7.2} ms | {:>8.2} MH/s",
            size,
            sample.elapsed_ms,
            sample.hashes_per_sec / 1_000_000.0
        );
    }

    let pressure_size = calibrated.min(1_000_000);
    pollster::block_on(memory_pressure_check(&ctx, pressure_size))?;
    println!(
        "Memory pressure: 3x batches (size {}) with 512 KiB prefix/suffix succeeded",
        pressure_size
    );

    Ok(())
}
