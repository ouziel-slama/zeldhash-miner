use alloc::{vec, vec::Vec};

/// Encode a nonce as minimal big-endian bytes.
/// Returns a single zero byte for nonce 0.
pub fn encode_nonce(nonce: u64) -> Vec<u8> {
    if nonce == 0 {
        return vec![0x00];
    }

    let len = nonce_byte_length(nonce);
    let mut out = vec![0u8; len];
    for (i, byte) in out.iter_mut().enumerate().take(len) {
        let shift = (len - 1 - i) * 8;
        *byte = (nonce >> shift) as u8;
    }
    out
}

/// Return the number of bytes required to encode `nonce` in minimal big-endian form.
pub fn nonce_byte_length(nonce: u64) -> usize {
    if nonce == 0 {
        return 1;
    }

    let leading = nonce.leading_zeros() as usize;
    ((64 - leading) + 7) / 8
}

#[cfg(test)]
mod tests {
    use super::{encode_nonce, nonce_byte_length};

    #[test]
    fn encodes_minimal_nonce_bytes() {
        assert_eq!(encode_nonce(0), vec![0x00]);
        assert_eq!(encode_nonce(255), vec![0xff]);
        assert_eq!(encode_nonce(256), vec![0x01, 0x00]);
        assert_eq!(encode_nonce(65_536), vec![0x01, 0x00, 0x00]);
        assert_eq!(
            encode_nonce(u64::MAX),
            vec![0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]
        );
    }

    #[test]
    fn reports_correct_length() {
        assert_eq!(nonce_byte_length(0), 1);
        assert_eq!(nonce_byte_length(255), 1);
        assert_eq!(nonce_byte_length(256), 2);
        assert_eq!(nonce_byte_length(65_536), 3);
        assert_eq!(nonce_byte_length(u64::MAX), 8);
    }
}
