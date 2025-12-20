use alloc::vec::Vec;

use bech32::segwit;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Network {
    Mainnet,
    Testnet,
    Regtest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressType {
    P2WPKH,
    P2TR,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedAddress {
    pub network: Network,
    pub address_type: AddressType,
    pub witness_program: Vec<u8>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AddressError {
    #[error("invalid bech32/bech32m address")]
    InvalidBech32,
    #[error("unsupported witness version: {0}")]
    UnsupportedWitnessVersion(u8),
    #[error("invalid witness program length")]
    InvalidProgramLength,
    #[error("unsupported address type")]
    UnsupportedAddressType,
    #[error("network mismatch")]
    NetworkMismatch,
}

/// Parse a bech32/bech32m SegWit address (P2WPKH or P2TR).
///
/// If the HRP is `tb`, the network is treated as Testnet (covers former Signet usage).
pub fn parse_address(addr: &str) -> Result<ParsedAddress, AddressError> {
    parse_address_for_network(addr, None)
}

/// Parse a SegWit address while optionally constraining the expected network.
pub fn parse_address_for_network(
    addr: &str,
    expected: Option<Network>,
) -> Result<ParsedAddress, AddressError> {
    let (hrp_value, version_fe, program) =
        segwit::decode(addr).map_err(|_| AddressError::InvalidBech32)?;
    let version = version_fe.to_u8();

    let network = match hrp_value.as_str() {
        "bc" => Network::Mainnet,
        "tb" => Network::Testnet,
        "bcrt" => Network::Regtest,
        _ => return Err(AddressError::NetworkMismatch),
    };

    if let Some(expected_net) = expected {
        if expected_net != network {
            return Err(AddressError::NetworkMismatch);
        }
    }

    let address_type = match version {
        0 => match program.len() {
            20 => AddressType::P2WPKH,
            _ => return Err(AddressError::InvalidProgramLength),
        },
        1 => match program.len() {
            32 => AddressType::P2TR,
            _ => return Err(AddressError::InvalidProgramLength),
        },
        v => return Err(AddressError::UnsupportedWitnessVersion(v)),
    };

    Ok(ParsedAddress {
        network,
        address_type,
        witness_program: program,
    })
}

/// Convert a parsed address to its scriptPubKey representation.
pub fn address_to_script_pubkey(parsed: &ParsedAddress) -> Vec<u8> {
    let mut script = Vec::with_capacity(2 + parsed.witness_program.len());
    match parsed.address_type {
        AddressType::P2WPKH => script.push(0x00), // OP_0
        AddressType::P2TR => script.push(0x51),   // OP_1
    }
    script.push(parsed.witness_program.len() as u8);
    script.extend_from_slice(&parsed.witness_program);
    script
}

#[cfg(test)]
mod tests {
    use super::*;
    use bech32::{hrp, segwit};

    #[test]
    fn parses_mainnet_p2wpkh() {
        let program = vec![0u8; 20];
        let addr = segwit::encode(hrp::BC, segwit::VERSION_0, &program).unwrap();
        let parsed = parse_address(&addr).unwrap();
        assert_eq!(parsed.network, Network::Mainnet);
        assert_eq!(parsed.address_type, AddressType::P2WPKH);
        assert_eq!(parsed.witness_program, program);
    }

    #[test]
    fn parses_mainnet_p2tr() {
        let program = vec![0x55u8; 32];
        let addr = segwit::encode(hrp::BC, segwit::VERSION_1, &program).unwrap();
        let parsed = parse_address(&addr).unwrap();
        assert_eq!(parsed.network, Network::Mainnet);
        assert_eq!(parsed.address_type, AddressType::P2TR);
        assert_eq!(parsed.witness_program, program);
    }

    #[test]
    fn parses_testnet_address() {
        let program = vec![0x11u8; 20];
        let addr = segwit::encode(hrp::TB, segwit::VERSION_0, &program).unwrap();
        let parsed = parse_address(&addr).unwrap();
        assert_eq!(parsed.network, Network::Testnet);
        assert_eq!(parsed.address_type, AddressType::P2WPKH);
    }

    #[test]
    fn parses_testnet_p2tr() {
        let program = vec![0x77u8; 32];
        let addr = segwit::encode(hrp::TB, segwit::VERSION_1, &program).unwrap();
        let parsed = parse_address(&addr).unwrap();
        assert_eq!(parsed.network, Network::Testnet);
        assert_eq!(parsed.address_type, AddressType::P2TR);
        assert_eq!(parsed.witness_program, program);
    }

    #[test]
    fn rejects_wrong_expected_network() {
        let program = vec![0x22u8; 20];
        let addr = segwit::encode(hrp::BC, segwit::VERSION_0, &program).unwrap();
        let err = parse_address_for_network(&addr, Some(Network::Testnet)).unwrap_err();
        assert_eq!(err, AddressError::NetworkMismatch);
    }

    #[test]
    fn rejects_legacy_addresses() {
        assert_eq!(
            parse_address("1BoatSLRHtKNngkdXEeobR76b53LETtpyT").unwrap_err(),
            AddressError::InvalidBech32
        );
    }

    #[test]
    fn rejects_invalid_checksum() {
        let bad = "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kygt08q";
        assert_eq!(parse_address(bad).unwrap_err(), AddressError::InvalidBech32);
    }

    #[test]
    fn builds_script_pubkeys() {
        let program = vec![0u8; 20];
        let parsed = ParsedAddress {
            network: Network::Mainnet,
            address_type: AddressType::P2WPKH,
            witness_program: program.clone(),
        };
        let spk = address_to_script_pubkey(&parsed);
        assert_eq!(spk[0], 0x00);
        assert_eq!(spk[1] as usize, program.len());
        assert_eq!(&spk[2..], program.as_slice());
    }
}
