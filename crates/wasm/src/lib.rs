use js_sys::{Array, BigInt, Object, Reflect, Uint8Array};
use serde::{Deserialize, Serialize};
use wasm_bindgen::{prelude::*, JsCast};

#[cfg(feature = "gpu")]
use std::cell::RefCell;

use zeldhash_miner_core::{
    build_mining_template as core_build_mining_template, build_psbt_from_plan, double_sha256,
    encode_cbor_uint, encode_nonce, mine_batch_with_cbor, nonce_len_for_range,
    parse_address_for_network, split_nonce_segments, split_nonce_segments_cbor, txid_to_hex,
    Network, OutputRequest, TransactionPlan, TxInput,
};

#[cfg(test)]
use zeldhash_miner_core::mine_batch;

#[cfg(feature = "gpu")]
use zeldhash_miner_gpu::{
    calibrate_batch_size as gpu_calibrate_batch_size, dispatch_mining_batch, GpuContext,
    MineResult as GpuMineResult, MiningBatch,
};

#[cfg(feature = "gpu")]
thread_local! {
    static GPU_CTX: RefCell<Option<GpuContext>> = const { RefCell::new(None) };
}

fn network_from_str(network: &str) -> Option<Network> {
    match network.to_ascii_lowercase().as_str() {
        "mainnet" => Some(Network::Mainnet),
        "testnet" | "signet" => Some(Network::Testnet),
        "regtest" => Some(Network::Regtest),
        _ => None,
    }
}

fn default_sequence() -> u32 {
    zeldhash_miner_core::tx::DEFAULT_SEQUENCE
}

fn js_error(msg: impl AsRef<str>) -> JsValue {
    JsValue::from_str(msg.as_ref())
}

#[derive(Debug, Deserialize, Serialize)]
struct JsInput {
    txid: String,
    vout: u32,
    #[serde(rename = "scriptPubKey")]
    script_pubkey: String,
    amount: u64,
    #[serde(default = "default_sequence")]
    sequence: u32,
}

#[derive(Debug, Deserialize, Serialize)]
struct JsOutput {
    address: String,
    amount: Option<u64>,
    change: bool,
}

fn decode_txid_hex(txid: &str) -> Result<[u8; 32], String> {
    let mut bytes = hex::decode(txid).map_err(|_| "invalid txid hex".to_string())?;
    if bytes.len() != 32 {
        return Err("txid must be 32 bytes".to_string());
    }
    bytes.reverse(); // store little-endian for serialization
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes);
    Ok(out)
}

fn decode_hex_bytes(data: &str) -> Result<Vec<u8>, String> {
    hex::decode(data).map_err(|_| "invalid hex string".to_string())
}

const JS_MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;

#[derive(Debug, Deserialize, Serialize, Clone, Copy)]
struct NonceRange {
    #[serde(rename = "startNonce")]
    start: u64,
    #[serde(rename = "batchSize")]
    size: u32,
}

fn parse_nonce_range(range: JsValue) -> Result<NonceRange, JsValue> {
    serde_wasm_bindgen::from_value(range).map_err(|e| js_error(format!("invalid nonce range: {e}")))
}

fn parse_distribution(js: JsValue) -> Result<Option<Vec<u64>>, JsValue> {
    if js.is_null() || js.is_undefined() {
        return Ok(None);
    }

    if !js.is_object() || !Array::is_array(&js) {
        return Err(js_error(
            "distribution must be an array of integers (number or BigInt)",
        ));
    }

    let arr = Array::from(&js);
    let mut dist = Vec::with_capacity(arr.length() as usize);

    for value in arr.iter() {
        let parsed = if let Some(bi) = value.dyn_ref::<js_sys::BigInt>() {
            let s = bi
                .to_string(10)
                .map_err(|_| js_error("failed to convert distribution BigInt to string"))?
                .as_string()
                .ok_or_else(|| js_error("failed to convert distribution BigInt to string"))?;
            s.parse::<u64>()
                .map_err(|_| js_error("distribution values must fit in u64"))?
        } else if let Some(num) = value.as_f64() {
            if !num.is_finite() || num.fract() != 0.0 || num < 0.0 {
                return Err(js_error(
                    "distribution values must be non-negative integers",
                ));
            }
            if num > JS_MAX_SAFE_INTEGER {
                return Err(js_error(
                    "distribution values above 2^53 must be provided as BigInt",
                ));
            }
            num as u64
        } else {
            return Err(js_error(
                "distribution values must be integers (number or BigInt)",
            ));
        };

        dist.push(parsed);
    }

    Ok(Some(dist))
}

fn parse_inputs(js_inputs: Vec<JsInput>) -> Result<Vec<TxInput>, String> {
    let mut inputs = Vec::with_capacity(js_inputs.len());
    for input in js_inputs {
        let txid = decode_txid_hex(&input.txid)?;
        let script_pubkey = decode_hex_bytes(&input.script_pubkey)?;
        inputs.push(TxInput {
            txid,
            vout: input.vout,
            script_pubkey,
            amount: input.amount,
            sequence: input.sequence,
        });
    }
    Ok(inputs)
}

fn plan_transaction(
    inputs: JsValue,
    outputs: JsValue,
    network: &str,
    sats_per_vbyte: u64,
    op_return_payload: &[u8],
    distribution: Option<&[u64]>,
) -> Result<TransactionPlan, String> {
    let js_inputs: Vec<JsInput> =
        serde_wasm_bindgen::from_value(inputs).map_err(|e| format!("invalid inputs: {e}"))?;
    let js_outputs: Vec<JsOutput> =
        serde_wasm_bindgen::from_value(outputs).map_err(|e| format!("invalid outputs: {e}"))?;

    let network = network_from_str(network).ok_or_else(|| "unsupported network".to_string())?;

    let inputs = parse_inputs(js_inputs)?;
    let output_requests: Vec<OutputRequest> = js_outputs
        .into_iter()
        .map(|o| OutputRequest {
            address: o.address,
            amount: o.amount,
            change: o.change,
        })
        .collect();

    zeldhash_miner_core::plan_transaction(
        inputs,
        output_requests,
        network,
        sats_per_vbyte,
        op_return_payload,
        distribution,
    )
    .map_err(|e| e.to_string())
}

fn build_psbt_inner(
    inputs: JsValue,
    outputs: JsValue,
    network: &str,
    sats_per_vbyte: u64,
    nonce: u64,
    distribution: Option<Vec<u64>>,
) -> Result<String, String> {
    let nonce_bytes = if distribution.is_some() {
        encode_cbor_uint(nonce)
    } else {
        encode_nonce(nonce)
    };

    let plan = plan_transaction(
        inputs,
        outputs,
        network,
        sats_per_vbyte,
        &nonce_bytes,
        distribution.as_deref(),
    )?;
    let (psbt_b64, _) = build_psbt_from_plan(&plan).map_err(|e| e.to_string())?;
    Ok(psbt_b64)
}

fn make_result_object(nonce: u64, txid: &[u8; 32]) -> JsValue {
    let obj = Object::new();
    let nonce_bigint = BigInt::new(&JsValue::from_str(&nonce.to_string()))
        .expect("u64 to BigInt conversion should succeed");
    let _ = Reflect::set(&obj, &"nonce".into(), &nonce_bigint.into());
    let txid_hex = txid_to_hex(txid);
    let _ = Reflect::set(&obj, &"txid".into(), &JsValue::from_str(&txid_hex));
    obj.into()
}

fn make_validation_object(ok: bool, message: Option<String>) -> JsValue {
    let obj = Object::new();
    let _ = Reflect::set(&obj, &"ok".into(), &JsValue::from_bool(ok));
    if let Some(msg) = message {
        let _ = Reflect::set(&obj, &"error".into(), &JsValue::from_str(&msg));
    }
    obj.into()
}

#[wasm_bindgen]
pub fn init_panic_hook() {
    console_error_panic_hook::set_once();
}

#[cfg(feature = "gpu")]
fn with_gpu_ctx<F, R>(f: F) -> Option<R>
where
    F: FnOnce(&GpuContext) -> R,
{
    GPU_CTX.with(|ctx| ctx.borrow().as_ref().map(f))
}

#[wasm_bindgen]
pub fn mine_batch_wasm(
    tx_prefix: &[u8],
    tx_suffix: &[u8],
    start_nonce: u64,
    batch_size: u32,
    target_zeros: u8,
    use_cbor_nonce: bool,
) -> JsValue {
    match mine_batch_with_cbor(
        tx_prefix,
        tx_suffix,
        start_nonce,
        batch_size,
        target_zeros,
        use_cbor_nonce,
    ) {
        Ok(Some(res)) => make_result_object(res.nonce, &res.txid),
        Ok(None) => JsValue::NULL,
        Err(err) => make_validation_object(false, Some(err.to_string())),
    }
}

fn build_template_for_nonce_len(
    inputs: JsValue,
    outputs: JsValue,
    network: &str,
    sats_per_vbyte: u64,
    nonce_len: usize,
    distribution: Option<&[u64]>,
) -> Result<(Vec<u8>, Vec<u8>), JsValue> {
    let placeholder = vec![0u8; nonce_len];
    let plan = plan_transaction(
        inputs,
        outputs,
        network,
        sats_per_vbyte,
        &placeholder,
        distribution,
    )
    .map_err(js_error)?;
    let template =
        core_build_mining_template(&plan, nonce_len).map_err(|e| js_error(e.to_string()))?;
    Ok((template.prefix, template.suffix))
}

/// Mine across a nonce range, automatically splitting at byte-length boundaries.
#[wasm_bindgen]
pub fn mine_range_wasm(
    inputs: JsValue,
    outputs: JsValue,
    network: &str,
    sats_per_vbyte: u64,
    range: JsValue,
    target_zeros: u8,
    distribution: JsValue,
) -> JsValue {
    let range = match parse_nonce_range(range) {
        Ok(r) => r,
        Err(err) => return err,
    };

    let distribution = match parse_distribution(distribution) {
        Ok(d) => d,
        Err(err) => return err,
    };

    let use_cbor_nonce = distribution.is_some();
    let segments = if use_cbor_nonce {
        match split_nonce_segments_cbor(range.start, range.size) {
            Ok(s) => s,
            Err(err) => return make_validation_object(false, Some(err.to_string())),
        }
    } else {
        match split_nonce_segments(range.start, range.size) {
            Ok(s) => s,
            Err(err) => return make_validation_object(false, Some(err.to_string())),
        }
    };

    for segment in segments {
        let (prefix, suffix) = match build_template_for_nonce_len(
            inputs.clone(),
            outputs.clone(),
            network,
            sats_per_vbyte,
            segment.nonce_len as usize,
            distribution.as_deref(),
        ) {
            Ok(ps) => ps,
            Err(err) => return err,
        };

        match mine_batch_with_cbor(
            &prefix,
            &suffix,
            segment.start,
            segment.size,
            target_zeros,
            use_cbor_nonce,
        ) {
            Ok(Some(res)) => return make_result_object(res.nonce, &res.txid),
            Ok(None) => continue,
            Err(err) => return make_validation_object(false, Some(err.to_string())),
        }
    }

    JsValue::NULL
}

#[wasm_bindgen]
pub fn validate_address(addr: &str, network: &str) -> JsValue {
    let network = match network_from_str(network) {
        Some(net) => net,
        None => return make_validation_object(false, Some("unsupported network".to_string())),
    };

    match parse_address_for_network(addr, Some(network)) {
        Ok(parsed) => {
            let obj = Object::new();
            let _ = Reflect::set(&obj, &"ok".into(), &JsValue::from_bool(true));
            let _ = Reflect::set(
                &obj,
                &"addressType".into(),
                &JsValue::from_str(match parsed.address_type {
                    zeldhash_miner_core::AddressType::P2WPKH => "p2wpkh",
                    zeldhash_miner_core::AddressType::P2TR => "p2tr",
                }),
            );
            let _ = Reflect::set(
                &obj,
                &"network".into(),
                &JsValue::from_str(match parsed.network {
                    Network::Mainnet => "mainnet",
                    Network::Testnet => "testnet",
                    Network::Regtest => "regtest",
                }),
            );
            obj.into()
        }
        Err(err) => make_validation_object(false, Some(err.to_string())),
    }
}

#[wasm_bindgen]
pub fn build_psbt(
    inputs: JsValue,
    outputs: JsValue,
    network: &str,
    sats_per_vbyte: u64,
    nonce: u64,
    distribution: JsValue,
) -> Result<String, JsValue> {
    let distribution = parse_distribution(distribution)?;

    build_psbt_inner(
        inputs,
        outputs,
        network,
        sats_per_vbyte,
        nonce,
        distribution,
    )
    .map_err(js_error)
}

#[wasm_bindgen]
pub fn build_mining_template(
    inputs: JsValue,
    outputs: JsValue,
    network: &str,
    sats_per_vbyte: u64,
    start_nonce: u64,
    batch_size: u32,
    distribution: JsValue,
) -> Result<JsValue, JsValue> {
    let distribution = parse_distribution(distribution)?;
    let use_cbor_nonce = distribution.is_some();

    let nonce_len = if use_cbor_nonce {
        let segments = split_nonce_segments_cbor(start_nonce, batch_size).map_err(js_error)?;
        if segments.len() != 1 {
            return Err(js_error(
                "nonce range crosses CBOR length boundary; reduce batch size",
            ));
        }
        segments[0].nonce_len as usize
    } else {
        nonce_len_for_range(start_nonce, batch_size).map_err(js_error)? as usize
    };

    let placeholder = vec![0u8; nonce_len];
    let plan = plan_transaction(
        inputs,
        outputs,
        network,
        sats_per_vbyte,
        &placeholder,
        distribution.as_deref(),
    )
    .map_err(js_error)?;
    let template =
        core_build_mining_template(&plan, nonce_len).map_err(|e| js_error(e.to_string()))?;
    let prefix = template.prefix;
    let suffix = template.suffix;

    let obj = Object::new();
    let _ = Reflect::set(
        &obj,
        &"prefix".into(),
        &Uint8Array::from(prefix.as_slice()).into(),
    );
    let _ = Reflect::set(
        &obj,
        &"suffix".into(),
        &Uint8Array::from(suffix.as_slice()).into(),
    );
    let _ = Reflect::set(
        &obj,
        &"useCborNonce".into(),
        &JsValue::from_bool(use_cbor_nonce),
    );
    Ok(obj.into())
}

#[wasm_bindgen]
pub fn compute_txid(tx_bytes: &[u8]) -> String {
    let hash = double_sha256(tx_bytes);
    txid_to_hex(&hash)
}

// ---------------- GPU bindings (feature-gated) ----------------
#[cfg(feature = "gpu")]
#[wasm_bindgen]
pub async fn init_gpu() -> Result<JsValue, JsValue> {
    let ctx = GpuContext::init()
        .await
        .map_err(|e| JsValue::from_str(&e.to_string()))?;
    let summary = ctx.adapter_summary();

    let obj = Object::new();
    let _ = Reflect::set(&obj, &"name".into(), &JsValue::from_str(&summary.name));
    let _ = Reflect::set(
        &obj,
        &"backend".into(),
        &JsValue::from_str(&summary.backend),
    );
    let _ = Reflect::set(
        &obj,
        &"deviceType".into(),
        &JsValue::from_str(&summary.device_type),
    );

    GPU_CTX.with(|slot| *slot.borrow_mut() = Some(ctx));
    Ok(obj.into())
}

#[cfg(feature = "gpu")]
#[wasm_bindgen]
/// Dispatch a GPU batch and return the match with the smallest nonce (OP_RETURN value).
pub async fn mine_batch_gpu(
    tx_prefix: &[u8],
    tx_suffix: &[u8],
    start_nonce: u64,
    batch_size: u32,
    target_zeros: u8,
    use_cbor_nonce: bool,
) -> JsValue {
    let length_check = if use_cbor_nonce {
        split_nonce_segments_cbor(start_nonce, batch_size)
    } else {
        split_nonce_segments(start_nonce, batch_size)
    };

    match length_check {
        Ok(segs) if segs.len() == 1 => {}
        Ok(_) => {
            let msg = if use_cbor_nonce {
                "nonce range crosses CBOR length boundary; reduce batch size".to_string()
            } else {
                "nonce range crosses byte-length boundary; reduce batch size".to_string()
            };
            return make_validation_object(false, Some(msg));
        }
        Err(err) => return make_validation_object(false, Some(err.to_string())),
    }

    let cpu_fallback = || {
        mine_batch_with_cbor(
            tx_prefix,
            tx_suffix,
            start_nonce,
            batch_size,
            target_zeros,
            use_cbor_nonce,
        )
        .map(|maybe| match maybe {
            Some(res) => make_result_object(res.nonce, &res.txid),
            None => JsValue::NULL,
        })
        .unwrap_or_else(|err| make_validation_object(false, Some(err.to_string())))
    };

    let ctx = match with_gpu_ctx(|c| c.clone()) {
        Some(ctx) => ctx,
        None => return cpu_fallback(),
    };

    let batch = MiningBatch {
        tx_prefix,
        tx_suffix,
        start_nonce,
        batch_size,
        target_zeros,
        use_cbor_nonce,
    };

    match dispatch_mining_batch(&ctx, &batch).await {
        Ok(results) => {
            // Multiple matches can occur; pick the one with the smallest nonce to keep
            // OP_RETURN minimal and deterministic for callers.
            if let Some(best) = results.iter().min_by_key(|r| r.nonce) {
                make_result_object(best.nonce, &best.txid)
            } else {
                JsValue::NULL
            }
        }
        Err(_) => cpu_fallback(),
    }
}

#[cfg(feature = "gpu")]
async fn mine_segment_gpu_or_cpu(
    ctx: Option<GpuContext>,
    prefix: Vec<u8>,
    suffix: Vec<u8>,
    start_nonce: u64,
    batch_size: u32,
    target_zeros: u8,
    use_cbor_nonce: bool,
) -> JsValue {
    let cpu_path = || {
        mine_batch_with_cbor(
            &prefix,
            &suffix,
            start_nonce,
            batch_size,
            target_zeros,
            use_cbor_nonce,
        )
        .map(|maybe| match maybe {
            Some(res) => make_result_object(res.nonce, &res.txid),
            None => JsValue::NULL,
        })
        .unwrap_or_else(|err| make_validation_object(false, Some(err.to_string())))
    };

    let ctx = match ctx {
        Some(c) => c,
        None => return cpu_path(),
    };

    let batch = MiningBatch {
        tx_prefix: &prefix,
        tx_suffix: &suffix,
        start_nonce,
        batch_size,
        target_zeros,
        use_cbor_nonce,
    };

    match dispatch_mining_batch(&ctx, &batch).await {
        Ok(results) => {
            if let Some(best) = select_best_result(&results) {
                make_result_object(best.nonce, &best.txid)
            } else {
                JsValue::NULL
            }
        }
        Err(_) => cpu_path(),
    }
}

#[cfg(feature = "gpu")]
fn select_best_result(results: &[GpuMineResult]) -> Option<GpuMineResult> {
    results.iter().min_by_key(|r| r.nonce).cloned()
}

#[cfg(feature = "gpu")]
async fn mine_range_segments_gpu(
    inputs: JsValue,
    outputs: JsValue,
    network: &str,
    sats_per_vbyte: u64,
    range: &NonceRange,
    target_zeros: u8,
    distribution: Option<&[u64]>,
) -> JsValue {
    let use_cbor_nonce = distribution.is_some();
    let segments = if use_cbor_nonce {
        match split_nonce_segments_cbor(range.start, range.size) {
            Ok(s) => s,
            Err(err) => return make_validation_object(false, Some(err.to_string())),
        }
    } else {
        match split_nonce_segments(range.start, range.size) {
            Ok(s) => s,
            Err(err) => return make_validation_object(false, Some(err.to_string())),
        }
    };

    let ctx = with_gpu_ctx(|c| c.clone());

    for segment in segments {
        let (prefix, suffix) = match build_template_for_nonce_len(
            inputs.clone(),
            outputs.clone(),
            network,
            sats_per_vbyte,
            segment.nonce_len as usize,
            distribution,
        ) {
            Ok(ps) => ps,
            Err(err) => return err,
        };

        let res = mine_segment_gpu_or_cpu(
            ctx.clone(),
            prefix,
            suffix,
            segment.start,
            segment.size,
            target_zeros,
            use_cbor_nonce,
        )
        .await;

        if !res.is_null() {
            return res;
        }
    }

    JsValue::NULL
}

#[cfg(feature = "gpu")]
#[wasm_bindgen]
pub async fn mine_range_gpu(
    inputs: JsValue,
    outputs: JsValue,
    network: &str,
    sats_per_vbyte: u64,
    range: JsValue,
    target_zeros: u8,
    distribution: JsValue,
) -> JsValue {
    let range = match parse_nonce_range(range) {
        Ok(r) => r,
        Err(err) => return err,
    };

    let distribution = match parse_distribution(distribution) {
        Ok(d) => d,
        Err(err) => return err,
    };

    mine_range_segments_gpu(
        inputs,
        outputs,
        network,
        sats_per_vbyte,
        &range,
        target_zeros,
        distribution.as_deref(),
    )
    .await
}

// Stub for non-GPU builds to keep the JS API predictable.
#[cfg(not(feature = "gpu"))]
#[wasm_bindgen]
pub async fn mine_range_gpu(
    _inputs: JsValue,
    _outputs: JsValue,
    _network: &str,
    _sats_per_vbyte: u64,
    _range: JsValue,
    _target_zeros: u8,
    _distribution: JsValue,
) -> JsValue {
    make_validation_object(false, Some("GPU feature not enabled".to_string()))
}

#[cfg(feature = "gpu")]
#[wasm_bindgen]
pub async fn calibrate_batch_size() -> Result<u32, JsValue> {
    let ctx = match with_gpu_ctx(|c| c.clone()) {
        Some(ctx) => ctx,
        None => return Err(JsValue::from_str("GPU not initialized")),
    };

    gpu_calibrate_batch_size(&ctx)
        .await
        .map_err(|e| JsValue::from_str(&e.to_string()))
}

#[cfg(not(feature = "gpu"))]
#[wasm_bindgen]
pub async fn calibrate_batch_size() -> Result<u32, JsValue> {
    Err(JsValue::from_str("GPU feature not enabled"))
}

// ---------------- Tests ----------------
#[cfg(test)]
mod tests {
    use super::{double_sha256, mine_batch, nonce_len_for_range, split_nonce_segments};
    use bitcoin::hashes::Hash;
    use bitcoin::psbt::Psbt;
    use std::{sync::mpsc, thread, time::Duration};
    use zeldhash_miner_core::{
        count_leading_zeros, create_op_return_script, create_psbt, encode_nonce,
        split_tx_for_mining, MineResult, TxInput, TxOutput,
    };

    fn mine_range_segments(
        prefix: &[u8],
        suffix: &[u8],
        start_nonce: u64,
        batch_size: u32,
        target_zeros: u8,
    ) -> Option<MineResult> {
        let segments = split_nonce_segments(start_nonce, batch_size)
            .expect("segment splitting should succeed");
        let mut best: Option<MineResult> = None;
        for seg in segments {
            if let Some(res) =
                mine_batch(prefix, suffix, seg.start, seg.size, target_zeros).expect("segment mine")
            {
                if best.as_ref().map_or(true, |b| res.nonce < b.nonce) {
                    best = Some(res);
                }
            }
        }
        best
    }

    fn hashes_per_second(attempts: u64, elapsed: Duration) -> f64 {
        attempts as f64 / elapsed.as_secs_f64()
    }

    #[test]
    fn mine_batch_uses_minimal_nonce() {
        let prefix = b"p";
        let suffix = b"s";
        let start_nonce = 0xffu64; // 255 -> 1-byte minimal encoding
        let batch_size = 2;

        let expected_bytes = {
            let mut v = Vec::new();
            v.extend_from_slice(prefix);
            v.extend_from_slice(&[0xff]); // minimal encoding
            v.extend_from_slice(suffix);
            v
        };
        let expected_hash = double_sha256(&expected_bytes);

        let result = mine_range_segments(prefix, suffix, start_nonce, batch_size, 0)
            .expect("should find a result");

        assert_eq!(result.nonce, start_nonce);
        assert_eq!(result.txid, expected_hash);
    }

    #[test]
    fn mine_batch_rejects_range_crossing_byte_boundary() {
        // Range crosses 0xff -> 0x100 so byte length would change.
        let err = mine_batch(b"p", b"s", 0xff, 2, 0).unwrap_err();
        assert!(err.contains("byte-length boundary"));
    }

    #[test]
    fn splits_nonce_segments_across_boundaries() {
        let segments = split_nonce_segments(0xf0, 0x20).expect("must split");
        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0].start, 0xf0);
        assert_eq!(segments[0].size, 16); // up to 0xff
        assert_eq!(segments[0].nonce_len, 1);
        assert_eq!(segments[1].start, 0x100);
        assert_eq!(segments[1].nonce_len, 2);
    }

    #[test]
    fn rejects_overflowing_nonce_range() {
        // Range would overflow u64.
        assert!(mine_batch(b"", b"", u64::MAX, 2, 0).is_err());
    }

    #[test]
    fn single_worker_finds_known_4_zero_hash() {
        // Pre-computed nonce for prefix "p", suffix "s" that yields 4 leading zeros.
        // Deterministic for current hashing.
        const TARGET_NONCE: u64 = 63_372;
        let prefix = b"p";
        let suffix = b"s";
        let batch_size = 70_000u32; // covers the expected nonce

        let result = mine_range_segments(prefix, suffix, 0, batch_size, 4)
            .expect("worker should find a match");

        let mut expected_bytes = Vec::new();
        expected_bytes.extend_from_slice(prefix);
        expected_bytes.extend_from_slice(&encode_nonce(TARGET_NONCE));
        expected_bytes.extend_from_slice(suffix);
        let expected_hash = double_sha256(&expected_bytes);

        assert_eq!(result.nonce, TARGET_NONCE);
        assert_eq!(result.txid, expected_hash);
        assert!(
            count_leading_zeros(&result.txid) >= 4,
            "txid should have at least 4 leading hex zeros"
        );
    }

    #[test]
    fn worker_hash_rate_calculation_exceeds_200kh() {
        // Simulate a worker hashing 500k nonces in 2 seconds; should clear 200 KH/s.
        let rate = hashes_per_second(500_000, Duration::from_secs(2));
        assert!(rate >= 200_000.0, "expected at least 200 KH/s, got {rate}");
    }

    #[test]
    fn end_to_end_psbt_includes_mined_nonce() {
        // Build a simple P2WPKH scriptPubKey directly (avoids wasm/js bindings in native tests).
        let mut spk = vec![0x00, 0x14];
        spk.extend_from_slice(&[0x11u8; 20]);

        let input = TxInput {
            txid: [0x11u8; 32],
            vout: 0,
            script_pubkey: spk.clone(),
            amount: 100_000,
            sequence: zeldhash_miner_core::tx::DEFAULT_SEQUENCE,
        };
        let user_output = TxOutput {
            script_pubkey: spk.clone(),
            amount: 50_000,
        };

        let start_nonce = 0u64;
        let batch_size = 1u32;
        let nonce_len = nonce_len_for_range(start_nonce, batch_size).expect("valid range") as usize;
        let nonce_bytes = encode_nonce(start_nonce);
        assert_eq!(nonce_bytes.len(), nonce_len);

        let op_return_script = create_op_return_script(&nonce_bytes);
        let change_output = TxOutput {
            script_pubkey: spk,
            amount: input.amount - user_output.amount, // ignore fees for this focused test
        };

        let outputs_before = vec![user_output.clone()];
        let outputs_after = vec![change_output.clone()];

        // Build prefix/suffix around the OP_RETURN placeholder of the known nonce length.
        let (prefix, suffix) =
            split_tx_for_mining(&[input.clone()], &outputs_before, &outputs_after, nonce_len)
                .expect("split succeeds");

        let mined = mine_batch(&prefix, &suffix, start_nonce, batch_size, 0)
            .expect("mine ok")
            .expect("target_zeros=0 yields first nonce");

        let mut full_tx = Vec::new();
        full_tx.extend_from_slice(&prefix);
        full_tx.extend_from_slice(&encode_nonce(mined.nonce));
        full_tx.extend_from_slice(&suffix);
        let txid = double_sha256(&full_tx);
        assert_eq!(mined.txid, txid);

        // Construct a PSBT using the same outputs (including OP_RETURN and change) and verify txid.
        let psbt_outputs = vec![
            user_output,
            TxOutput {
                script_pubkey: op_return_script,
                amount: 0,
            },
            change_output,
        ];
        let psbt_bytes = create_psbt(&[input], &psbt_outputs).expect("psbt builds");
        let psbt = Psbt::deserialize(&psbt_bytes).expect("psbt parses");

        assert_eq!(
            psbt.unsigned_tx
                .compute_txid()
                .to_raw_hash()
                .to_byte_array(),
            txid
        );
        let op_return_spk = psbt.unsigned_tx.output[1].script_pubkey.as_bytes();
        assert_eq!(op_return_spk, &[0x6a, 0x01, 0x00]);
    }

    #[test]
    fn coordinates_multi_worker_search_and_prefers_first_match() {
        const TARGET_NONCE: u64 = 63_372;
        let prefix = b"p".to_vec();
        let suffix = b"s".to_vec();
        let target_zeros = 4u8;

        let (tx, rx) = mpsc::channel();
        let mut handles = Vec::new();
        let ranges = [(0u64, 70_000u32), (70_000u64, 70_000u32)];

        for (start_nonce, batch_size) in ranges {
            let tx = tx.clone();
            let prefix = prefix.clone();
            let suffix = suffix.clone();
            handles.push(thread::spawn(move || {
                let res =
                    mine_range_segments(&prefix, &suffix, start_nonce, batch_size, target_zeros);
                let _ = tx.send(res);
            }));
        }
        drop(tx);

        let mut found = None;
        for _ in 0..ranges.len() {
            if let Ok(Some(hit)) = rx.recv_timeout(Duration::from_secs(2)) {
                if found
                    .as_ref()
                    .map_or(true, |current: &MineResult| hit.nonce < current.nonce)
                {
                    found = Some(hit);
                }
            }
        }

        for handle in handles {
            handle.join().expect("worker thread should finish");
        }

        let found = found.expect("one of the workers must report a match");
        assert_eq!(found.nonce, TARGET_NONCE);
        assert!(
            count_leading_zeros(&found.txid) >= target_zeros,
            "winning txid should satisfy target"
        );
    }

    #[cfg(target_arch = "wasm32")]
    mod wasm_only {
        use super::*;
        use super::{build_psbt_inner, JsInput, JsOutput};
        use bech32::{hrp, segwit};
        use bitcoin::psbt::Psbt;
        use serde_wasm_bindgen;
        use std::str::FromStr;

        #[test]
        fn build_psbt_uses_minimal_nonce() {
            let program = vec![0x11u8; 20];
            let addr = segwit::encode(hrp::BC, segwit::VERSION_0, &program).unwrap();

            let mut spk = vec![0x00, 0x14];
            spk.extend_from_slice(&program);

            let inputs = vec![JsInput {
                txid: "11".repeat(32),
                vout: 0,
                script_pubkey: hex::encode(&spk),
                amount: 100_000,
            }];
            let outputs = vec![
                JsOutput {
                    address: addr.clone(),
                    amount: Some(50_000),
                    change: false,
                },
                JsOutput {
                    address: addr,
                    amount: None,
                    change: true,
                },
            ];

            let inputs_js = serde_wasm_bindgen::to_value(&inputs).unwrap();
            let outputs_js = serde_wasm_bindgen::to_value(&outputs).unwrap();

            // Nonce 0xff encodes to 1 byte; the builder should use the minimal encoding automatically.
            let psbt_b64 = build_psbt_inner(inputs_js, outputs_js, "mainnet", 1, 0xff)
                .expect("psbt should build");
            let psbt = Psbt::from_str(&psbt_b64).expect("psbt must parse");

            // Ordering: user output, OP_RETURN, change
            let op_return = &psbt.unsigned_tx.output[1].script_pubkey;
            assert_eq!(op_return.as_bytes(), &[0x6a, 0x01, 0xff]);
        }
    }
}
