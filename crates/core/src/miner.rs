use alloc::{format, string::String, vec, vec::Vec};

use thiserror::Error;

use crate::{
    address::{address_to_script_pubkey, parse_address_for_network, AddressType, Network},
    cbor::{cbor_nonce_byte_length, encode_cbor_array_header, encode_cbor_uint},
    error::ZeldError,
    fees::{calculate_change, calculate_fee, calculate_vsize, FeeError},
    hash::{double_sha256, hash_meets_target},
    nonce::nonce_byte_length,
    psbt::{create_psbt, psbt_to_base64, PsbtError},
    tx::{
        create_op_return_script, split_tx_for_distribution_mining, split_tx_for_mining,
        zeld_distribution_payload_length_with_nonce, TxError, TxInput, TxOutput, ZELD_PREFIX,
    },
};

fn dust_limit_for_address(address_type: AddressType) -> u64 {
    match address_type {
        // Bech32 P2WPKH (bc1q... / tb1q...)
        AddressType::P2WPKH => 310,
        // Taproot P2TR (bc1p... / tb1p...)
        AddressType::P2TR => 330,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputRequest {
    pub address: String,
    pub amount: Option<u64>,
    pub change: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransactionPlan {
    pub inputs: Vec<TxInput>,
    pub user_outputs: Vec<TxOutput>,
    pub change_output: TxOutput,
    pub op_return_script: Vec<u8>,
    pub op_return_size: usize,
    pub distribution: Option<Vec<u64>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MiningTemplate {
    pub prefix: Vec<u8>,
    pub suffix: Vec<u8>,
    pub nonce_len: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NonceSegment {
    pub start: u64,
    pub size: u32,
    pub nonce_len: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MineResult {
    pub nonce: u64,
    pub txid: [u8; 32],
}

#[derive(Debug, Error)]
pub enum MinerError {
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("missing change output")]
    MissingChangeOutput,
    #[error("multiple change outputs are not allowed")]
    MultipleChangeOutputs,
    #[error(transparent)]
    Core(#[from] ZeldError),
}

pub type MinerResult<T> = core::result::Result<T, MinerError>;

impl MinerError {
    fn invalid<S: Into<String>>(msg: S) -> Self {
        MinerError::InvalidInput(msg.into())
    }
}

impl From<TxError> for MinerError {
    fn from(err: TxError) -> Self {
        MinerError::Core(err.into())
    }
}

impl From<PsbtError> for MinerError {
    fn from(err: PsbtError) -> Self {
        MinerError::Core(err.into())
    }
}

fn validate_fee_rate(sats_per_vbyte: u64) -> MinerResult<()> {
    if sats_per_vbyte == 0 {
        return Err(MinerError::invalid(
            "sats_per_vbyte must be greater than zero",
        ));
    }
    Ok(())
}

fn collect_outputs(
    outputs: &[OutputRequest],
    network: Network,
) -> MinerResult<(Vec<TxOutput>, Vec<u8>, AddressType)> {
    if outputs.is_empty() {
        return Err(MinerError::invalid("at least one output is required"));
    }

    let change_count = outputs.iter().filter(|o| o.change).count();
    if change_count == 0 {
        return Err(MinerError::MissingChangeOutput);
    }
    if change_count > 1 {
        return Err(MinerError::MultipleChangeOutputs);
    }

    let mut user_outputs = Vec::new();
    let mut change_spk: Option<Vec<u8>> = None;
    let mut change_address_type: Option<AddressType> = None;

    for (idx, output) in outputs.iter().enumerate() {
        let parsed = parse_address_for_network(&output.address, Some(network))
            .map_err(|err| MinerError::invalid(format!("outputs[{idx}] address: {err}")))?;
        let spk = address_to_script_pubkey(&parsed);
        let dust_limit = dust_limit_for_address(parsed.address_type);

        if output.change {
            if change_spk.is_some() {
                return Err(MinerError::MultipleChangeOutputs);
            }
            change_spk = Some(spk);
            change_address_type = Some(parsed.address_type);
            continue;
        }

        let amount = output.amount.ok_or_else(|| {
            MinerError::invalid(format!(
                "outputs[{idx}] amount is required for non-change outputs"
            ))
        })?;
        if amount < dust_limit {
            return Err(MinerError::invalid(format!(
                "outputs[{idx}] amount must be at least {dust_limit} sats"
            )));
        }

        user_outputs.push(TxOutput {
            script_pubkey: spk,
            amount,
        });
    }

    let change_spk = change_spk.ok_or(MinerError::MissingChangeOutput)?;
    let change_address_type =
        change_address_type.ok_or_else(|| MinerError::invalid("missing change output"))?;

    Ok((user_outputs, change_spk, change_address_type))
}

fn validate_cbor_nonce_len(len: usize) -> MinerResult<()> {
    match len {
        1 | 2 | 3 | 5 | 9 => Ok(()),
        _ => Err(MinerError::invalid(
            "invalid CBOR nonce length; must be 1, 2, 3, 5, or 9 bytes",
        )),
    }
}

fn build_distribution_payload(
    distribution: &[u64],
    cbor_nonce: &[u8],
) -> MinerResult<(Vec<u8>, usize)> {
    validate_cbor_nonce_len(cbor_nonce.len())?;

    let payload_len = zeld_distribution_payload_length_with_nonce(distribution, cbor_nonce.len());

    let mut payload = Vec::with_capacity(payload_len);
    payload.extend_from_slice(ZELD_PREFIX);
    payload.extend(encode_cbor_array_header(distribution.len() + 1));
    for &value in distribution {
        payload.extend(encode_cbor_uint(value));
    }
    payload.extend_from_slice(cbor_nonce);

    debug_assert_eq!(payload.len(), payload_len);
    Ok((payload, payload_len))
}

pub fn plan_transaction(
    inputs: Vec<TxInput>,
    outputs: Vec<OutputRequest>,
    network: Network,
    sats_per_vbyte: u64,
    op_return_payload: &[u8],
    distribution: Option<&[u64]>,
) -> MinerResult<TransactionPlan> {
    validate_fee_rate(sats_per_vbyte)?;

    let (user_outputs, change_spk, change_address_type) = collect_outputs(&outputs, network)?;
    let change_dust_limit = dust_limit_for_address(change_address_type);
    if let Some(dist) = distribution {
        let outputs_len = user_outputs.len() + 1; // include change output
        if dist.len() != outputs_len {
            return Err(MinerError::invalid(format!(
                "distribution length ({}) must match outputs ({})",
                dist.len(),
                outputs_len
            )));
        }
    }

    let total_input: u64 = inputs.iter().map(|i| i.amount).sum();
    let outputs_sum: u64 = user_outputs.iter().map(|o| o.amount).sum();

    // Include change output (amount placeholder) for accurate fee estimation.
    let mut outputs_for_fee = user_outputs.clone();
    outputs_for_fee.push(TxOutput {
        script_pubkey: change_spk.clone(),
        amount: 0,
    });

    let (op_return_script, op_return_size, distribution_owned) = if let Some(dist) = distribution {
        let (payload, payload_len) = build_distribution_payload(dist, op_return_payload)?;
        let script = create_op_return_script(&payload);
        (script, payload_len, Some(dist.to_vec()))
    } else {
        let script = create_op_return_script(op_return_payload);
        (script, op_return_payload.len(), None)
    };

    let vsize = calculate_vsize(&inputs, &outputs_for_fee, op_return_size);
    let fee = calculate_fee(vsize, sats_per_vbyte);
    let change_amount = calculate_change(total_input, outputs_sum, fee, change_dust_limit)
        .map_err(|err| match err {
            FeeError::InsufficientFunds => MinerError::invalid("insufficient funds for outputs"),
            FeeError::DustOutput => MinerError::invalid(format!(
                "change would be dust (requires at least {change_dust_limit} sats)"
            )),
        })?;

    let change_output = TxOutput {
        script_pubkey: change_spk,
        amount: change_amount,
    };

    Ok(TransactionPlan {
        inputs,
        user_outputs,
        change_output,
        op_return_script,
        op_return_size,
        distribution: distribution_owned,
    })
}

pub fn build_mining_template(
    plan: &TransactionPlan,
    nonce_len: usize,
) -> MinerResult<MiningTemplate> {
    let outputs_before = plan.user_outputs.clone();
    let outputs_after = vec![plan.change_output.clone()];

    let (prefix, suffix) = if let Some(dist) = plan.distribution.as_ref() {
        validate_cbor_nonce_len(nonce_len)?;
        split_tx_for_distribution_mining(
            &plan.inputs,
            &outputs_before,
            &outputs_after,
            dist,
            nonce_len,
        )
        .map_err(MinerError::from)?
    } else {
        split_tx_for_mining(&plan.inputs, &outputs_before, &outputs_after, nonce_len)
            .map_err(MinerError::from)?
    };

    Ok(MiningTemplate {
        prefix,
        suffix,
        nonce_len: nonce_len as u8,
    })
}

pub fn build_psbt_from_plan(plan: &TransactionPlan) -> MinerResult<(String, [u8; 32])> {
    let mut outputs = plan.user_outputs.clone();
    outputs.push(TxOutput {
        script_pubkey: plan.op_return_script.clone(),
        amount: 0,
    });
    outputs.push(plan.change_output.clone());

    let psbt_bytes = create_psbt(&plan.inputs, &outputs).map_err(MinerError::from)?;
    let psbt_b64 = psbt_to_base64(&psbt_bytes);

    let unsigned = crate::tx::serialize_tx_for_txid(&plan.inputs, &outputs);
    let txid = double_sha256(&unsigned);

    Ok((psbt_b64, txid))
}

pub fn txid_to_hex(hash: &[u8; 32]) -> String {
    let mut bytes = *hash;
    bytes.reverse();
    hex::encode(bytes)
}

pub fn nonce_len_for_range(start_nonce: u64, batch_size: u32) -> Result<u8, &'static str> {
    if batch_size == 0 {
        return Err("batch_size must be positive");
    }
    let last = start_nonce
        .checked_add(batch_size as u64 - 1)
        .ok_or("nonce range overflow")?;

    let start_len = nonce_byte_length(start_nonce);
    let last_len = nonce_byte_length(last);
    if start_len != last_len {
        return Err("nonce range crosses byte-length boundary; split batch");
    }
    if start_len > 8 {
        return Err("nonce range exceeds 8 bytes");
    }
    Ok(start_len as u8)
}

fn cbor_nonce_len_for_range(start_nonce: u64, batch_size: u32) -> Result<u8, &'static str> {
    if batch_size == 0 {
        return Err("batch_size must be positive");
    }
    let last = start_nonce
        .checked_add(batch_size as u64 - 1)
        .ok_or("nonce range overflow")?;

    let start_len = cbor_nonce_byte_length(start_nonce);
    let last_len = cbor_nonce_byte_length(last);
    if start_len != last_len {
        return Err("nonce range crosses CBOR length boundary; split batch");
    }
    if start_len > 9 {
        return Err("nonce range exceeds 9 bytes");
    }
    Ok(start_len as u8)
}

pub fn split_nonce_segments(
    start_nonce: u64,
    batch_size: u32,
) -> Result<Vec<NonceSegment>, &'static str> {
    if batch_size == 0 {
        return Err("batch_size must be positive");
    }
    let end = start_nonce
        .checked_add(batch_size as u64 - 1)
        .ok_or("nonce range overflow")?;

    let mut segments = Vec::new();
    let mut current = start_nonce;

    while current <= end {
        let len = nonce_byte_length(current) as u8;
        let max_for_len = if len == 8 {
            u64::MAX
        } else {
            (1u64 << (len as u32 * 8)) - 1
        };
        let seg_end = core::cmp::min(end, max_for_len);
        let seg_size_u64 = seg_end - current + 1;
        let seg_size = u32::try_from(seg_size_u64).map_err(|_| "segment size exceeds u32")?;
        segments.push(NonceSegment {
            start: current,
            size: seg_size,
            nonce_len: len,
        });
        if seg_end == u64::MAX {
            break;
        }
        current = seg_end.checked_add(1).ok_or("nonce increment overflow")?;
    }

    Ok(segments)
}

fn max_nonce_for_cbor_len(len: u8) -> u64 {
    match len {
        1 => 23,
        2 => 255,
        3 => 65_535,
        5 => 0xFFFF_FFFF,
        9 => u64::MAX,
        _ => unreachable!("validated earlier"),
    }
}

pub fn split_nonce_segments_cbor(
    start_nonce: u64,
    batch_size: u32,
) -> Result<Vec<NonceSegment>, &'static str> {
    if batch_size == 0 {
        return Err("batch_size must be positive");
    }
    let end = start_nonce
        .checked_add(batch_size as u64 - 1)
        .ok_or("nonce range overflow")?;

    let mut segments = Vec::new();
    let mut current = start_nonce;

    while current <= end {
        let len = cbor_nonce_byte_length(current) as u8;
        let max_for_len = max_nonce_for_cbor_len(len);
        let seg_end = core::cmp::min(end, max_for_len);
        let seg_size_u64 = seg_end - current + 1;
        let seg_size = u32::try_from(seg_size_u64).map_err(|_| "segment size exceeds u32")?;
        segments.push(NonceSegment {
            start: current,
            size: seg_size,
            nonce_len: len,
        });
        if seg_end == u64::MAX {
            break;
        }
        current = seg_end.checked_add(1).ok_or("nonce increment overflow")?;
    }

    Ok(segments)
}

pub fn mine_batch(
    tx_prefix: &[u8],
    tx_suffix: &[u8],
    start_nonce: u64,
    batch_size: u32,
    target_zeros: u8,
) -> Result<Option<MineResult>, &'static str> {
    mine_batch_with_cbor(
        tx_prefix,
        tx_suffix,
        start_nonce,
        batch_size,
        target_zeros,
        false,
    )
}

pub fn mine_batch_with_cbor(
    tx_prefix: &[u8],
    tx_suffix: &[u8],
    start_nonce: u64,
    batch_size: u32,
    target_zeros: u8,
    use_cbor_nonce: bool,
) -> Result<Option<MineResult>, &'static str> {
    let nonce_len = if use_cbor_nonce {
        cbor_nonce_len_for_range(start_nonce, batch_size)?
    } else {
        nonce_len_for_range(start_nonce, batch_size)?
    };

    let mut buffer = Vec::with_capacity(tx_prefix.len() + tx_suffix.len() + nonce_len as usize);
    let mut nonce_buf = [0u8; 9];

    for offset in 0..batch_size {
        let nonce = match start_nonce.checked_add(offset as u64) {
            Some(n) => n,
            None => break,
        };

        let written = if use_cbor_nonce {
            let encoded = encode_cbor_uint(nonce);
            if encoded.len() != nonce_len as usize {
                return Err("CBOR nonce length mismatch for segment");
            }
            nonce_buf[..encoded.len()].copy_from_slice(&encoded);
            encoded.len()
        } else {
            encode_nonce_fixed_into(nonce, nonce_len, &mut nonce_buf)?
        };

        buffer.clear();
        buffer.extend_from_slice(tx_prefix);
        buffer.extend_from_slice(&nonce_buf[..written]);
        buffer.extend_from_slice(tx_suffix);

        let hash = double_sha256(&buffer);
        if hash_meets_target(&hash, target_zeros) {
            return Ok(Some(MineResult { nonce, txid: hash }));
        }
    }

    Ok(None)
}

pub fn mine_range(
    prefix: &[u8],
    suffix: &[u8],
    start_nonce: u64,
    batch_size: u32,
    target_zeros: u8,
) -> Result<Option<MineResult>, &'static str> {
    let segments = split_nonce_segments(start_nonce, batch_size)?;

    for segment in segments {
        if let Some(found) = mine_batch(prefix, suffix, segment.start, segment.size, target_zeros)?
        {
            return Ok(Some(found));
        }
    }

    Ok(None)
}

fn encode_nonce_fixed_into(
    nonce: u64,
    nonce_len: u8,
    out: &mut [u8; 9],
) -> Result<usize, &'static str> {
    if nonce_len == 0 || nonce_len > 9 {
        return Err("nonce_len must be between 1 and 9 bytes");
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
        return Err("nonce length does not match minimal encoding");
    }

    let src_start = 9 - minimal_len;
    out[..target_len].copy_from_slice(&buf[src_start..]);
    Ok(target_len)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        address::{address_to_script_pubkey, parse_address_for_network},
        cbor::encode_cbor_uint,
        encode_nonce,
        fees::{calculate_fee, calculate_vsize},
        hash::hash_meets_target,
        tx::{
            create_zeld_distribution_op_return, serialize_tx_for_txid,
            zeld_distribution_payload_length_with_nonce, DEFAULT_SEQUENCE, ZELD_PREFIX,
        },
    };
    use bech32::{hrp, segwit};

    fn sample_addresses(network: Network) -> (String, String) {
        debug_assert!(matches!(network, Network::Mainnet));
        let user = segwit::encode(hrp::BC, segwit::VERSION_0, &[0x11u8; 20]).unwrap();
        let change = segwit::encode(hrp::BC, segwit::VERSION_0, &[0x22u8; 20]).unwrap();
        (user, change)
    }

    fn segwit_address(fill: u8, network: Network) -> String {
        debug_assert!(matches!(network, Network::Mainnet));
        segwit::encode(hrp::BC, segwit::VERSION_0, &[fill; 20]).unwrap()
    }

    fn script_pubkey(addr: &str, network: Network) -> Vec<u8> {
        let parsed = parse_address_for_network(addr, Some(network)).expect("address must parse");
        address_to_script_pubkey(&parsed)
    }

    fn sample_input(amount: u64, change_addr: &str, network: Network) -> TxInput {
        TxInput {
            txid: [0x11u8; 32],
            vout: 0,
            script_pubkey: script_pubkey(change_addr, network),
            amount,
            sequence: DEFAULT_SEQUENCE,
        }
    }

    fn outputs(user_amount: u64, user_addr: &str, change_addr: &str) -> Vec<OutputRequest> {
        vec![
            OutputRequest {
                address: user_addr.to_string(),
                amount: Some(user_amount),
                change: false,
            },
            OutputRequest {
                address: change_addr.to_string(),
                amount: None,
                change: true,
            },
        ]
    }

    fn dust_limit_for_addr(addr: &str, network: Network) -> u64 {
        let parsed = parse_address_for_network(addr, Some(network)).expect("address must parse");
        super::dust_limit_for_address(parsed.address_type)
    }

    #[test]
    fn plan_transaction_computes_fees_change_and_op_return() {
        let network = Network::Mainnet;
        let (user_addr, change_addr) = sample_addresses(network);
        let inputs = vec![sample_input(120_000, &change_addr, network)];
        let outputs = outputs(60_000, &user_addr, &change_addr);
        let op_return = [0xAAu8, 0xBB, 0xCC];

        let plan = plan_transaction(
            inputs.clone(),
            outputs.clone(),
            network,
            2, // sats/vB
            &op_return,
            None,
        )
        .expect("plan should succeed");

        // Fee estimation should align with the plan's computed change.
        let user_spk = script_pubkey(&user_addr, network);
        let change_spk = script_pubkey(&change_addr, network);
        let outputs_for_fee = vec![
            TxOutput {
                script_pubkey: user_spk.clone(),
                amount: outputs[0].amount.unwrap(),
            },
            TxOutput {
                script_pubkey: change_spk.clone(),
                amount: 0,
            },
        ];

        let vsize = calculate_vsize(&inputs, &outputs_for_fee, op_return.len());
        let fee = calculate_fee(vsize, 2);
        let expected_change = inputs[0].amount - outputs_for_fee[0].amount - fee;
        let change_dust_limit = dust_limit_for_addr(&change_addr, network);

        assert_eq!(plan.user_outputs.len(), 1);
        assert_eq!(plan.user_outputs[0].script_pubkey, user_spk);
        assert_eq!(plan.change_output.script_pubkey, change_spk);
        assert_eq!(plan.change_output.amount, expected_change);
        assert!(
            plan.change_output.amount >= change_dust_limit,
            "change must remain above the dust limit"
        );
        assert_eq!(plan.op_return_size, op_return.len());
        assert_eq!(plan.op_return_script, create_op_return_script(&op_return));
    }

    #[test]
    fn plan_transaction_rejects_dusty_change() {
        let network = Network::Mainnet;
        let (user_addr, change_addr) = sample_addresses(network);
        let inputs = vec![sample_input(20_000, &change_addr, network)];
        let outputs = outputs(8_000, &user_addr, &change_addr);
        let op_return = [0u8];

        // Craft a fee rate that keeps funds positive but pushes change below the dust limit.
        let user_spk = script_pubkey(&user_addr, network);
        let change_spk = script_pubkey(&change_addr, network);
        let outputs_for_fee = vec![
            TxOutput {
                script_pubkey: user_spk,
                amount: outputs[0].amount.unwrap(),
            },
            TxOutput {
                script_pubkey: change_spk,
                amount: 0,
            },
        ];
        let vsize = calculate_vsize(&inputs, &outputs_for_fee, op_return.len());
        let total_input: u64 = inputs.iter().map(|i| i.amount).sum();
        let outputs_sum = outputs_for_fee[0].amount;
        let change_dust_limit = dust_limit_for_addr(&change_addr, network);
        let max_fee_before_dust = total_input - outputs_sum - change_dust_limit;
        let sats_per_vbyte = (max_fee_before_dust / vsize as u64) + 1;

        let err = plan_transaction(inputs, outputs, network, sats_per_vbyte, &op_return, None)
            .expect_err("fee choice should make change dust");

        match err {
            MinerError::InvalidInput(msg) => {
                assert!(
                    msg.to_lowercase().contains("dust"),
                    "expected dust-related error, got: {msg}"
                );
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn nonce_len_and_segments_handle_boundaries() {
        assert_eq!(nonce_len_for_range(0, 1).unwrap(), 1);
        assert!(nonce_len_for_range(0xff, 2).is_err()); // crosses byte boundary

        let segments = split_nonce_segments(0xf0, 0x20).expect("must split at 0xff");
        assert_eq!(segments.len(), 2);
        assert_eq!(
            segments[0],
            NonceSegment {
                start: 0xf0,
                size: 16,
                nonce_len: 1
            }
        );
        assert_eq!(
            segments[1],
            NonceSegment {
                start: 0x100,
                size: 16,
                nonce_len: 2
            }
        );
    }

    #[test]
    fn split_nonce_segments_cbor_respects_cbor_boundaries() {
        let segments = split_nonce_segments_cbor(0, 300).expect("cbor split ok");
        assert_eq!(segments.len(), 3);
        assert_eq!(
            segments[0],
            NonceSegment {
                start: 0,
                size: 24,
                nonce_len: 1
            }
        );
        assert_eq!(
            segments[1],
            NonceSegment {
                start: 24,
                size: 232,
                nonce_len: 2
            }
        );
        assert_eq!(
            segments[2],
            NonceSegment {
                start: 256,
                size: 44,
                nonce_len: 3
            }
        );
    }

    #[test]
    fn mine_batch_finds_first_nonce_for_zero_target() {
        let prefix = b"prefix";
        let suffix = b"suffix";
        let start_nonce = 7u64;
        let result = mine_batch(prefix, suffix, start_nonce, 4, 0)
            .expect("batch ok")
            .expect("target 0 must accept first hash");

        assert_eq!(result.nonce, start_nonce);

        let mut full_tx = Vec::new();
        full_tx.extend_from_slice(prefix);
        full_tx.extend_from_slice(&encode_nonce(start_nonce));
        full_tx.extend_from_slice(suffix);
        assert_eq!(result.txid, double_sha256(&full_tx));
    }

    #[test]
    fn hash_meets_target_guardrails() {
        let mut hash = [0xffu8; 32];
        assert!(!hash_meets_target(&hash, 1));
        assert!(hash_meets_target(&hash, 0));

        hash[31] = 0x0f; // one leading zero nibble
        assert!(hash_meets_target(&hash, 1));
        assert!(!hash_meets_target(&hash, 2));
    }

    #[test]
    fn plan_transaction_with_distribution_uses_cbor_payload() {
        let network = Network::Mainnet;
        let (user_addr, change_addr) = sample_addresses(network);
        let inputs = vec![sample_input(120_000, &change_addr, network)];
        let addr_b = segwit_address(0x33, network);
        let addr_c = segwit_address(0x44, network);
        let outputs = vec![
            OutputRequest {
                address: user_addr,
                amount: Some(40_000),
                change: false,
            },
            OutputRequest {
                address: addr_b,
                amount: Some(30_000),
                change: false,
            },
            OutputRequest {
                address: addr_c,
                amount: Some(20_000),
                change: false,
            },
            OutputRequest {
                address: change_addr,
                amount: None,
                change: true,
            },
        ];
        let distribution = [600u64, 300, 100, 0];
        let cbor_nonce = encode_cbor_uint(42);

        let plan = plan_transaction(
            inputs,
            outputs,
            network,
            2,
            &cbor_nonce,
            Some(&distribution),
        )
        .expect("plan should include distribution");

        assert!(plan.op_return_script.windows(4).any(|w| w == ZELD_PREFIX));
        assert_eq!(plan.distribution.as_deref(), Some(distribution.as_slice()));
        assert_eq!(
            plan.op_return_size,
            zeld_distribution_payload_length_with_nonce(&distribution, cbor_nonce.len())
        );
        assert_eq!(plan.user_outputs.len() + 1, distribution.len());
    }

    #[test]
    fn plan_transaction_with_distribution() {
        let network = Network::Mainnet;
        let (user_addr, change_addr) = sample_addresses(network);
        let addr_b = segwit_address(0x55, network);
        let addr_c = segwit_address(0x66, network);
        let inputs = vec![sample_input(200_000, &change_addr, network)];
        let outputs = vec![
            OutputRequest {
                address: user_addr,
                amount: Some(50_000),
                change: false,
            },
            OutputRequest {
                address: addr_b,
                amount: Some(40_000),
                change: false,
            },
            OutputRequest {
                address: addr_c,
                amount: Some(30_000),
                change: false,
            },
            OutputRequest {
                address: change_addr,
                amount: None,
                change: true,
            },
        ];
        let distribution = [600u64, 300, 100, 0];
        let cbor_nonce = encode_cbor_uint(1);

        let plan = plan_transaction(
            inputs,
            outputs,
            network,
            2,
            &cbor_nonce,
            Some(&distribution),
        )
        .expect("plan should succeed with matching distribution length");

        let expected_payload_len =
            zeld_distribution_payload_length_with_nonce(&distribution, cbor_nonce.len());

        assert_eq!(plan.distribution.as_deref(), Some(distribution.as_slice()));
        assert_eq!(plan.user_outputs.len() + 1, distribution.len());
        assert_eq!(plan.op_return_size, expected_payload_len);
        assert_eq!(plan.op_return_script[0], 0x6a); // OP_RETURN
        assert!(plan
            .op_return_script
            .windows(ZELD_PREFIX.len())
            .any(|w| w == ZELD_PREFIX));
    }

    #[test]
    fn mining_template_splits_correctly_with_distribution() {
        let network = Network::Mainnet;
        let (user_addr, change_addr) = sample_addresses(network);
        let addr_b = segwit_address(0x77, network);
        let addr_c = segwit_address(0x88, network);
        let inputs = vec![sample_input(200_000, &change_addr, network)];
        let outputs = vec![
            OutputRequest {
                address: user_addr,
                amount: Some(50_000),
                change: false,
            },
            OutputRequest {
                address: addr_b,
                amount: Some(40_000),
                change: false,
            },
            OutputRequest {
                address: addr_c,
                amount: Some(30_000),
                change: false,
            },
            OutputRequest {
                address: change_addr,
                amount: None,
                change: true,
            },
        ];
        let distribution = [600u64, 300, 100, 0];
        let cbor_nonce = encode_cbor_uint(42);

        let plan = plan_transaction(
            inputs.clone(),
            outputs,
            network,
            2,
            &cbor_nonce,
            Some(&distribution),
        )
        .expect("plan should include distribution");

        let template =
            build_mining_template(&plan, cbor_nonce.len()).expect("template should build");
        assert_eq!(template.nonce_len as usize, cbor_nonce.len());

        let mut rebuilt = template.prefix.clone();
        rebuilt.extend_from_slice(&cbor_nonce);
        rebuilt.extend_from_slice(&template.suffix);

        let mut full_outputs = plan.user_outputs.clone();
        full_outputs.push(TxOutput {
            script_pubkey: plan.op_return_script.clone(),
            amount: 0,
        });
        full_outputs.push(plan.change_output);

        let expected_tx = serialize_tx_for_txid(&inputs, &full_outputs);
        assert_eq!(rebuilt, expected_tx);
    }

    #[test]
    fn mined_tx_has_valid_zeld_op_return() {
        let network = Network::Mainnet;
        let (user_addr, change_addr) = sample_addresses(network);
        let addr_b = segwit_address(0x99, network);
        let addr_c = segwit_address(0xaa, network);
        let inputs = vec![sample_input(200_000, &change_addr, network)];
        let outputs = vec![
            OutputRequest {
                address: user_addr,
                amount: Some(50_000),
                change: false,
            },
            OutputRequest {
                address: addr_b,
                amount: Some(40_000),
                change: false,
            },
            OutputRequest {
                address: addr_c,
                amount: Some(30_000),
                change: false,
            },
            OutputRequest {
                address: change_addr,
                amount: None,
                change: true,
            },
        ];
        let distribution = [600u64, 300, 100, 0];
        let placeholder_nonce = encode_cbor_uint(0);

        let plan = plan_transaction(
            inputs.clone(),
            outputs,
            network,
            1,
            &placeholder_nonce,
            Some(&distribution),
        )
        .expect("plan should include distribution");

        let template =
            build_mining_template(&plan, placeholder_nonce.len()).expect("template should build");

        let mine_result = mine_batch_with_cbor(&template.prefix, &template.suffix, 0, 4, 0, true)
            .expect("mining must succeed")
            .expect("target 0 accepts first nonce");

        let cbor_nonce = encode_cbor_uint(mine_result.nonce);
        let mut mined_tx = template.prefix.clone();
        mined_tx.extend_from_slice(&cbor_nonce);
        mined_tx.extend_from_slice(&template.suffix);

        let expected_op_return =
            create_zeld_distribution_op_return(&distribution, mine_result.nonce);
        let mut outputs_full = plan.user_outputs.clone();
        outputs_full.push(TxOutput {
            script_pubkey: expected_op_return.clone(),
            amount: 0,
        });
        outputs_full.push(plan.change_output);
        let expected_tx = serialize_tx_for_txid(&inputs, &outputs_full);

        assert_eq!(mined_tx, expected_tx);
        assert!(expected_op_return
            .windows(ZELD_PREFIX.len())
            .any(|w| w == ZELD_PREFIX));
    }
}
