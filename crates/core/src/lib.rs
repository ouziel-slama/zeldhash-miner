#![cfg_attr(not(feature = "std"), no_std)]

pub mod address;
pub mod cbor;
pub mod error;
pub mod fees;
pub mod hash;
pub mod miner;
pub mod nonce;
pub mod psbt;
pub mod tx;

#[allow(unused_extern_crates)]
extern crate alloc;

pub use address::{
    address_to_script_pubkey, parse_address, parse_address_for_network, AddressError, AddressType,
    Network, ParsedAddress,
};
pub use cbor::{
    cbor_array_header_length, cbor_array_length, cbor_nonce_byte_length, cbor_uint_length,
    encode_cbor_array, encode_cbor_array_header, encode_cbor_uint,
};
pub use error::{Result, ZeldError};
pub use fees::{
    calculate_change, calculate_fee, calculate_vsize, calculate_vsize_with_nonce_len, FeeError,
};
pub use hash::{count_leading_zeros, double_sha256, hash_meets_target};
pub use miner::{
    build_mining_template, build_psbt_from_plan, mine_batch, mine_batch_with_cbor, mine_range,
    nonce_len_for_range, plan_transaction, split_nonce_segments, split_nonce_segments_cbor,
    txid_to_hex, MineResult, MinerError, MinerResult, MiningTemplate, NonceSegment, OutputRequest,
    TransactionPlan,
};
pub use nonce::{encode_nonce, nonce_byte_length};
pub use psbt::{create_psbt, psbt_to_base64, PsbtError};
pub use tx::{
    create_op_return_script, create_zeld_distribution_op_return, encode_varint,
    serialize_tx_for_txid, serialize_unsigned_tx, split_tx_for_distribution_mining,
    split_tx_for_mining, TxError, TxInput, TxOutput,
};
