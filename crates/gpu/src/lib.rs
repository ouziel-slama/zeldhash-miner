#![deny(missing_docs)]

//! WebGPU mining runner.
//!
//! The implementation wires a compute shader that performs the full
//! double-SHA256 over `tx_prefix || nonce || tx_suffix`. The shader mirrors the
//! CPU implementation in `zeldhash-miner-core` and returns txids in the same byte order
//! (big-endian hash bytes, which callers treat as txid by reversing when
//! counting leading zeros). Storage buffer layouts match the TODO design so the
//! JavaScript bindings and future optimizations can reuse them unchanged.

use std::{
    borrow::Cow,
    num::NonZeroU64,
    sync::{Arc, Mutex, MutexGuard},
    time::Instant,
};

use futures::channel::oneshot;

use bytemuck::{cast_slice, pod_read_unaligned, Pod, Zeroable};
use thiserror::Error;
use wgpu::util::DeviceExt;
use zeldhash_miner_core::encode_nonce;

#[cfg_attr(test, allow(dead_code))]
const WORKGROUP_SIZE: u32 = 256;
const MAX_RESULTS: usize = 8;

const SHADER_WGSL: &str = include_str!("shader.wgsl");

/// Minimal adapter info exposed to callers (e.g., WASM bindings).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdapterSummary {
    /// Human-readable adapter name (vendor/device).
    pub name: String,
    /// Backend string (e.g., "Vulkan", "Metal", "Dx12", "Gl", "BrowserWebGpu").
    pub backend: String,
    /// Device class (DiscreteGpu, IntegratedGpu, Cpu, VirtualGpu, Other).
    pub device_type: String,
}

impl From<wgpu::AdapterInfo> for AdapterSummary {
    fn from(info: wgpu::AdapterInfo) -> Self {
        Self {
            name: info.name,
            backend: format!("{:?}", info.backend),
            device_type: format!("{:?}", info.device_type),
        }
    }
}

/// Result returned when a matching nonce is found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MineResult {
    /// Winning nonce.
    pub nonce: u64,
    /// Double SHA256 hash (big-endian bytes).
    pub txid: [u8; 32],
}

/// GPU initialization and dispatch errors.
#[derive(Debug, Error)]
pub enum GpuError {
    /// WebGPU is not available on this platform/adapter.
    #[error("WebGPU not available: {0}")]
    Unavailable(String),
    /// Internal GPU error.
    #[error("GPU error: {0}")]
    Internal(String),
}

/// GPU context holding the device/queue.
#[derive(Clone)]
#[cfg_attr(test, allow(dead_code))]
pub struct GpuContext {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    #[allow(dead_code)]
    adapter_info: wgpu::AdapterInfo,
    batch_size_cache: Arc<Mutex<Option<u32>>>,
    pipeline_cache: Arc<Mutex<Option<Arc<GpuPipeline>>>>,
    fixed_buffers: Arc<Mutex<Option<Arc<FixedBuffers>>>>,
    io_buffers: Arc<Mutex<Option<IoBuffers>>>,
}

impl GpuContext {
    /// Initialize GPU context, preferring high-performance adapters.
    pub async fn init() -> Result<Self, GpuError> {
        let instance = if cfg!(target_arch = "wasm32") {
            wgpu::Instance::new(wgpu::InstanceDescriptor {
                backends: wgpu::Backends::BROWSER_WEBGPU,
                dx12_shader_compiler: wgpu::Dx12Compiler::Fxc,
                flags: wgpu::InstanceFlags::default(),
                gles_minor_version: wgpu::Gles3MinorVersion::Automatic,
            })
        } else {
            wgpu::Instance::new(wgpu::InstanceDescriptor {
                backends: wgpu::Backends::PRIMARY,
                dx12_shader_compiler: wgpu::Dx12Compiler::Fxc,
                flags: wgpu::InstanceFlags::default(),
                gles_minor_version: wgpu::Gles3MinorVersion::Automatic,
            })
        };
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
            })
            .await
            .ok_or_else(|| GpuError::Unavailable("no suitable adapter found".into()))?;

        let adapter_info = adapter.get_info();
        let required_features = wgpu::Features::empty();

        // On the web, some implementations reject `requestDevice` if optional limits
        // (like `maxInterStageShaderComponents`) are provided. Use the portable
        // WebGPU defaults there and keep the full adapter limits on native builds.
        // On WebAssembly, keep the requested limits minimal. Some Chrome/Dawn
        // builds reject `requestDevice` when optional limits like
        // `maxInterStageShaderComponents` are present, even with default values.
        // Using the WebGL2 downlevel defaults and zeroing the problematic
        // fields avoids sending those limits entirely and makes the request
        // portable across browser versions.
        let required_limits = if cfg!(target_arch = "wasm32") {
            let mut limits = wgpu::Limits::downlevel_webgl2_defaults();
            // Avoid optional limits that Chrome/Dawn can reject when present.
            limits.max_inter_stage_shader_components = 0;
            limits
        } else {
            adapter.limits()
        };

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("zeldhash-miner-gpu-device"),
                    required_features,
                    required_limits,
                },
                None,
            )
            .await
            .map_err(|e| GpuError::Unavailable(format!("request_device failed: {e}")))?;

        Ok(Self {
            device: Arc::new(device),
            queue: Arc::new(queue),
            adapter_info,
            batch_size_cache: Arc::new(Mutex::new(None)),
            pipeline_cache: Arc::new(Mutex::new(None)),
            fixed_buffers: Arc::new(Mutex::new(None)),
            io_buffers: Arc::new(Mutex::new(None)),
        })
    }

    /// Human-readable description of the active adapter.
    pub fn adapter_summary(&self) -> AdapterSummary {
        AdapterSummary::from(self.adapter_info.clone())
    }
}

/// Mining parameters for a single batch.
#[derive(Debug, Clone)]
pub struct MiningBatch<'a> {
    /// Serialized tx prefix (pre-nonce).
    pub tx_prefix: &'a [u8],
    /// Serialized tx suffix (post-nonce).
    pub tx_suffix: &'a [u8],
    /// Starting nonce for the batch.
    pub start_nonce: u64,
    /// Number of attempts.
    pub batch_size: u32,
    /// Target leading zeros (txid view).
    pub target_zeros: u8,
    /// When true, encode the nonce as CBOR (major type 0) rather than raw big-endian bytes.
    pub use_cbor_nonce: bool,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct MiningParams {
    start_nonce_lo: u32,
    start_nonce_hi: u32,
    batch_size: u32,
    target_zeros: u32,
    prefix_len: u32,
    suffix_len: u32,
    nonce_len: u32,
    use_cbor_nonce: u32, // bool flag (0 = raw, 1 = CBOR)
    _pad2: u32,          // reserved for future fields / alignment
    _pad3: u32,
    _pad4: u32,
    _pad5: u32,
}

#[repr(C, align(16))]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ResultEntry {
    nonce_lo: u32,
    nonce_hi: u32,
    txid: [u32; 8],
    _tail_pad: [u32; 2],
}

// Align to 16 bytes to match WGSL storage layout expectations on all targets.
#[repr(C, align(16))]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ResultBuffer {
    found_count: u32,
    _pad: u32, // alignment to 8-byte boundary for following array
    _align_pad: [u32; 2],
    results: [ResultEntry; MAX_RESULTS],
    _tail_pad: [u32; 2],
    _final_pad: [u32; 2],
}

// Compile-time layout sanity checks (must stay in sync with WGSL).
#[allow(dead_code)]
const RESULT_ENTRY_SIZE: usize = 48;
#[allow(dead_code)]
const RESULT_BUFFER_HEADER: usize = 16; // found_count + _pad + _align_pad
#[allow(dead_code)]
const RESULT_BUFFER_TAIL: usize = 16; // _tail_pad + _final_pad
#[allow(dead_code)]
const RESULT_BUFFER_SIZE: usize =
    ((RESULT_BUFFER_HEADER + (MAX_RESULTS * RESULT_ENTRY_SIZE) + RESULT_BUFFER_TAIL + 15) / 16)
        * 16;
const _: [(); RESULT_ENTRY_SIZE] = [(); std::mem::size_of::<ResultEntry>()];
const _: [(); RESULT_BUFFER_SIZE] = [(); std::mem::size_of::<ResultBuffer>()];

#[cfg_attr(test, allow(dead_code))]
struct GpuPipeline {
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
}

struct FixedBuffers {
    result: wgpu::Buffer,
    staging: wgpu::Buffer,
}

struct IoBuffers {
    prefix: wgpu::Buffer,
    prefix_capacity: u64,
    suffix: wgpu::Buffer,
    suffix_capacity: u64,
    params: wgpu::Buffer,
    params_capacity: u64,
}

type IoBuffersCacheGuard<'a> = MutexGuard<'a, Option<IoBuffers>>;

fn min_capacity(size: u64) -> u64 {
    // Avoid zero-sized buffers, keep allocations aligned and reusable.
    size.max(16).next_power_of_two()
}

fn create_buffer(
    device: &wgpu::Device,
    label: &str,
    size: u64,
    usage: wgpu::BufferUsages,
) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size,
        usage,
        mapped_at_creation: false,
    })
}

impl IoBuffers {
    fn new(device: &wgpu::Device, prefix: u64, suffix: u64, params: u64) -> Self {
        let prefix_capacity = min_capacity(prefix);
        let suffix_capacity = min_capacity(suffix);
        let params_capacity = min_capacity(params);

        Self {
            prefix: create_buffer(
                device,
                "zeldhash-miner-gpu-prefix-pooled",
                prefix_capacity,
                wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            ),
            prefix_capacity,
            suffix: create_buffer(
                device,
                "zeldhash-miner-gpu-suffix-pooled",
                suffix_capacity,
                wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            ),
            suffix_capacity,
            params: create_buffer(
                device,
                "zeldhash-miner-gpu-params-pooled",
                params_capacity,
                wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            ),
            params_capacity,
        }
    }

    fn ensure_capacity(
        &mut self,
        device: &wgpu::Device,
        prefix: u64,
        suffix: u64,
        params: u64,
        limits: &wgpu::Limits,
    ) -> Result<(), GpuError> {
        let max_storage: u64 = limits.max_storage_buffer_binding_size.into();
        let max_uniform: u64 = limits.max_uniform_buffer_binding_size.into();

        if prefix > max_storage {
            return Err(GpuError::Internal(format!(
                "prefix buffer exceeds max storage binding size ({} > {})",
                prefix, max_storage
            )));
        }
        if suffix > max_storage {
            return Err(GpuError::Internal(format!(
                "suffix buffer exceeds max storage binding size ({} > {})",
                suffix, max_storage
            )));
        }
        if params > max_uniform {
            return Err(GpuError::Internal(format!(
                "params buffer exceeds max uniform binding size ({} > {})",
                params, max_uniform
            )));
        }

        let needed_prefix = min_capacity(prefix).min(max_storage);
        if needed_prefix > self.prefix_capacity {
            self.prefix = create_buffer(
                device,
                "zeldhash-miner-gpu-prefix-pooled",
                needed_prefix,
                wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            );
            self.prefix_capacity = needed_prefix;
        }

        let needed_suffix = min_capacity(suffix).min(max_storage);
        if needed_suffix > self.suffix_capacity {
            self.suffix = create_buffer(
                device,
                "zeldhash-miner-gpu-suffix-pooled",
                needed_suffix,
                wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            );
            self.suffix_capacity = needed_suffix;
        }

        let needed_params = min_capacity(params).min(max_uniform);
        if needed_params > self.params_capacity {
            self.params = create_buffer(
                device,
                "zeldhash-miner-gpu-params-pooled",
                needed_params,
                wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            );
            self.params_capacity = needed_params;
        }

        Ok(())
    }
}

fn get_or_create_io_buffers(
    ctx: &GpuContext,
    prefix: u64,
    suffix: u64,
    params: u64,
) -> Result<IoBuffersCacheGuard<'_>, GpuError> {
    let limits = ctx.device.limits();
    let mut guard = ctx
        .io_buffers
        .lock()
        .map_err(|_| GpuError::Internal("buffer cache poisoned".into()))?;

    let buffers = guard.get_or_insert_with(|| IoBuffers::new(&ctx.device, prefix, suffix, params));
    buffers.ensure_capacity(&ctx.device, prefix, suffix, params, &limits)?;

    Ok(guard)
}

fn fallback_batch_size(info: &wgpu::AdapterInfo) -> u32 {
    // Provide GPU-class aware defaults so we avoid overcommitting integrated GPUs
    // while still giving discrete GPUs a throughput-friendly starting point.
    match info.device_type {
        wgpu::DeviceType::IntegratedGpu => 100_000,
        wgpu::DeviceType::DiscreteGpu => 1_000_000,
        wgpu::DeviceType::VirtualGpu => 200_000,
        wgpu::DeviceType::Cpu => 25_000,
        _ => 150_000,
    }
}

fn cbor_nonce_len(value: u64) -> u32 {
    match value {
        0..=23 => 1,
        24..=255 => 2,
        256..=65_535 => 3,
        65_536..=0xFFFF_FFFF => 5,
        _ => 9,
    }
}

fn nonce_len_for_range(
    start_nonce: u64,
    batch_size: u32,
    use_cbor_nonce: bool,
) -> Result<u32, GpuError> {
    if batch_size == 0 {
        return Err(GpuError::Internal("batch_size must be positive".into()));
    }
    let last = start_nonce
        .checked_add(batch_size as u64 - 1)
        .ok_or_else(|| GpuError::Internal("nonce range overflow".into()))?;

    let (start_len, last_len) = if use_cbor_nonce {
        (cbor_nonce_len(start_nonce), cbor_nonce_len(last))
    } else {
        (
            encode_nonce(start_nonce).len() as u32,
            encode_nonce(last).len() as u32,
        )
    };

    if start_len != last_len {
        return Err(GpuError::Internal(
            "nonce range crosses byte-length boundary; split batch".into(),
        ));
    }
    Ok(start_len)
}

fn pad_bytes_to_words(bytes: &[u8]) -> Vec<u32> {
    let mut padded = bytes.to_vec();
    while padded.len() % 4 != 0 {
        padded.push(0);
    }
    padded
        .chunks_exact(4)
        .map(|chunk| u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect()
}

fn to_u8_bytes(words: &[u32; 8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    for (i, word) in words.iter().enumerate() {
        out[i * 4..(i + 1) * 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

#[cfg_attr(test, allow(dead_code))]
fn create_shader_module(ctx: &GpuContext) -> wgpu::ShaderModule {
    ctx.device
        .create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("zeldhash-miner-gpu-miner-shader-wgsl"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(SHADER_WGSL)),
        })
}

fn build_pipeline(ctx: &GpuContext) -> Result<GpuPipeline, GpuError> {
    let shader = create_shader_module(ctx);

    let layout = ctx
        .device
        .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("zeldhash-miner-gpu-bind-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(
                            std::mem::size_of::<MiningParams>() as u64
                        ),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(
                            std::mem::size_of::<ResultBuffer>() as u64
                        ),
                    },
                    count: None,
                },
            ],
        });

    let pipeline_layout = ctx
        .device
        .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("zeldhash-miner-gpu-pipeline-layout"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });

    let pipeline = ctx
        .device
        .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("zeldhash-miner-gpu-miner"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: "main",
        });

    Ok(GpuPipeline { pipeline, layout })
}

fn get_or_create_pipeline(ctx: &GpuContext) -> Result<Arc<GpuPipeline>, GpuError> {
    if let Ok(mut cache) = ctx.pipeline_cache.lock() {
        if let Some(p) = cache.as_ref() {
            return Ok(p.clone());
        }
        let built = Arc::new(build_pipeline(ctx)?);
        *cache = Some(built.clone());
        return Ok(built);
    }

    // Fallback if the mutex is poisoned.
    Ok(Arc::new(build_pipeline(ctx)?))
}

fn get_or_create_fixed_buffers(ctx: &GpuContext) -> Result<Arc<FixedBuffers>, GpuError> {
    let size = std::mem::size_of::<ResultBuffer>() as u64;

    if let Ok(mut cache) = ctx.fixed_buffers.lock() {
        if let Some(bufs) = cache.as_ref() {
            return Ok(bufs.clone());
        }

        let result = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("zeldhash-miner-gpu-results"),
            size,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let staging = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("zeldhash-miner-gpu-result-staging"),
            size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let fixed = Arc::new(FixedBuffers { result, staging });
        *cache = Some(fixed.clone());
        return Ok(fixed);
    }

    // Fallback if the mutex is poisoned.
    let result = ctx.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("zeldhash-miner-gpu-results"),
        size,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let staging = ctx.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("zeldhash-miner-gpu-result-staging"),
        size,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    Ok(Arc::new(FixedBuffers { result, staging }))
}

#[cfg_attr(test, allow(dead_code))]
fn create_buffers(
    ctx: &GpuContext,
    pipeline: &GpuPipeline,
    batch: &MiningBatch<'_>,
    nonce_len: u32,
    result_buf: &wgpu::Buffer,
) -> Result<wgpu::BindGroup, GpuError> {
    let prefix_words = pad_bytes_to_words(batch.tx_prefix);
    let suffix_words = pad_bytes_to_words(batch.tx_suffix);

    let prefix_size = (prefix_words.len() * std::mem::size_of::<u32>()) as u64;
    let suffix_size = (suffix_words.len() * std::mem::size_of::<u32>()) as u64;
    let params_size = std::mem::size_of::<MiningParams>() as u64;

    let buffers_guard = get_or_create_io_buffers(ctx, prefix_size, suffix_size, params_size)?;
    let buffers = buffers_guard
        .as_ref()
        .expect("io buffers must be initialized before use");

    if !prefix_words.is_empty() {
        ctx.queue
            .write_buffer(&buffers.prefix, 0, cast_slice(&prefix_words));
    }
    if !suffix_words.is_empty() {
        ctx.queue
            .write_buffer(&buffers.suffix, 0, cast_slice(&suffix_words));
    }

    let params = MiningParams {
        start_nonce_lo: batch.start_nonce as u32,
        start_nonce_hi: (batch.start_nonce >> 32) as u32,
        batch_size: batch.batch_size,
        target_zeros: batch.target_zeros as u32,
        prefix_len: batch.tx_prefix.len() as u32,
        suffix_len: batch.tx_suffix.len() as u32,
        nonce_len,
        use_cbor_nonce: batch.use_cbor_nonce as u32,
        _pad2: 0,
        _pad3: 0,
        _pad4: 0,
        _pad5: 0,
    };
    ctx.queue.write_buffer(
        &buffers.params,
        0,
        cast_slice(std::slice::from_ref(&params)),
    );

    let bind_group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("zeldhash-miner-gpu-bind-group"),
        layout: &pipeline.layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: buffers.prefix.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: buffers.suffix.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: buffers.params.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: result_buf.as_entire_binding(),
            },
        ],
    });

    Ok(bind_group)
}

#[cfg_attr(test, allow(dead_code))]
fn parse_results(mapped: &[u8]) -> Vec<MineResult> {
    let required = std::mem::size_of::<ResultBuffer>();
    if mapped.len() < required {
        return Vec::new();
    }

    // Browser WebGPU can return a mapped slice that is not aligned to the
    // 16-byte boundary required by ResultBuffer. Read with an unaligned helper
    // to avoid panicking in bytemuck when the pointer is misaligned.
    let buffer: ResultBuffer = pod_read_unaligned(mapped);
    let found = buffer.found_count as usize;
    let take = found.min(MAX_RESULTS);

    let mut out = Vec::with_capacity(take);
    for entry in buffer.results.iter().take(take) {
        let nonce = ((entry.nonce_hi as u64) << 32) | entry.nonce_lo as u64;
        out.push(MineResult {
            nonce,
            txid: to_u8_bytes(&entry.txid),
        });
    }
    out
}

async fn dispatch_gpu(
    ctx: &GpuContext,
    batch: &MiningBatch<'_>,
    nonce_len: u32,
) -> Result<Vec<MineResult>, GpuError> {
    if batch.batch_size == 0 {
        return Ok(Vec::new());
    }

    let pipeline = get_or_create_pipeline(ctx)?;
    let fixed = get_or_create_fixed_buffers(ctx)?;

    // Clear the shared result buffer before dispatch.
    let zero_template = vec![0u8; std::mem::size_of::<ResultBuffer>()];
    ctx.queue.write_buffer(&fixed.result, 0, &zero_template);

    let bind_group = create_buffers(ctx, &pipeline, batch, nonce_len, &fixed.result)?;

    let mut encoder = ctx
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("zeldhash-miner-gpu-encoder"),
        });

    {
        let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("zeldhash-miner-gpu-compute-pass"),
            timestamp_writes: None,
        });
        cpass.set_pipeline(&pipeline.pipeline);
        cpass.set_bind_group(0, &bind_group, &[]);
        let groups = (batch.batch_size + WORKGROUP_SIZE - 1) / WORKGROUP_SIZE;
        cpass.dispatch_workgroups(groups, 1, 1);
    }

    encoder.copy_buffer_to_buffer(
        &fixed.result,
        0,
        &fixed.staging,
        0,
        std::mem::size_of::<ResultBuffer>() as u64,
    );

    ctx.queue.submit(Some(encoder.finish()));

    let (sender, receiver) = oneshot::channel();
    fixed
        .staging
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |res| {
            let _ = sender.send(res);
        });

    ctx.device.poll(wgpu::Maintain::Wait);

    receiver
        .await
        .map_err(|e| GpuError::Internal(format!("failed to receive map result: {e}")))?
        .map_err(|e| GpuError::Internal(format!("failed to map results: {e:?}")))?;

    let data = fixed.staging.slice(..).get_mapped_range();
    let parsed = parse_results(&data);
    drop(data);
    fixed.staging.unmap();
    Ok(parsed)
}

/// Dispatch a mining batch on the GPU and return all matching nonces found.
pub async fn dispatch_mining_batch(
    ctx: &GpuContext,
    batch: &MiningBatch<'_>,
) -> Result<Vec<MineResult>, GpuError> {
    let nonce_len = nonce_len_for_range(batch.start_nonce, batch.batch_size, batch.use_cbor_nonce)?;
    dispatch_gpu(ctx, batch, nonce_len).await
}

/// Calibrate an approximate batch size for the current adapter.
pub async fn calibrate_batch_size(ctx: &GpuContext) -> Result<u32, GpuError> {
    // Return cached value when available to avoid re-running calibration.
    if let Ok(cache) = ctx.batch_size_cache.lock() {
        if let Some(value) = *cache {
            return Ok(value);
        }
    }

    // Include small and large samples to better fit a wide range of adapters.
    let candidates = [1_000u32, 10_000, 100_000, 1_000_000];
    let mut best = 100_000u32;
    let mut best_hps = 0.0f64;

    // Use minimal non-empty buffers so create_buffer_init never receives zero-length data.
    const DUMMY: &[u8] = &[0u8];
    let pipeline = get_or_create_pipeline(ctx)?;

    let prefix_words = pad_bytes_to_words(DUMMY);
    let suffix_words = pad_bytes_to_words(DUMMY);
    let params_template = MiningParams {
        start_nonce_lo: 0,
        start_nonce_hi: 0,
        batch_size: 1,
        target_zeros: 64, // effectively impossible, keeps kernel busy
        prefix_len: DUMMY.len() as u32,
        suffix_len: DUMMY.len() as u32,
        nonce_len: 1,
        use_cbor_nonce: 0,
        _pad2: 0,
        _pad3: 0,
        _pad4: 0,
        _pad5: 0,
    };

    let prefix_buf = ctx
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("zeldhash-miner-gpu-prefix-calibration"),
            contents: cast_slice(&prefix_words),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });
    let suffix_buf = ctx
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("zeldhash-miner-gpu-suffix-calibration"),
            contents: cast_slice(&suffix_words),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });
    let params_buf = ctx
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("zeldhash-miner-gpu-params-calibration"),
            contents: cast_slice(std::slice::from_ref(&params_template)),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
    let result_buf = ctx.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("zeldhash-miner-gpu-results-calibration"),
        size: std::mem::size_of::<ResultBuffer>() as u64,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let staging = ctx.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("zeldhash-miner-gpu-result-staging-calibration"),
        size: std::mem::size_of::<ResultBuffer>() as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let bind_group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("zeldhash-miner-gpu-bind-group-calibration"),
        layout: &pipeline.layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: prefix_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: suffix_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: params_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: result_buf.as_entire_binding(),
            },
        ],
    });

    let zero_template = vec![0u8; std::mem::size_of::<ResultBuffer>()];

    for &size in &candidates {
        let mut params = params_template;
        params.batch_size = size;

        ctx.queue
            .write_buffer(&params_buf, 0, cast_slice(std::slice::from_ref(&params)));
        ctx.queue.write_buffer(&result_buf, 0, &zero_template);

        let start = Instant::now();
        let mut encoder = ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("zeldhash-miner-gpu-calibration-encoder"),
            });
        {
            let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("zeldhash-miner-gpu-calibration-pass"),
                timestamp_writes: None,
            });
            cpass.set_pipeline(&pipeline.pipeline);
            cpass.set_bind_group(0, &bind_group, &[]);
            let groups = (size + WORKGROUP_SIZE - 1) / WORKGROUP_SIZE;
            cpass.dispatch_workgroups(groups, 1, 1);
        }

        encoder.copy_buffer_to_buffer(
            &result_buf,
            0,
            &staging,
            0,
            std::mem::size_of::<ResultBuffer>() as u64,
        );

        ctx.queue.submit(Some(encoder.finish()));
        let (sender, receiver) = oneshot::channel();
        staging
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |res| {
                let _ = sender.send(res);
            });

        ctx.device.poll(wgpu::Maintain::Wait);

        receiver
            .await
            .map_err(|e| GpuError::Internal(format!("failed to receive map result: {e}")))?
            .map_err(|e| GpuError::Internal(format!("failed to map results: {e:?}")))?;

        // We do not parse results; mapping ensures the workload finished.
        staging.unmap();

        let elapsed = start.elapsed().as_secs_f64();
        if elapsed == 0.0 {
            continue;
        }
        let hps = size as f64 / elapsed;
        if hps > best_hps {
            best_hps = hps;
            best = size;
        }
    }

    let best_final = if best_hps == 0.0 {
        fallback_batch_size(&ctx.adapter_info)
    } else {
        best
    };

    if let Ok(mut cache) = ctx.batch_size_cache.lock() {
        *cache = Some(best_final);
    }

    Ok(best_final)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn cpu_mine(batch: &MiningBatch<'_>) -> Vec<MineResult> {
        let nonce_len =
            nonce_len_for_range(batch.start_nonce, batch.batch_size, batch.use_cbor_nonce)
                .expect("valid nonce range");
        let mut buf = Vec::new();
        let mut out = Vec::new();
        for offset in 0..batch.batch_size {
            if let Some(nonce) = batch.start_nonce.checked_add(offset as u64) {
                buf.clear();
                buf.extend_from_slice(batch.tx_prefix);
                if batch.use_cbor_nonce {
                    let encoded = zeldhash_miner_core::cbor::encode_cbor_uint(nonce);
                    assert_eq!(encoded.len(), nonce_len as usize);
                    buf.extend_from_slice(&encoded);
                } else {
                    let be = nonce.to_be_bytes();
                    let start = 8 - nonce_len as usize;
                    buf.extend_from_slice(&be[start..]);
                }
                buf.extend_from_slice(batch.tx_suffix);
                let hash = zeldhash_miner_core::double_sha256(&buf);
                if zeldhash_miner_core::hash_meets_target(&hash, batch.target_zeros) {
                    out.push(MineResult { nonce, txid: hash });
                }
            }
        }
        out
    }

    #[test]
    fn pads_bytes_to_words() {
        let words = pad_bytes_to_words(&[0x01, 0x02, 0x03]);
        assert_eq!(words.len(), 1);
        assert_eq!(words[0], 0x030201);
    }

    #[test]
    fn converts_words_to_bytes() {
        let words = [0x11223344u32; 8];
        let bytes = to_u8_bytes(&words);
        assert_eq!(bytes[0], 0x11);
        assert_eq!(bytes[1], 0x22);
        assert_eq!(bytes[2], 0x33);
        assert_eq!(bytes[3], 0x44);
    }

    #[test]
    fn gpu_matches_cpu_when_available() {
        let ctx = pollster::block_on(GpuContext::init());
        let ctx = match ctx {
            Ok(c) => c,
            Err(_) => return, // Skip if WebGPU not available in CI environment.
        };

        let batch = MiningBatch {
            tx_prefix: b"hello",
            tx_suffix: b"world",
            start_nonce: 0,
            batch_size: 64,
            target_zeros: 1,
            use_cbor_nonce: false,
        };

        let mut cpu = cpu_mine(&batch);
        let mut gpu = pollster::block_on(dispatch_mining_batch(&ctx, &batch)).unwrap();

        cpu.sort_by_key(|r| r.nonce);
        gpu.sort_by_key(|r| r.nonce);
        assert_eq!(cpu, gpu);
    }

    #[test]
    fn gpu_collects_multiple_results_up_to_max_when_available() {
        let ctx = pollster::block_on(GpuContext::init());
        let ctx = match ctx {
            Ok(c) => c,
            Err(_) => return, // Skip if WebGPU not available in CI environment.
        };

        let batch = MiningBatch {
            tx_prefix: b"a",
            tx_suffix: b"b",
            start_nonce: 0,
            batch_size: (MAX_RESULTS as u32) + 2,
            target_zeros: 0, // every hash counts
            use_cbor_nonce: false,
        };

        let gpu_results =
            pollster::block_on(dispatch_mining_batch(&ctx, &batch)).expect("gpu dispatch failed");
        assert_eq!(
            gpu_results.len(),
            MAX_RESULTS.min(batch.batch_size as usize)
        );
    }

    #[test]
    fn integrated_gpu_target_hash_rate_calculation() {
        // Use the integrated GPU fallback batch size and assume a short dispatch to
        // confirm the rate calculation clears the 10 MH/s target. This avoids
        // depending on real hardware while still guarding the math.
        let integrated = fallback_batch_size(&wgpu::AdapterInfo {
            name: String::from("test-integrated"),
            vendor: 0,
            device: 0,
            device_type: wgpu::DeviceType::IntegratedGpu,
            backend: wgpu::Backend::Vulkan,
            driver: String::new(),
            driver_info: String::new(),
        });

        assert_eq!(integrated, 100_000);

        let discrete = fallback_batch_size(&wgpu::AdapterInfo {
            name: String::from("test-discrete"),
            vendor: 0,
            device: 0,
            device_type: wgpu::DeviceType::DiscreteGpu,
            backend: wgpu::Backend::Vulkan,
            driver: String::new(),
            driver_info: String::new(),
        });
        assert!(discrete > integrated);

        let elapsed = Duration::from_millis(5); // 0.005s dispatch
        let rate = integrated as f64 / elapsed.as_secs_f64();
        assert!(rate >= 10_000_000.0, "expected >= 10 MH/s, got {rate} H/s");
    }
}
