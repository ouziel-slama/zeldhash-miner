use crate::tx::{encode_varint, TxInput, TxOutput};
use thiserror::Error;

// Typical witness sizes to keep fees tight while remaining realistic.
// If callers need a hard upper bound, they can bump these values at call sites.
const P2WPKH_WITNESS_TYPICAL: usize = 1 /* stack items */ + 1 /* sig len */ + 72 /* sig */ +
    1 /* pk len */ + 33 /* pk */;
const P2TR_WITNESS_TYPICAL: usize = 1 /* stack items */ + 1 /* sig len */ + 64 /* sig */;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum FeeError {
    #[error("insufficient funds for outputs and fee")]
    InsufficientFunds,
    #[error("change would be dust")]
    DustOutput,
}

/// Estimate virtual size (vbytes) for a SegWit transaction with a fixed-size OP_RETURN.
pub fn calculate_vsize(inputs: &[TxInput], outputs: &[TxOutput], op_return_size: usize) -> usize {
    let base = base_tx_size(inputs, outputs, op_return_size);
    let witness = witness_size(inputs);
    let weight = base * 4 + witness;
    (weight + 3) / 4
}

/// Convenience wrapper that uses the caller-provided nonce length for OP_RETURN sizing.
pub fn calculate_vsize_with_nonce_len(
    inputs: &[TxInput],
    outputs: &[TxOutput],
    nonce_len: usize,
) -> usize {
    calculate_vsize(inputs, outputs, nonce_len)
}

/// Fee in satoshis for a given vsize and rate.
pub fn calculate_fee(vsize: usize, sat_per_vbyte: u64) -> u64 {
    vsize as u64 * sat_per_vbyte
}

/// Compute change amount, enforcing dust and sufficient funds rules.
pub fn calculate_change(
    total_input: u64,
    outputs_sum: u64,
    fee: u64,
    dust_limit: u64,
) -> Result<u64, FeeError> {
    if total_input < outputs_sum + fee {
        return Err(FeeError::InsufficientFunds);
    }

    let change = total_input - outputs_sum - fee;
    if change < dust_limit {
        return Err(FeeError::DustOutput);
    }

    Ok(change)
}

fn base_tx_size(inputs: &[TxInput], outputs: &[TxOutput], op_return_size: usize) -> usize {
    let mut size = 4; // version

    size += encode_varint(inputs.len() as u64).len();
    size += inputs.len() * (32 + 4 + 1 + 4); // txid + vout + script len (0) + sequence

    let total_outputs = outputs.len() + 1; // include OP_RETURN
    size += encode_varint(total_outputs as u64).len();

    // user outputs
    for output in outputs {
        size += output_size(output);
    }

    // OP_RETURN output
    let push_prefix_len = pushdata_prefix_len(op_return_size);
    let script_len = 1 + push_prefix_len + op_return_size;
    size += 8; // amount
    size += encode_varint(script_len as u64).len();
    size += script_len;

    size += 4; // locktime
    size
}

fn output_size(output: &TxOutput) -> usize {
    8 + encode_varint(output.script_pubkey.len() as u64).len() + output.script_pubkey.len()
}

fn witness_size(inputs: &[TxInput]) -> usize {
    let mut total = 0usize;
    let mut any_segwit = false;

    for input in inputs {
        match input.script_pubkey.as_slice() {
            // P2WPKH: stack = [sig, pubkey]
            // Typical size: 1 (items) + 1 (sig len) + 72 (sig) + 1 (pk len) + 33 (pk) = 108 bytes.
            [0x00, 0x14, ..] if input.script_pubkey.len() == 22 => {
                total += P2WPKH_WITNESS_TYPICAL;
                any_segwit = true;
            }
            // P2TR key-path: stack = [sig]
            // Typical sizes: 1 (items) + 1 (sig len) + 64 (sig) = 66 bytes.
            [0x51, 0x20, ..] if input.script_pubkey.len() == 34 => {
                total += P2TR_WITNESS_TYPICAL;
                any_segwit = true;
            }
            _ => {}
        }
    }

    if any_segwit {
        // SegWit marker + flag (weight 1 each, counted in witness section for weight calc).
        total += 2;
    }

    total
}

fn pushdata_prefix_len(len: usize) -> usize {
    if len <= 0x4b {
        1
    } else if len <= 0xff {
        2
    } else if len <= 0xffff {
        3
    } else {
        5
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_input_p2wpkh() -> TxInput {
        let mut spk = vec![0x00, 0x14];
        spk.extend(vec![0x11; 20]);
        TxInput {
            txid: [0x22u8; 32],
            vout: 0,
            script_pubkey: spk,
            amount: 100_000,
            sequence: crate::tx::DEFAULT_SEQUENCE,
        }
    }

    fn sample_output() -> TxOutput {
        let mut spk = vec![0x00, 0x14];
        spk.extend(vec![0x33; 20]);
        TxOutput {
            script_pubkey: spk,
            amount: 50_000,
        }
    }

    #[test]
    fn estimates_vsize_with_op_return() {
        let inputs = vec![sample_input_p2wpkh()];
        let outputs = vec![sample_output()];
        let vsize = calculate_vsize_with_nonce_len(&inputs, &outputs, 8);
        assert_eq!(vsize, 129);
    }

    #[test]
    fn detects_insufficient_funds_and_dust() {
        let err = calculate_change(10_000, 9_500, 600, 330).unwrap_err();
        assert_eq!(err, FeeError::InsufficientFunds);

        // Construct a change amount below the dust limit (400 change was no longer dust).
        let err = calculate_change(10_000, 9_400, 271, 330).unwrap_err(); // change = 329
        assert_eq!(err, FeeError::DustOutput);
    }

    #[test]
    fn fixed_nonce_vsize_matches_constant() {
        let inputs = vec![sample_input_p2wpkh()];
        let outputs = vec![sample_output()];

        let v_fixed = calculate_vsize_with_nonce_len(&inputs, &outputs, 8);
        let v_manual = calculate_vsize(&inputs, &outputs, 8);
        assert_eq!(v_fixed, v_manual);
    }

    #[test]
    fn matches_known_vsize_against_manual_weight() {
        let inputs = vec![sample_input_p2wpkh()];
        let outputs = vec![sample_output()];
        let op_return_size = 4; // small OP_RETURN payload (e.g., nonce bytes)

        let vsize = calculate_vsize(&inputs, &outputs, op_return_size);

        // Manual weight calculation for regression:
        // base size (non-witness) + witness size / 4 (rounded up).
        let mut base = 4; // version
        base += encode_varint(inputs.len() as u64).len();
        base += inputs.len() * (32 + 4 + 1 + 4); // txid + vout + script len (0) + sequence

        let total_outputs = outputs.len() + 1; // add OP_RETURN
        base += encode_varint(total_outputs as u64).len();

        for output in &outputs {
            base += output_size(output);
        }

        let push_prefix_len = pushdata_prefix_len(op_return_size);
        let op_return_script_len = 1 + push_prefix_len + op_return_size;
        base += 8; // amount
        base += encode_varint(op_return_script_len as u64).len();
        base += op_return_script_len; // script body

        base += 4; // locktime

        // Witness: one P2WPKH input (typical 108 bytes) + marker/flag (weight 2)
        let witness = P2WPKH_WITNESS_TYPICAL + 2;
        let expected = (base * 4 + witness + 3) / 4;

        assert_eq!(vsize, expected);
        assert_eq!(vsize, 125); // guardrail against regressions
    }

    #[test]
    fn uses_typical_p2wpkh_witness_size_for_fees() {
        let inputs = vec![sample_input_p2wpkh()];
        let outputs = vec![sample_output()];
        let op_return_size = 1;

        let vsize = calculate_vsize(&inputs, &outputs, op_return_size);

        let mut base = 4;
        base += encode_varint(inputs.len() as u64).len();
        base += inputs.len() * (32 + 4 + 1 + 4);

        let total_outputs = outputs.len() + 1;
        base += encode_varint(total_outputs as u64).len();
        for output in &outputs {
            base += output_size(output);
        }

        let push_prefix_len = pushdata_prefix_len(op_return_size);
        let op_return_script_len = 1 + push_prefix_len + op_return_size;
        base += 8;
        base += encode_varint(op_return_script_len as u64).len();
        base += op_return_script_len;
        base += 4;

        let witness = P2WPKH_WITNESS_TYPICAL + 2; // include marker/flag weight
        let expected = (base * 4 + witness + 3) / 4;
        assert_eq!(vsize, expected);
    }
}
