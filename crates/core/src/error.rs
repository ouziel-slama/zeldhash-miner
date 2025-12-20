use crate::{address::AddressError, fees::FeeError, psbt::PsbtError, tx::TxError};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ZeldError {
    #[error("address error: {0}")]
    Address(#[from] AddressError),
    #[error("fee error: {0}")]
    Fee(#[from] FeeError),
    #[error("transaction error: {0}")]
    Tx(#[from] TxError),
    #[error("psbt error: {0}")]
    Psbt(#[from] PsbtError),
}

pub type Result<T> = core::result::Result<T, ZeldError>;
