use alloc::{string::String, vec::Vec};

use crate::tx::{encode_varint, TxInput, TxOutput};
use base64::Engine;
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PsbtError {
    #[error("invalid psbt construction")]
    InvalidPsbt,
}

pub fn create_psbt(inputs: &[TxInput], outputs: &[TxOutput]) -> Result<Vec<u8>, PsbtError> {
    let mut psbt = Vec::new();
    psbt.extend_from_slice(&[0x70, 0x73, 0x62, 0x74, 0xff]); // magic

    // Global map: PSBT_GLOBAL_UNSIGNED_TX (0x00)
    // Embed the unsigned transaction using legacy (non-witness) serialization per BIP174.
    let unsigned_tx = crate::tx::serialize_unsigned_tx(inputs, outputs);
    psbt.extend(encode_varint(1)); // key length = 1 (just the type)
    psbt.push(0x00); // key type
    psbt.extend(encode_varint(unsigned_tx.len() as u64));
    psbt.extend_from_slice(&unsigned_tx);
    psbt.push(0x00); // separator

    // Input maps: include PSBT_IN_WITNESS_UTXO (0x01)
    for input in inputs {
        let mut witness_utxo = Vec::new();
        witness_utxo.extend_from_slice(&input.amount.to_le_bytes());
        witness_utxo.extend(encode_varint(input.script_pubkey.len() as u64));
        witness_utxo.extend_from_slice(&input.script_pubkey);

        psbt.extend(encode_varint(1)); // key length
        psbt.push(0x01); // PSBT_IN_WITNESS_UTXO
        psbt.extend(encode_varint(witness_utxo.len() as u64));
        psbt.extend_from_slice(&witness_utxo);
        psbt.push(0x00); // separator
    }

    // Output maps: empty (separator only)
    psbt.resize(psbt.len() + outputs.len(), 0x00);

    Ok(psbt)
}

pub fn psbt_to_base64(psbt: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(psbt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::psbt::Psbt;

    fn sample_input_output() -> (TxInput, TxOutput) {
        let mut spk = vec![0x00, 0x14];
        spk.extend(vec![0x44; 20]);
        let input = TxInput {
            txid: [0x33u8; 32],
            vout: 0,
            script_pubkey: spk.clone(),
            amount: 75_000,
            sequence: crate::tx::DEFAULT_SEQUENCE,
        };
        let output = TxOutput {
            script_pubkey: spk,
            amount: 70_000,
        };
        (input, output)
    }

    #[test]
    fn builds_psbt_with_witness_utxo() {
        let (input, output) = sample_input_output();
        let psbt = create_psbt(&[input], &[output]).unwrap();
        assert_eq!(&psbt[..5], b"psbt\xff");
        let b64 = psbt_to_base64(&psbt);
        assert!(!b64.is_empty());
    }

    #[test]
    fn parses_psbt_with_bitcoin_crate() {
        let (input, output) = sample_input_output();
        let psbt_bytes = create_psbt(&[input.clone()], &[output.clone()]).unwrap();
        let parsed = Psbt::deserialize(&psbt_bytes).expect("psbt must round-trip");

        assert_eq!(parsed.unsigned_tx.input.len(), 1);
        assert_eq!(parsed.unsigned_tx.output.len(), 1);

        let witness_utxo = parsed.inputs[0]
            .witness_utxo
            .as_ref()
            .expect("witness utxo present");
        assert_eq!(witness_utxo.value, bitcoin::Amount::from_sat(input.amount));
        assert_eq!(witness_utxo.script_pubkey.as_bytes(), output.script_pubkey);
    }
}
