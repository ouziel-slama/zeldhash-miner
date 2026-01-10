use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Condvar, Mutex,
    },
    time::Instant,
};

#[cfg(not(feature = "rayon"))]
use std::thread;

#[cfg(feature = "rayon")]
use rayon::prelude::*;
use thiserror::Error;
use zeldhash_miner_core::{
    build_mining_template, build_psbt_from_plan, encode_cbor_uint, encode_nonce,
    split_nonce_segments, split_nonce_segments_cbor, txid_to_hex, AddressError, FeeError,
    MinerError, Network, NonceSegment, OutputRequest, TxInput,
};
use zeldhash_miner_core::{double_sha256, hash_meets_target};

#[cfg(feature = "gpu")]
use zeldhash_miner_gpu::{
    dispatch_mining_batch, GpuContext, MineResult as GpuMineResult, MiningBatch,
};

// Accept 0 to mirror JS/WASM bindings semantics: 0 means "accept first hash".
const MAX_TARGET_ZEROS: u8 = 32;
#[cfg(feature = "gpu")]
const GPU_MAX_BATCH_SIZE: u32 = 100_000;

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, Copy)]
pub enum NetworkOption {
    Mainnet,
    Testnet,
    Signet,
    Regtest,
}

impl NetworkOption {
    fn to_core(self) -> Network {
        match self {
            NetworkOption::Mainnet => Network::Mainnet,
            NetworkOption::Testnet | NetworkOption::Signet => Network::Testnet,
            NetworkOption::Regtest => Network::Regtest,
        }
    }
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone)]
pub struct ZeldMinerOptions {
    pub network: NetworkOption,
    pub batch_size: u32,
    pub use_gpu: bool,
    pub worker_threads: usize,
    pub sats_per_vbyte: u64,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone)]
pub struct TxInputDesc {
    pub txid: String,
    pub vout: u32,
    pub script_pubkey: String,
    pub amount: u64,
    pub sequence: Option<u32>,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone)]
pub struct TxOutputDesc {
    pub address: String,
    pub amount: Option<u64>,
    pub change: bool,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone)]
pub struct MineParams {
    pub inputs: Vec<TxInputDesc>,
    pub outputs: Vec<TxOutputDesc>,
    pub target_zeros: u8,
    pub start_nonce: Option<u64>,
    pub batch_size: Option<u32>,
    pub distribution: Option<Vec<u64>>,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone)]
pub struct MineResult {
    pub psbt: String,
    pub txid: String,
    pub nonce: u64,
    pub attempts: u128,
    pub duration_ms: u128,
    pub hash_rate: f64,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone)]
pub struct ProgressStats {
    pub hashes_processed: u128,
    pub hash_rate: f64,
    pub elapsed_ms: u128,
    pub last_nonce: Option<u64>,
}

#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(rename_all = "snake_case")
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZeldMinerErrorCode {
    InvalidAddress,
    UnsupportedAddressType,
    InsufficientFunds,
    NoChangeOutput,
    MultipleChangeOutputs,
    InvalidInput,
    WorkerError,
    MiningAborted,
    NoMatchingNonce,
    DustOutput,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Error)]
#[error("{code:?}: {message}")]
pub struct ZeldMinerError {
    pub code: ZeldMinerErrorCode,
    pub message: String,
}

type Result<T> = std::result::Result<T, ZeldMinerError>;

impl ZeldMinerError {
    fn new(code: ZeldMinerErrorCode, message: impl Into<String>) -> Self {
        ZeldMinerError {
            code,
            message: message.into(),
        }
    }
}

/// Coordinates one active mining run per instance; create separate instances if you
/// need concurrent searches to avoid cross-talk on pause/stop signals.
pub struct ZeldMiner {
    opts: ZeldMinerOptions,
    control: MiningControl,
    #[cfg(feature = "gpu")]
    gpu_ctx: Option<GpuContext>,
}

#[derive(Default)]
struct MiningControl {
    stop: AtomicBool,
    pause: AtomicBool,
    mutex: Mutex<()>,
    cv: Condvar,
}

impl MiningControl {
    fn new() -> Self {
        Self {
            stop: AtomicBool::new(false),
            pause: AtomicBool::new(false),
            mutex: Mutex::new(()),
            cv: Condvar::new(),
        }
    }

    fn reset(&self) {
        self.stop.store(false, Ordering::Release);
        self.pause.store(false, Ordering::Release);
        self.cv.notify_all();
    }

    fn request_stop(&self) {
        self.stop.store(true, Ordering::Release);
        self.cv.notify_all();
    }

    fn request_pause(&self) {
        self.pause.store(true, Ordering::Release);
    }

    fn resume(&self) {
        self.pause.store(false, Ordering::Release);
        self.cv.notify_all();
    }

    fn wait_if_paused_or_stopped(&self) -> Result<()> {
        if self.stop.load(Ordering::Acquire) {
            return Err(mining_aborted_error("mining stopped"));
        }
        if !self.pause.load(Ordering::Acquire) {
            return Ok(());
        }

        let mut guard = self
            .mutex
            .lock()
            .expect("pause mutex should not be poisoned");
        while self.pause.load(Ordering::Acquire) && !self.stop.load(Ordering::Acquire) {
            guard = self.cv.wait(guard).expect("pause condvar wait failed");
        }

        if self.stop.load(Ordering::Acquire) {
            Err(mining_aborted_error("mining stopped"))
        } else {
            Ok(())
        }
    }
}

impl ZeldMiner {
    pub fn new(opts: ZeldMinerOptions) -> Result<Self> {
        if opts.batch_size == 0 {
            return Err(ZeldMinerError::new(
                ZeldMinerErrorCode::InvalidInput,
                "batch_size must be greater than zero",
            ));
        }
        if opts.worker_threads == 0 {
            return Err(ZeldMinerError::new(
                ZeldMinerErrorCode::InvalidInput,
                "worker_threads must be greater than zero",
            ));
        }
        if opts.sats_per_vbyte == 0 {
            return Err(ZeldMinerError::new(
                ZeldMinerErrorCode::InvalidInput,
                "sats_per_vbyte must be greater than zero",
            ));
        }
        #[cfg(feature = "gpu")]
        let gpu_ctx = if opts.use_gpu {
            match pollster::block_on(GpuContext::init()) {
                Ok(ctx) => Some(ctx),
                Err(err) => {
                    // Fall back to CPU if GPU init fails; keep error context for visibility.
                    eprintln!("GPU initialization failed, falling back to CPU: {err}");
                    None
                }
            }
        } else {
            None
        };

        Ok(Self {
            opts,
            control: MiningControl::new(),
            #[cfg(feature = "gpu")]
            gpu_ctx,
        })
    }

    pub fn mine_transaction<F, G>(
        &self,
        params: MineParams,
        mut on_progress: Option<F>,
        mut on_found: Option<G>,
    ) -> Result<MineResult>
    where
        F: FnMut(ProgressStats),
        G: FnMut(&MineResult),
    {
        if params.target_zeros > MAX_TARGET_ZEROS {
            return Err(ZeldMinerError::new(
                ZeldMinerErrorCode::InvalidInput,
                "target_zeros must be between 0 and 32",
            ));
        }

        let batch_size = params.batch_size.unwrap_or(self.opts.batch_size);
        if batch_size == 0 {
            return Err(ZeldMinerError::new(
                ZeldMinerErrorCode::InvalidInput,
                "batch_size must be greater than zero",
            ));
        }

        let start_nonce = params.start_nonce.unwrap_or(0);
        let network = self.opts.network.to_core();
        let parsed_inputs = parse_inputs(&params.inputs)?;
        let output_requests: Vec<OutputRequest> = params
            .outputs
            .iter()
            .cloned()
            .map(OutputRequest::from)
            .collect();
        let distribution = params.distribution.as_deref();

        let segments = if distribution.is_some() {
            split_nonce_segments_cbor(start_nonce, batch_size)
        } else {
            split_nonce_segments(start_nonce, batch_size)
        }
        .map_err(|e| ZeldMinerError::new(ZeldMinerErrorCode::InvalidInput, e))?;
        let started_at = Instant::now();
        let mut attempts: u128 = 0;

        self.control.reset();

        let workers = self.opts.worker_threads.max(1);

        for segment in segments {
            self.control.wait_if_paused_or_stopped()?;

            let placeholder = vec![0u8; segment.nonce_len as usize];
            let plan = zeldhash_miner_core::plan_transaction(
                parsed_inputs.clone(),
                output_requests.clone(),
                network,
                self.opts.sats_per_vbyte,
                &placeholder,
                distribution,
            )
            .map_err(map_miner_error)?;

            let template = build_mining_template(&plan, segment.nonce_len as usize)
                .map_err(map_miner_error)?;

            let mut attempts_this_segment: u128 = 0;
            let mut mined: Option<(u64, [u8; 32])> = None;

            if self.opts.use_gpu {
                #[cfg(feature = "gpu")]
                {
                    if let Some(ctx) = self.gpu_ctx.as_ref() {
                        if let Ok(res) = mine_segment_gpu_controlled(
                            ctx,
                            &template.prefix,
                            &template.suffix,
                            &segment,
                            params.target_zeros,
                            distribution.is_some(),
                            &self.control,
                        ) {
                            attempts_this_segment = res.attempts;
                            mined = res.hit;
                            if mined.is_none() {
                                // GPU completed successfully with no hit; do not rerun on CPU.
                                attempts += attempts_this_segment;
                                if let Some(cb) = on_progress.as_mut() {
                                    let elapsed_ms = started_at.elapsed().as_millis();
                                    cb(ProgressStats {
                                        hashes_processed: attempts,
                                        hash_rate: hash_rate(attempts, elapsed_ms),
                                        elapsed_ms,
                                        last_nonce: Some(segment.start + segment.size as u64 - 1),
                                    });
                                }
                                // Honor a stop/pause request before moving to the next segment.
                                self.control.wait_if_paused_or_stopped()?;
                                continue;
                            }
                        } else {
                            // GPU path failed; fall back to CPU for this segment.
                        }
                    }
                }
                // If GPU work concluded and a stop was requested, abort before continuing.
                self.control.wait_if_paused_or_stopped()?;
            }

            if mined.is_none() {
                let controlled_result = if workers > 1 {
                    mine_segment_cpu_parallel(
                        &template.prefix,
                        &template.suffix,
                        &segment,
                        params.target_zeros,
                        distribution.is_some(),
                        &self.control,
                        workers,
                    )?
                } else {
                    mine_segment_cpu_controlled(
                        &template.prefix,
                        &template.suffix,
                        &segment,
                        params.target_zeros,
                        distribution.is_some(),
                        &self.control,
                        None,
                    )?
                };

                attempts_this_segment = controlled_result.attempts;
                mined = controlled_result.hit;

                if mined.is_none() {
                    self.control.wait_if_paused_or_stopped()?;
                    attempts += attempts_this_segment;
                    if let Some(cb) = on_progress.as_mut() {
                        let elapsed_ms = started_at.elapsed().as_millis();
                        cb(ProgressStats {
                            hashes_processed: attempts,
                            hash_rate: hash_rate(attempts, elapsed_ms),
                            elapsed_ms,
                            last_nonce: Some(segment.start + segment.size as u64 - 1),
                        });
                    }
                    continue;
                }
            }

            if let Some((nonce, _txid_bytes)) = mined {
                attempts += attempts_this_segment;
                let nonce_bytes = if distribution.is_some() {
                    encode_cbor_uint(nonce)
                } else {
                    encode_nonce(nonce)
                };
                let plan_with_nonce = zeldhash_miner_core::plan_transaction(
                    parsed_inputs,
                    output_requests,
                    network,
                    self.opts.sats_per_vbyte,
                    &nonce_bytes,
                    distribution,
                )
                .map_err(map_miner_error)?;
                let (psbt, txid_bytes) =
                    build_psbt_from_plan(&plan_with_nonce).map_err(map_miner_error)?;

                let duration_ms = started_at.elapsed().as_millis();
                let hash_rate = hash_rate(attempts, duration_ms);

                let result = MineResult {
                    psbt,
                    txid: txid_to_hex(&txid_bytes),
                    nonce,
                    attempts,
                    duration_ms,
                    hash_rate,
                };

                if let Some(cb) = on_progress.as_mut() {
                    cb(ProgressStats {
                        hashes_processed: attempts,
                        hash_rate,
                        elapsed_ms: duration_ms,
                        last_nonce: Some(nonce),
                    });
                }

                if let Some(cb) = on_found.as_mut() {
                    cb(&result);
                }

                return Ok(result);
            }
        }

        Err(ZeldMinerError::new(
            ZeldMinerErrorCode::NoMatchingNonce,
            "no matching nonce found in provided range",
        ))
    }

    pub fn stop(&self) {
        self.control.request_stop();
    }

    pub fn pause(&self) {
        self.control.request_pause();
    }

    pub fn resume(&self) {
        self.control.resume();
    }
}

fn hash_rate(attempts: u128, duration_ms: u128) -> f64 {
    if duration_ms == 0 {
        return 0.0;
    }
    attempts as f64 / (duration_ms as f64 / 1000.0)
}

fn decode_txid_hex(txid: &str) -> Result<[u8; 32]> {
    let mut bytes = hex::decode(txid)
        .map_err(|_| ZeldMinerError::new(ZeldMinerErrorCode::InvalidInput, "invalid txid hex"))?;
    if bytes.len() != 32 {
        return Err(ZeldMinerError::new(
            ZeldMinerErrorCode::InvalidInput,
            "txid must be 32 bytes",
        ));
    }
    bytes.reverse();
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes);
    Ok(out)
}

fn decode_hex_bytes(data: &str) -> Result<Vec<u8>> {
    hex::decode(data).map_err(|_| {
        ZeldMinerError::new(
            ZeldMinerErrorCode::InvalidInput,
            "script_pubkey must be valid hex",
        )
    })
}

fn parse_inputs(inputs: &[TxInputDesc]) -> Result<Vec<TxInput>> {
    inputs
        .iter()
        .map(|input| {
            let txid = decode_txid_hex(&input.txid)?;
            let script_pubkey = decode_hex_bytes(&input.script_pubkey)?;
            Ok(TxInput {
                txid,
                vout: input.vout,
                script_pubkey,
                amount: input.amount,
                sequence: input
                    .sequence
                    .unwrap_or(zeldhash_miner_core::tx::DEFAULT_SEQUENCE),
            })
        })
        .collect()
}

fn map_miner_error(err: MinerError) -> ZeldMinerError {
    match err {
        MinerError::InvalidInput(msg) => ZeldMinerError::new(ZeldMinerErrorCode::InvalidInput, msg),
        MinerError::MultipleChangeOutputs => ZeldMinerError::new(
            ZeldMinerErrorCode::MultipleChangeOutputs,
            "multiple change outputs are not allowed",
        ),
        MinerError::Core(core) => match core {
            zeldhash_miner_core::ZeldError::Address(addr_err) => match addr_err {
                AddressError::UnsupportedAddressType => ZeldMinerError::new(
                    ZeldMinerErrorCode::UnsupportedAddressType,
                    addr_err.to_string(),
                ),
                _ => ZeldMinerError::new(ZeldMinerErrorCode::InvalidAddress, addr_err.to_string()),
            },
            zeldhash_miner_core::ZeldError::Fee(fee_err) => match fee_err {
                FeeError::InsufficientFunds => {
                    ZeldMinerError::new(ZeldMinerErrorCode::InsufficientFunds, fee_err.to_string())
                }
            },
            zeldhash_miner_core::ZeldError::Tx(_) | zeldhash_miner_core::ZeldError::Psbt(_) => {
                ZeldMinerError::new(ZeldMinerErrorCode::WorkerError, core.to_string())
            }
        },
    }
}

fn mining_aborted_error(message: &str) -> ZeldMinerError {
    ZeldMinerError::new(ZeldMinerErrorCode::MiningAborted, message)
}

struct ControlledMineResult {
    attempts: u128,
    hit: Option<(u64, [u8; 32])>,
}

fn mine_segment_cpu_controlled(
    prefix: &[u8],
    suffix: &[u8],
    segment: &NonceSegment,
    target_zeros: u8,
    use_cbor_nonce: bool,
    control: &MiningControl,
    found_flag: Option<&AtomicBool>,
) -> Result<ControlledMineResult> {
    let nonce_len = segment.nonce_len as usize;
    let mut buffer = Vec::with_capacity(prefix.len() + suffix.len() + nonce_len);
    let mut nonce_buf = [0u8; 9];

    for offset in 0..segment.size {
        control.wait_if_paused_or_stopped()?;

        if let Some(flag) = found_flag {
            if flag.load(Ordering::Acquire) {
                return Ok(ControlledMineResult {
                    attempts: offset as u128,
                    hit: None,
                });
            }
        }

        let nonce = match segment.start.checked_add(offset as u64) {
            Some(n) => n,
            None => {
                return Err(ZeldMinerError::new(
                    ZeldMinerErrorCode::InvalidInput,
                    "nonce range overflow",
                ))
            }
        };

        let written =
            encode_nonce_for_segment(nonce, segment.nonce_len, use_cbor_nonce, &mut nonce_buf)?;

        buffer.clear();
        buffer.extend_from_slice(prefix);
        buffer.extend_from_slice(&nonce_buf[..written]);
        buffer.extend_from_slice(suffix);

        let hash = double_sha256(&buffer);
        if hash_meets_target(&hash, target_zeros) {
            return Ok(ControlledMineResult {
                attempts: offset as u128 + 1,
                hit: Some((nonce, hash)),
            });
        }
    }

    Ok(ControlledMineResult {
        attempts: segment.size as u128,
        hit: None,
    })
}

fn split_segment_for_workers(segment: &NonceSegment, workers: usize) -> Result<Vec<NonceSegment>> {
    let workers = workers.max(1).min(segment.size as usize);
    let base = segment.size / workers as u32;
    let remainder = segment.size % workers as u32;

    let mut subs = Vec::with_capacity(workers);
    let mut start = segment.start;

    for idx in 0..workers {
        let extra = if (idx as u32) < remainder { 1 } else { 0 };
        let size = base + extra;
        if size == 0 {
            continue;
        }
        subs.push(NonceSegment {
            start,
            size,
            nonce_len: segment.nonce_len,
        });
        start = start.checked_add(size as u64).ok_or_else(|| {
            ZeldMinerError::new(ZeldMinerErrorCode::InvalidInput, "nonce range overflow")
        })?;
    }

    Ok(subs)
}

fn mine_segment_cpu_parallel(
    prefix: &[u8],
    suffix: &[u8],
    segment: &NonceSegment,
    target_zeros: u8,
    use_cbor_nonce: bool,
    control: &MiningControl,
    workers: usize,
) -> Result<ControlledMineResult> {
    if workers <= 1 || segment.size <= 1 {
        return mine_segment_cpu_controlled(
            prefix,
            suffix,
            segment,
            target_zeros,
            use_cbor_nonce,
            control,
            None,
        );
    }

    let subs = split_segment_for_workers(segment, workers)?;
    if subs.len() == 1 {
        return mine_segment_cpu_controlled(
            prefix,
            suffix,
            segment,
            target_zeros,
            use_cbor_nonce,
            control,
            None,
        );
    }

    #[cfg(feature = "rayon")]
    {
        let found = Arc::new(AtomicBool::new(false));
        let attempts_acc = Arc::new(Mutex::new(0u128));
        let result = Arc::new(Mutex::new(None));

        subs.into_par_iter().try_for_each(|sub| {
            let res = mine_segment_cpu_controlled(
                prefix,
                suffix,
                &sub,
                target_zeros,
                use_cbor_nonce,
                control,
                Some(found.as_ref()),
            );

            match res {
                Ok(r) => {
                    if r.hit.is_some() && !found.swap(true, Ordering::AcqRel) {
                        *result.lock().expect("result mutex poisoned") = r.hit;
                    }
                    *attempts_acc.lock().expect("attempts mutex poisoned") += r.attempts;
                    Ok(())
                }
                Err(err) => Err(err),
            }
        })?;

        let attempts = *attempts_acc.lock().expect("attempts mutex poisoned");
        let hit = result.lock().expect("result mutex poisoned").take();

        Ok(ControlledMineResult { attempts, hit })
    }

    #[cfg(not(feature = "rayon"))]
    {
        let found = Arc::new(AtomicBool::new(false));
        let attempts_acc = Arc::new(Mutex::new(0u128));
        let result = Arc::new(Mutex::new(None));
        let first_err = Arc::new(Mutex::new(None));

        thread::scope(|scope| {
            for sub in subs {
                let found = Arc::clone(&found);
                let attempts_acc = Arc::clone(&attempts_acc);
                let result = Arc::clone(&result);
                let first_err = Arc::clone(&first_err);
                scope.spawn(move || {
                    let res = mine_segment_cpu_controlled(
                        prefix,
                        suffix,
                        &sub,
                        target_zeros,
                        use_cbor_nonce,
                        control,
                        Some(found.as_ref()),
                    );

                    match res {
                        Ok(r) => {
                            if r.hit.is_some() && !found.swap(true, Ordering::AcqRel) {
                                *result.lock().expect("result mutex poisoned") = r.hit;
                            }
                            *attempts_acc.lock().expect("attempts mutex poisoned") += r.attempts;
                        }
                        Err(err) => {
                            *first_err.lock().expect("error mutex poisoned") = Some(err);
                        }
                    }
                });
            }
        });

        if let Some(err) = first_err.lock().expect("error mutex poisoned").take() {
            return Err(err);
        }

        let attempts = *attempts_acc.lock().expect("attempts mutex poisoned");
        let hit = result.lock().expect("result mutex poisoned").take();

        Ok(ControlledMineResult { attempts, hit })
    }
}

#[cfg(feature = "gpu")]
fn mine_segment_gpu_controlled(
    ctx: &GpuContext,
    prefix: &[u8],
    suffix: &[u8],
    segment: &NonceSegment,
    target_zeros: u8,
    use_cbor_nonce: bool,
    control: &MiningControl,
) -> Result<ControlledMineResult> {
    let mut attempts: u128 = 0;
    let mut remaining = segment.size;
    let mut current_start = segment.start;

    while remaining > 0 {
        control.wait_if_paused_or_stopped()?;

        let chunk = remaining.min(GPU_MAX_BATCH_SIZE);
        let batch = MiningBatch {
            tx_prefix: prefix,
            tx_suffix: suffix,
            start_nonce: current_start,
            batch_size: chunk,
            target_zeros,
            use_cbor_nonce,
        };

        let results = pollster::block_on(dispatch_mining_batch(ctx, &batch))
            .map_err(|err| ZeldMinerError::new(ZeldMinerErrorCode::WorkerError, err.to_string()))?;

        if let Some(best) = select_best_gpu_result(&results) {
            let attempts_to_hit = best
                .nonce
                .checked_sub(current_start)
                .and_then(|offset| offset.checked_add(1))
                .ok_or_else(|| {
                    ZeldMinerError::new(ZeldMinerErrorCode::InvalidInput, "nonce range overflow")
                })?;

            attempts += attempts_to_hit as u128;
            return Ok(ControlledMineResult {
                attempts,
                hit: Some((best.nonce, best.txid)),
            });
        }

        attempts += chunk as u128;
        current_start = current_start.checked_add(chunk as u64).ok_or_else(|| {
            ZeldMinerError::new(ZeldMinerErrorCode::InvalidInput, "nonce range overflow")
        })?;
        remaining -= chunk;
    }

    Ok(ControlledMineResult {
        attempts,
        hit: None,
    })
}

#[cfg(feature = "gpu")]
fn select_best_gpu_result(results: &[GpuMineResult]) -> Option<GpuMineResult> {
    results.iter().min_by_key(|r| r.nonce).cloned()
}

fn encode_nonce_for_segment(
    nonce: u64,
    nonce_len: u8,
    use_cbor_nonce: bool,
    out: &mut [u8; 9],
) -> Result<usize> {
    if use_cbor_nonce {
        let encoded = encode_cbor_uint(nonce);
        if encoded.len() != nonce_len as usize {
            return Err(ZeldMinerError::new(
                ZeldMinerErrorCode::InvalidInput,
                "CBOR nonce length mismatch for segment",
            ));
        }
        out[..encoded.len()].copy_from_slice(&encoded);
        return Ok(encoded.len());
    }

    encode_nonce_fixed_into(nonce, nonce_len, out)
}

fn encode_nonce_fixed_into(nonce: u64, nonce_len: u8, out: &mut [u8; 9]) -> Result<usize> {
    if nonce_len == 0 || nonce_len > 9 {
        return Err(ZeldMinerError::new(
            ZeldMinerErrorCode::InvalidInput,
            "nonce_len must be between 1 and 9 bytes",
        ));
    }

    let mut tmp = nonce;
    let mut minimal_len = 0usize;
    let mut buf = [0u8; 9];

    if tmp == 0 {
        minimal_len = 1;
        buf[8] = 0;
    } else {
        while tmp != 0 {
            minimal_len += 1;
            buf[9 - minimal_len] = (tmp & 0xff) as u8;
            tmp >>= 8;
        }
    }

    let target_len = nonce_len as usize;
    if minimal_len != target_len {
        return Err(ZeldMinerError::new(
            ZeldMinerErrorCode::InvalidInput,
            "nonce length does not match minimal encoding",
        ));
    }

    let src_start = 9 - minimal_len;
    out[..target_len].copy_from_slice(&buf[src_start..]);
    Ok(target_len)
}

impl From<TxOutputDesc> for OutputRequest {
    fn from(value: TxOutputDesc) -> Self {
        OutputRequest {
            address: value.address,
            amount: value.amount,
            change: value.change,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::bech32::{segwit, Hrp};
    use bitcoin::psbt::Psbt;
    use std::str::FromStr;
    use zeldhash_miner_core::create_zeld_distribution_op_return;

    fn sample_addresses() -> (String, String) {
        let hrp = Hrp::parse("bc").expect("mainnet hrp");
        let user_program = [0x33u8; 20];
        let change_program = [0x22u8; 20];
        let user = segwit::encode(hrp, segwit::VERSION_0, &user_program).unwrap();
        let change = segwit::encode(hrp, segwit::VERSION_0, &change_program).unwrap();
        (user, change)
    }

    fn sample_input_desc(amount: u64) -> TxInputDesc {
        let mut spk = vec![0x00, 0x14];
        spk.extend_from_slice(&[0x22u8; 20]);

        TxInputDesc {
            txid: "11".repeat(32),
            vout: 0,
            script_pubkey: hex::encode(spk),
            amount,
            sequence: Some(zeldhash_miner_core::tx::DEFAULT_SEQUENCE),
        }
    }

    fn sample_outputs(user_addr: &str, change_addr: &str) -> Vec<TxOutputDesc> {
        vec![
            TxOutputDesc {
                address: user_addr.to_string(),
                amount: Some(60_000),
                change: false,
            },
            TxOutputDesc {
                address: change_addr.to_string(),
                amount: None,
                change: true,
            },
        ]
    }

    fn miner_opts(use_gpu: bool) -> ZeldMinerOptions {
        ZeldMinerOptions {
            network: NetworkOption::Mainnet,
            batch_size: 4,
            use_gpu,
            worker_threads: 1,
            sats_per_vbyte: 2,
        }
    }

    fn mine_once(use_gpu: bool) -> MineResult {
        let miner = ZeldMiner::new(miner_opts(use_gpu)).expect("miner builds");
        let (user_addr, change_addr) = sample_addresses();
        let params = MineParams {
            inputs: vec![sample_input_desc(120_000)],
            outputs: sample_outputs(&user_addr, &change_addr),
            target_zeros: 0,
            start_nonce: Some(0),
            batch_size: Some(4),
            distribution: None,
        };

        let mut progress_called = false;
        let mut found_called = false;

        let result = miner
            .mine_transaction(
                params,
                Some(|stats: ProgressStats| {
                    progress_called = true;
                    assert!(stats.hashes_processed >= 1);
                }),
                Some(|_: &MineResult| found_called = true),
            )
            .expect("mining succeeds");

        assert!(progress_called, "progress callback should fire");
        assert!(found_called, "found callback should fire");
        result
    }

    #[test]
    fn cpu_path_mines_and_builds_psbt() {
        let result = mine_once(false);
        assert_eq!(result.nonce, 0);
        assert!(result.attempts >= 1);

        let psbt = Psbt::from_str(&result.psbt).expect("psbt parses");
        let txid = psbt.unsigned_tx.compute_txid().to_string();
        assert_eq!(txid, result.txid);
    }

    #[test]
    fn mines_with_custom_distribution() {
        let miner = ZeldMiner::new(miner_opts(false)).expect("miner builds");
        let (user_addr, change_addr) = sample_addresses();
        let hrp = Hrp::parse("bc").expect("mainnet hrp");
        let alt_addr = segwit::encode(hrp, segwit::VERSION_0, &[0x44u8; 20]).unwrap();
        // Distribution now must match total outputs (including change).
        let distribution = vec![600u64, 400, 0];

        let params = MineParams {
            inputs: vec![sample_input_desc(150_000)],
            outputs: vec![
                TxOutputDesc {
                    address: user_addr,
                    amount: Some(60_000),
                    change: false,
                },
                TxOutputDesc {
                    address: alt_addr,
                    amount: Some(30_000),
                    change: false,
                },
                TxOutputDesc {
                    address: change_addr,
                    amount: None,
                    change: true,
                },
            ],
            target_zeros: 0,
            start_nonce: Some(0),
            batch_size: Some(4),
            distribution: Some(distribution.clone()),
        };

        let mut progress_called = false;
        let mut found_called = false;

        let result = miner
            .mine_transaction(
                params,
                Some(|stats: ProgressStats| {
                    progress_called = true;
                    assert!(stats.hashes_processed >= 1);
                    assert_eq!(stats.last_nonce, Some(0));
                }),
                Some(|_: &MineResult| found_called = true),
            )
            .expect("mining succeeds with distribution");

        assert!(progress_called, "progress callback should fire");
        assert!(found_called, "found callback should fire");
        assert_eq!(result.nonce, 0);
        assert!(result.attempts >= 1);

        let psbt = Psbt::from_str(&result.psbt).expect("psbt parses");
        let expected_op_return = create_zeld_distribution_op_return(&distribution, result.nonce);

        assert!(
            psbt.unsigned_tx
                .output
                .iter()
                .any(|o| o.script_pubkey.as_bytes() == expected_op_return),
            "psbt must include ZELD distribution OP_RETURN"
        );
    }

    #[cfg(not(feature = "gpu"))]
    #[test]
    fn gpu_flag_falls_back_without_feature() {
        let result = mine_once(true);
        assert_eq!(result.nonce, 0);
    }

    #[cfg(feature = "gpu")]
    #[test]
    fn gpu_feature_path_runs_or_falls_back() {
        let result = mine_once(true);
        assert_eq!(result.nonce, 0);

        let psbt = Psbt::from_str(&result.psbt).expect("psbt parses");
        let txid = psbt.unsigned_tx.compute_txid().to_string();
        assert_eq!(txid, result.txid);
    }

    #[test]
    fn mines_successfully_when_change_is_dust() {
        // Craft amounts so that change ends up below dust limit
        // P2WPKH dust limit is 310 sats
        // For a tx with 1 input, 2 outputs (user + change) + OP_RETURN:
        // vsize ~ 129 vbytes at 2 sats/vB = 258 sats fee
        // With input=10000, user=9500, fee~258, change would be ~242 (below dust limit of 310)
        let miner = ZeldMiner::new(ZeldMinerOptions {
            network: NetworkOption::Mainnet,
            batch_size: 4,
            use_gpu: false,
            worker_threads: 1,
            sats_per_vbyte: 2,
        })
        .expect("miner builds");

        let (user_addr, change_addr) = sample_addresses();

        let params = MineParams {
            inputs: vec![sample_input_desc(10_000)],
            outputs: vec![
                TxOutputDesc {
                    address: user_addr,
                    amount: Some(9_500), // Leave ~500 sats for fee + dust change
                    change: false,
                },
                TxOutputDesc {
                    address: change_addr,
                    amount: None,
                    change: true,
                },
            ],
            target_zeros: 0,
            start_nonce: Some(0),
            batch_size: Some(4),
            distribution: None,
        };

        let result = miner
            .mine_transaction(params, None::<fn(ProgressStats)>, None::<fn(&MineResult)>)
            .expect("mining should succeed even when change is dust");

        // PSBT should parse and only have 2 outputs: user + OP_RETURN (no change)
        let psbt = Psbt::from_str(&result.psbt).expect("psbt parses");
        assert_eq!(
            psbt.unsigned_tx.output.len(),
            2,
            "should have user output + OP_RETURN only (no change)"
        );

        // Verify the txid matches
        let txid = psbt.unsigned_tx.compute_txid().to_string();
        assert_eq!(txid, result.txid);
    }
}
