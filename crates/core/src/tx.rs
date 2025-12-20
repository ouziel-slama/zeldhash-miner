use alloc::{vec, vec::Vec};

use crate::cbor::{
    cbor_array_header_length, cbor_uint_length, encode_cbor_array_header, encode_cbor_uint,
};
use thiserror::Error;

/// ZELD protocol prefix for distribution OP_RETURN.
pub const ZELD_PREFIX: &[u8; 4] = b"ZELD";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TxInput {
    pub txid: [u8; 32],
    pub vout: u32,
    pub script_pubkey: Vec<u8>,
    pub amount: u64,
    pub sequence: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TxOutput {
    pub script_pubkey: Vec<u8>,
    pub amount: u64,
}

pub const DEFAULT_SEQUENCE: u32 = 0xffff_fffd;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum TxError {
    #[error("data too large to encode")]
    DataTooLarge,
    #[error("invalid CBOR nonce length; must be 1, 2, 3, 5, or 9 bytes")]
    InvalidCborNonceLength,
}

/// Bitcoin varint encoding as defined in BIP-008.
pub fn encode_varint(n: u64) -> Vec<u8> {
    match n {
        0..=0xfc => vec![n as u8],
        0xfd..=0xffff => {
            let mut out = vec![0xfd];
            out.extend_from_slice(&(n as u16).to_le_bytes());
            out
        }
        0x1_0000..=0xffff_ffff => {
            let mut out = vec![0xfe];
            out.extend_from_slice(&(n as u32).to_le_bytes());
            out
        }
        _ => {
            let mut out = vec![0xff];
            out.extend_from_slice(&n.to_le_bytes());
            out
        }
    }
}

/// Minimal push opcode prefix for a given length (no data bytes).
fn pushdata_prefix(len: usize) -> Result<Vec<u8>, TxError> {
    if len <= 0x4b {
        Ok(vec![len as u8])
    } else if len <= 0xff {
        Ok(vec![0x4c, len as u8])
    } else if len <= 0xffff {
        let mut out = vec![0x4d];
        out.extend_from_slice(&(len as u16).to_le_bytes());
        Ok(out)
    } else if len <= 0xffff_ffff {
        let mut out = vec![0x4e];
        out.extend_from_slice(&(len as u32).to_le_bytes());
        Ok(out)
    } else {
        Err(TxError::DataTooLarge)
    }
}

/// Build an OP_RETURN script with minimal push encoding for the provided data.
pub fn create_op_return_script(data: &[u8]) -> Vec<u8> {
    let mut script = Vec::with_capacity(1 + data.len() + 5);
    script.push(0x6a); // OP_RETURN
    script.extend(pushdata_prefix(data.len()).expect("data length is bounded by usize"));
    script.extend_from_slice(data);
    script
}

/// Build an OP_RETURN script for ZELD distribution with an embedded nonce.
///
/// Format: `OP_RETURN | push | "ZELD" | CBOR([distribution..., nonce])`
///
/// The nonce is appended as the last element of the CBOR array. The protocol
/// automatically trims extra elements that exceed the number of outputs, so
/// the nonce will be stripped when the distribution is parsed.
///
/// # Arguments
///
/// * `distribution` - The ZELD distribution values (one per output)
/// * `nonce` - The mining nonce to append
///
/// # Examples
///
/// ```
/// use zeldhash_miner_core::tx::create_zeld_distribution_op_return;
///
/// let script = create_zeld_distribution_op_return(&[600, 300, 100], 42);
/// // OP_RETURN + push(15) + "ZELD" + CBOR([600, 300, 100, 42])
/// // Payload = 4 (ZELD) + 1 (array header) + 3 + 3 + 2 + 2 = 15 bytes
/// assert_eq!(&script[0..2], &[0x6a, 0x0f]); // OP_RETURN + push 15 bytes
/// assert_eq!(&script[2..6], b"ZELD");
/// ```
pub fn create_zeld_distribution_op_return(distribution: &[u64], nonce: u64) -> Vec<u8> {
    // Build the CBOR array: distribution values + nonce
    let array_len = distribution.len() + 1;
    let mut cbor_data = encode_cbor_array_header(array_len);
    for &value in distribution {
        cbor_data.extend(encode_cbor_uint(value));
    }
    cbor_data.extend(encode_cbor_uint(nonce));

    // Build the payload: "ZELD" + CBOR array
    let mut payload = Vec::with_capacity(4 + cbor_data.len());
    payload.extend_from_slice(ZELD_PREFIX);
    payload.extend(cbor_data);

    // Build the OP_RETURN script
    create_op_return_script(&payload)
}

/// Calculate the byte length of a ZELD distribution OP_RETURN payload.
///
/// This is useful for fee estimation. Uses the maximum CBOR nonce length (9 bytes)
/// to ensure fees are sufficient for any nonce value.
///
/// Returns the length of: `"ZELD" + CBOR([distribution..., max_nonce])`
pub fn zeld_distribution_payload_length(distribution: &[u64]) -> usize {
    const MAX_CBOR_NONCE_LEN: usize = 9; // CBOR u64 max length
    4 // "ZELD"
        + cbor_array_header_length(distribution.len() + 1)
        + distribution.iter().map(|&v| cbor_uint_length(v)).sum::<usize>()
        + MAX_CBOR_NONCE_LEN
}

/// Calculate the byte length of a ZELD distribution OP_RETURN payload with a specific nonce length.
///
/// Unlike `zeld_distribution_payload_length`, this uses the exact CBOR length for the
/// given nonce byte length, which is useful for mining when we know the nonce range.
pub fn zeld_distribution_payload_length_with_nonce(
    distribution: &[u64],
    cbor_nonce_len: usize,
) -> usize {
    4 // "ZELD"
        + cbor_array_header_length(distribution.len() + 1)
        + distribution.iter().map(|&v| cbor_uint_length(v)).sum::<usize>()
        + cbor_nonce_len
}

fn write_output(buf: &mut Vec<u8>, output: &TxOutput) {
    buf.extend_from_slice(&output.amount.to_le_bytes());
    buf.extend(encode_varint(output.script_pubkey.len() as u64));
    buf.extend_from_slice(&output.script_pubkey);
}

/// Serialize a transaction without witness data (txid serialization).
pub fn serialize_tx_for_txid(inputs: &[TxInput], outputs: &[TxOutput]) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(&2u32.to_le_bytes()); // version

    buf.extend(encode_varint(inputs.len() as u64));
    for input in inputs {
        buf.extend_from_slice(&input.txid);
        buf.extend_from_slice(&input.vout.to_le_bytes());
        buf.push(0x00); // empty scriptSig
        buf.extend_from_slice(&input.sequence.to_le_bytes()); // sequence (caller provided)
    }

    buf.extend(encode_varint(outputs.len() as u64));
    for output in outputs {
        write_output(&mut buf, output);
    }

    buf.extend_from_slice(&0u32.to_le_bytes()); // locktime
    buf
}

/// Serialize an unsigned transaction in the legacy (non-witness) form.
///
/// PSBTs embed the unsigned transaction without the SegWit marker/flag and
/// without any witness data (BIP174).
pub fn serialize_unsigned_tx(inputs: &[TxInput], outputs: &[TxOutput]) -> Vec<u8> {
    serialize_tx_for_txid(inputs, outputs)
}

/// Split a transaction around the OP_RETURN nonce for mining.
///
/// Prefix includes version, inputs, output count, all outputs before the OP_RETURN,
/// and the OP_RETURN amount + script header (opcode + push opcode).
/// Suffix includes remaining outputs (after the nonce) and locktime.
pub fn split_tx_for_mining(
    inputs: &[TxInput],
    outputs_before_opreturn: &[TxOutput],
    outputs_after_opreturn: &[TxOutput],
    nonce_len: usize,
) -> Result<(Vec<u8>, Vec<u8>), TxError> {
    let mut prefix = Vec::new();
    prefix.extend_from_slice(&2u32.to_le_bytes());

    prefix.extend(encode_varint(inputs.len() as u64));
    for input in inputs {
        prefix.extend_from_slice(&input.txid);
        prefix.extend_from_slice(&input.vout.to_le_bytes());
        prefix.push(0x00);
        prefix.extend_from_slice(&input.sequence.to_le_bytes());
    }

    let total_outputs = outputs_before_opreturn.len() + 1 + outputs_after_opreturn.len();
    prefix.extend(encode_varint(total_outputs as u64));

    for output in outputs_before_opreturn {
        write_output(&mut prefix, output);
    }

    let push_prefix = pushdata_prefix(nonce_len)?;
    let script_len = 1 + push_prefix.len() + nonce_len; // OP_RETURN + push opcode + data

    // OP_RETURN output (value = 0)
    prefix.extend_from_slice(&0u64.to_le_bytes());
    prefix.extend(encode_varint(script_len as u64));
    prefix.push(0x6a);
    prefix.extend_from_slice(&push_prefix);

    let mut suffix = Vec::new();
    for output in outputs_after_opreturn {
        write_output(&mut suffix, output);
    }
    suffix.extend_from_slice(&0u32.to_le_bytes()); // locktime

    Ok((prefix, suffix))
}

/// Split a transaction for mining with ZELD distribution.
///
/// The split point is within the CBOR array, after the distribution values but before the nonce.
/// This allows the mining loop to only vary the CBOR-encoded nonce bytes.
///
/// # Arguments
///
/// * `inputs` - Transaction inputs
/// * `outputs_before_opreturn` - Outputs before the OP_RETURN (typically user outputs)
/// * `outputs_after_opreturn` - Outputs after the OP_RETURN (typically change)
/// * `distribution` - The ZELD distribution values
/// * `cbor_nonce_len` - The byte length of the CBOR-encoded nonce (1, 2, 3, 5, or 9)
///
/// # Returns
///
/// A tuple of (prefix, suffix) where:
/// - prefix: Everything up to and including the distribution values in the CBOR array
/// - suffix: Everything after the nonce (remaining outputs and locktime)
///
/// To reconstruct the full transaction: `prefix + cbor_encode_nonce(nonce) + suffix`
pub fn split_tx_for_distribution_mining(
    inputs: &[TxInput],
    outputs_before_opreturn: &[TxOutput],
    outputs_after_opreturn: &[TxOutput],
    distribution: &[u64],
    cbor_nonce_len: usize,
) -> Result<(Vec<u8>, Vec<u8>), TxError> {
    // Validate CBOR nonce length to avoid mismatched script lengths.
    match cbor_nonce_len {
        1 | 2 | 3 | 5 | 9 => {}
        _ => return Err(TxError::InvalidCborNonceLength),
    }

    let mut prefix = Vec::new();
    prefix.extend_from_slice(&2u32.to_le_bytes()); // version

    prefix.extend(encode_varint(inputs.len() as u64));
    for input in inputs {
        prefix.extend_from_slice(&input.txid);
        prefix.extend_from_slice(&input.vout.to_le_bytes());
        prefix.push(0x00);
        prefix.extend_from_slice(&input.sequence.to_le_bytes());
    }

    let total_outputs = outputs_before_opreturn.len() + 1 + outputs_after_opreturn.len();
    prefix.extend(encode_varint(total_outputs as u64));

    for output in outputs_before_opreturn {
        write_output(&mut prefix, output);
    }

    // Calculate the payload length: "ZELD" + CBOR array
    let payload_len = zeld_distribution_payload_length_with_nonce(distribution, cbor_nonce_len);
    let push_prefix = pushdata_prefix(payload_len)?;
    let script_len = 1 + push_prefix.len() + payload_len; // OP_RETURN + push opcode + data

    // OP_RETURN output (value = 0)
    prefix.extend_from_slice(&0u64.to_le_bytes());
    prefix.extend(encode_varint(script_len as u64));
    prefix.push(0x6a); // OP_RETURN
    prefix.extend_from_slice(&push_prefix);

    // "ZELD" prefix
    prefix.extend_from_slice(ZELD_PREFIX);

    // CBOR array header (for distribution.len() + 1 elements)
    let array_len = distribution.len() + 1;
    prefix.extend(encode_cbor_array_header(array_len));

    // All CBOR-encoded distribution values
    for &value in distribution {
        prefix.extend(encode_cbor_uint(value));
    }

    // The nonce slot is between prefix and suffix (CBOR-encoded nonce goes here)

    let mut suffix = Vec::new();
    for output in outputs_after_opreturn {
        write_output(&mut suffix, output);
    }
    suffix.extend_from_slice(&0u32.to_le_bytes()); // locktime

    Ok((prefix, suffix))
}

#[cfg(test)]
mod tests {
    use super::{
        create_op_return_script, create_zeld_distribution_op_return, encode_varint,
        serialize_tx_for_txid, serialize_unsigned_tx, split_tx_for_distribution_mining,
        split_tx_for_mining, zeld_distribution_payload_length,
        zeld_distribution_payload_length_with_nonce, TxError, TxInput, TxOutput, DEFAULT_SEQUENCE,
        ZELD_PREFIX,
    };
    use crate::cbor::{cbor_uint_length, encode_cbor_uint};
    use crate::hash::double_sha256;
    use bitcoin::{
        absolute::LockTime,
        consensus,
        hashes::{sha256d, Hash},
        transaction::Version,
        Amount, Transaction, TxIn, TxOut, Txid,
    };

    fn p2wpkh_script(fill: u8) -> Vec<u8> {
        let mut script = vec![0x00, 0x14];
        script.extend(vec![fill; 20]);
        script
    }

    fn sample_input() -> TxInput {
        TxInput {
            txid: [0x11u8; 32],
            vout: 1,
            script_pubkey: p2wpkh_script(0x22),
            amount: 50_000,
            sequence: DEFAULT_SEQUENCE,
        }
    }

    #[test]
    fn encodes_varints() {
        assert_eq!(encode_varint(0xfc), vec![0xfc]);
        assert_eq!(encode_varint(0xfd), vec![0xfd, 0xfd, 0x00]);
        assert_eq!(encode_varint(0xffff), vec![0xfd, 0xff, 0xff]);
        assert_eq!(encode_varint(0x1_0000), vec![0xfe, 0x00, 0x00, 0x01, 0x00]);
    }

    #[test]
    fn serializes_unsigned_tx_without_witness_data() {
        let input = sample_input();
        let outputs = vec![TxOutput {
            script_pubkey: p2wpkh_script(0x22),
            amount: 50_000,
        }];

        let unsigned = serialize_unsigned_tx(&[input.clone()], &outputs);

        // PSBT unsigned tx must match the legacy (txid) serialization.
        let txid_view = serialize_tx_for_txid(&[input], &outputs);
        assert_eq!(unsigned, txid_view);

        // After version, the next byte should be the input count (not a segwit marker).
        assert_eq!(unsigned[4], 0x01);

        let locktime_bytes = &unsigned[unsigned.len() - 4..];
        assert_eq!(locktime_bytes, [0x00, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn serializes_txid_view() {
        let input = sample_input();
        let outputs = vec![TxOutput {
            script_pubkey: p2wpkh_script(0x22),
            amount: 50_000,
        }];

        let raw = serialize_tx_for_txid(&[input], &outputs);
        assert_eq!(
            hex::encode(raw),
            "020000000111111111111111111111111111111111111111111111111111111111111111110100000000fdffffff0150c3000000000000160014222222222222222222222222222222222222222200000000"
        );
    }

    #[test]
    fn real_mainnet_txid_matches_block_explorer_reference() {
        // Raw mainnet transaction (5 inputs, 1 output) fetched from mempool.space.
        let raw_hex = concat!(
            "02000000000105d6f361f6bb3e9be7ee5d91dd16d280dd10edb05541c59360753a4c6566bc32e7",
            "0100000000ffffffff",
            "1c824b5e5fa4191bff2586dc583f29281af5c7a67d42e497da79e282dc5b2028",
            "0100000000ffffffff",
            "adb61c0611dfbdb774de332a4defac9cf92d82dda8a7442fe5cc99dccfbdb026",
            "0100000000ffffffff",
            "c017ef9a2d7eb7a8cf55138563b96b4ce5b7f65b6d135ac8c8bf029e23f82579",
            "0100000000ffffffff",
            "434ceb8cb8ada7b29307f79aa67fa5dbd56cee1fa9ef33c5162a136eadabcace",
            "0100000000ffffffff",
            "015c5d000000000000160014dc51f2e07673595bef9d717f6641501705b5f4a2",
            "0140d6314d9e9261526a871138b5112d992d79f2050994e6aadca64a7464be6b",
            "1592f54782c759dfb480b0eec9ae0072b3c7e60ed4fd9ae2eb6537d879580ef7",
            "13d90140c272a54fd626f1a9b9b1ef3bc777194b91e1bf63f389211d4db3ce0f",
            "89a2d0a58e2b334b8f40ea6bbcae3e105ec262c7aa27ad6d678929a0f8e15d1e",
            "9e4d48d9014069fcdd5073934c90a7c72d6360899147f3900a69a8480f75d3a0",
            "fcdfb5cce44dca815023a1dcc3e013cf70913ca58de607d9ee878dc08523cd17",
            "90c2c86299660140cf1ff8e602796969f0ba6b5645fe55f9514b87da2805643e",
            "48ab7d9202da3108d9b09bf7b6cae17ee70d10ca8e8f7f48875df015bbc98c0f",
            "742e1dc0e7b9613c01406fdbca37dca7e2ca1f9361d4e790b18d9c0c6799b2e2",
            "b6c68493bdaa2cd5784d820bf8be8873ffa021fc88f36fb26e0d62f9c6812126",
            "593f6a8101de9afebc7800000000"
        );
        let raw = hex::decode(raw_hex).expect("valid hex");
        let tx: Transaction = consensus::deserialize(&raw).expect("transaction parses");
        let expected_txid = "31ec8643f0fd9ccd34dca9af5575a54c9ef77bf2cb6ddf776881dbb6e936cf51";
        assert_eq!(tx.compute_txid().to_string(), expected_txid);

        let inputs: Vec<TxInput> = tx
            .input
            .iter()
            .map(|i| TxInput {
                txid: i.previous_output.txid.to_byte_array(),
                vout: i.previous_output.vout,
                script_pubkey: Vec::new(),
                amount: 0,
                sequence: i.sequence.to_consensus_u32(),
            })
            .collect();

        let outputs: Vec<TxOutput> = tx
            .output
            .iter()
            .map(|o| TxOutput {
                script_pubkey: o.script_pubkey.as_bytes().to_vec(),
                amount: o.value.to_sat(),
            })
            .collect();

        let ours = serialize_tx_for_txid(&inputs, &outputs);
        let ours_txid = sha256d::Hash::hash(&ours);

        assert_eq!(ours_txid, tx.compute_txid().to_raw_hash());
        assert_eq!(ours_txid.to_string(), expected_txid);
    }

    #[test]
    fn builds_op_return_script() {
        let script = create_op_return_script(&[0xaa, 0xbb]);
        assert_eq!(script, vec![0x6a, 0x02, 0xaa, 0xbb]);
    }

    #[test]
    fn splits_prefix_and_suffix_for_nonce() {
        let input = sample_input();
        let outputs_before = vec![TxOutput {
            script_pubkey: p2wpkh_script(0x22),
            amount: 25_000,
        }];
        let outputs_after = vec![TxOutput {
            script_pubkey: p2wpkh_script(0x33),
            amount: 24_000,
        }];

        let nonce_bytes = vec![0xde, 0xad, 0xbe, 0xef];
        let op_return_script = create_op_return_script(&nonce_bytes);

        let full_outputs = {
            let mut list = outputs_before.clone();
            list.push(TxOutput {
                script_pubkey: op_return_script,
                amount: 0,
            });
            list.extend(outputs_after.clone());
            list
        };

        let full_tx = serialize_tx_for_txid(&[input.clone()], &full_outputs);
        let (prefix, suffix) =
            split_tx_for_mining(&[input], &outputs_before, &outputs_after, nonce_bytes.len())
                .unwrap();

        let mut rebuilt = prefix;
        rebuilt.extend_from_slice(&nonce_bytes);
        rebuilt.extend_from_slice(&suffix);

        assert_eq!(rebuilt, full_tx);
    }

    #[test]
    fn matches_bitcoin_crate_txid_for_known_tx() {
        let input = sample_input();
        let outputs = vec![TxOutput {
            script_pubkey: p2wpkh_script(0x33),
            amount: 50_000,
        }];

        // Our serialization (txid view, no witnesses).
        let ours = serialize_tx_for_txid(&[input.clone()], &outputs);
        let our_txid = double_sha256(&ours);

        // Build the same transaction with the bitcoin crate to cross-check.
        let btc_tx = Transaction {
            version: Version::TWO,
            lock_time: LockTime::ZERO,
            input: vec![TxIn {
                previous_output: bitcoin::OutPoint {
                    txid: Txid::from_raw_hash(bitcoin::hashes::sha256d::Hash::from_byte_array(
                        input.txid,
                    )),
                    vout: input.vout,
                },
                script_sig: bitcoin::ScriptBuf::new(),
                sequence: bitcoin::Sequence::ENABLE_RBF_NO_LOCKTIME,
                witness: bitcoin::Witness::new(),
            }],
            output: outputs
                .iter()
                .map(|o| TxOut {
                    value: Amount::from_sat(o.amount),
                    script_pubkey: bitcoin::ScriptBuf::from_bytes(o.script_pubkey.clone()),
                })
                .collect(),
        };

        let btc_serialized = consensus::encode::serialize(&btc_tx);
        assert_eq!(ours, btc_serialized);

        let btc_txid = btc_tx.compute_txid().to_raw_hash().to_byte_array();
        assert_eq!(our_txid, btc_txid);
    }

    #[test]
    fn builds_zeld_distribution_op_return() {
        // Example from todo.md: [600, 300, 100] with nonce 42
        let script = create_zeld_distribution_op_return(&[600, 300, 100], 42);

        // Expected structure:
        // 6a           - OP_RETURN
        // 0f           - push 15 bytes
        // 5a 45 4c 44  - "ZELD"
        // 84           - CBOR array of 4 items
        // 19 02 58     - 600
        // 19 01 2c     - 300
        // 18 64        - 100
        // 18 2a        - 42
        //
        // Payload = 4 (ZELD) + 1 (array header) + 3 + 3 + 2 + 2 = 15 bytes

        assert_eq!(script[0], 0x6a); // OP_RETURN
        assert_eq!(script[1], 15); // push 15 bytes
        assert_eq!(&script[2..6], b"ZELD"); // ZELD prefix

        // CBOR array
        assert_eq!(script[6], 0x84); // array of 4
        assert_eq!(&script[7..10], &[0x19, 0x02, 0x58]); // 600
        assert_eq!(&script[10..13], &[0x19, 0x01, 0x2c]); // 300
        assert_eq!(&script[13..15], &[0x18, 0x64]); // 100
        assert_eq!(&script[15..17], &[0x18, 0x2a]); // 42

        // Full length: 2 (script header) + 15 (payload) = 17
        assert_eq!(script.len(), 17);
    }

    #[test]
    fn zeld_distribution_op_return_with_small_values() {
        // Small values (0-23) use 1-byte CBOR encoding
        let script = create_zeld_distribution_op_return(&[10, 5, 1], 0);

        assert_eq!(script[0], 0x6a); // OP_RETURN
        assert_eq!(&script[2..6], b"ZELD");
        assert_eq!(script[6], 0x84); // array of 4
        assert_eq!(script[7], 0x0a); // 10
        assert_eq!(script[8], 0x05); // 5
        assert_eq!(script[9], 0x01); // 1
        assert_eq!(script[10], 0x00); // 0 (nonce)
    }

    #[test]
    fn zeld_distribution_payload_length_calculation() {
        let dist = [600u64, 300, 100];
        let payload_len = zeld_distribution_payload_length(&dist);

        // 4 (ZELD) + 1 (array header for 4 items) + 3+3+2 (distribution) + 9 (max nonce)
        // = 4 + 1 + 8 + 9 = 22
        assert_eq!(payload_len, 22);

        // With specific nonce length
        let with_nonce = zeld_distribution_payload_length_with_nonce(&dist, 2); // 2-byte CBOR nonce
                                                                                // 4 + 1 + 8 + 2 = 15
        assert_eq!(with_nonce, 15);
    }

    #[test]
    fn splits_tx_for_distribution_mining_correctly() {
        let input = sample_input();
        let outputs_before = vec![TxOutput {
            script_pubkey: p2wpkh_script(0x22),
            amount: 25_000,
        }];
        let outputs_after = vec![TxOutput {
            script_pubkey: p2wpkh_script(0x33),
            amount: 24_000,
        }];

        let distribution = [600u64, 300, 100];
        let nonce: u64 = 42;
        let cbor_nonce = encode_cbor_uint(nonce);
        let cbor_nonce_len = cbor_nonce.len();

        // Build the expected full transaction
        let op_return_script = create_zeld_distribution_op_return(&distribution, nonce);
        let full_outputs = {
            let mut list = outputs_before.clone();
            list.push(TxOutput {
                script_pubkey: op_return_script,
                amount: 0,
            });
            list.extend(outputs_after.clone());
            list
        };
        let full_tx = serialize_tx_for_txid(&[input.clone()], &full_outputs);

        // Split and reconstruct
        let (prefix, suffix) = split_tx_for_distribution_mining(
            &[input],
            &outputs_before,
            &outputs_after,
            &distribution,
            cbor_nonce_len,
        )
        .unwrap();

        let mut rebuilt = prefix;
        rebuilt.extend_from_slice(&cbor_nonce);
        rebuilt.extend_from_slice(&suffix);

        assert_eq!(rebuilt, full_tx);
    }

    #[test]
    fn splits_tx_for_distribution_with_large_nonce() {
        let input = sample_input();
        let outputs_before = vec![TxOutput {
            script_pubkey: p2wpkh_script(0x22),
            amount: 25_000,
        }];
        let outputs_after = vec![TxOutput {
            script_pubkey: p2wpkh_script(0x33),
            amount: 24_000,
        }];

        let distribution = [1000u64, 500];
        let nonce: u64 = 1_000_000; // Requires 5-byte CBOR encoding
        let cbor_nonce = encode_cbor_uint(nonce);
        assert_eq!(cbor_nonce.len(), 5); // 0x1a + u32

        let op_return_script = create_zeld_distribution_op_return(&distribution, nonce);
        let full_outputs = {
            let mut list = outputs_before.clone();
            list.push(TxOutput {
                script_pubkey: op_return_script,
                amount: 0,
            });
            list.extend(outputs_after.clone());
            list
        };
        let full_tx = serialize_tx_for_txid(&[input.clone()], &full_outputs);

        let (prefix, suffix) = split_tx_for_distribution_mining(
            &[input],
            &outputs_before,
            &outputs_after,
            &distribution,
            cbor_nonce.len(),
        )
        .unwrap();

        let mut rebuilt = prefix;
        rebuilt.extend_from_slice(&cbor_nonce);
        rebuilt.extend_from_slice(&suffix);

        assert_eq!(rebuilt, full_tx);
    }

    #[test]
    fn distribution_prefix_contains_zeld_and_cbor_header() {
        let input = sample_input();
        let outputs_before = vec![TxOutput {
            script_pubkey: p2wpkh_script(0x22),
            amount: 25_000,
        }];
        let outputs_after = vec![];
        let distribution = [100u64, 50];

        let (prefix, _) = split_tx_for_distribution_mining(
            &[input],
            &outputs_before,
            &outputs_after,
            &distribution,
            cbor_uint_length(0), // 1-byte nonce
        )
        .unwrap();

        // Check that prefix ends with ZELD + CBOR array header + distribution values
        // Find "ZELD" in prefix
        let zeld_pos = prefix
            .windows(4)
            .position(|w| w == ZELD_PREFIX)
            .expect("ZELD prefix should be in prefix");

        // After ZELD, we should have CBOR array header
        let cbor_start = zeld_pos + 4;
        assert_eq!(prefix[cbor_start], 0x83); // array of 3 (2 distribution + 1 nonce)

        // Then distribution values
        assert_eq!(&prefix[cbor_start + 1..cbor_start + 3], &[0x18, 0x64]); // 100
        assert_eq!(&prefix[cbor_start + 3..cbor_start + 5], &[0x18, 0x32]); // 50
    }

    #[test]
    fn rejects_invalid_cbor_nonce_length() {
        let input = sample_input();
        let outputs_before = vec![TxOutput {
            script_pubkey: p2wpkh_script(0x22),
            amount: 25_000,
        }];
        let outputs_after = vec![];
        let distribution = [100u64, 50];

        let err = split_tx_for_distribution_mining(
            &[input],
            &outputs_before,
            &outputs_after,
            &distribution,
            4, // invalid length for CBOR uint
        )
        .unwrap_err();

        assert_eq!(err, TxError::InvalidCborNonceLength);
    }
}
