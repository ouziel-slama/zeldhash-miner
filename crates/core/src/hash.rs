use sha2::{Digest, Sha256};

/// Compute SHA256(SHA256(data)).
pub fn double_sha256(data: &[u8]) -> [u8; 32] {
    let first = Sha256::digest(data);
    let second = Sha256::digest(first);
    let mut out = [0u8; 32];
    out.copy_from_slice(&second);
    out
}

/// Count leading hex zeros of a txid-style hash.
/// Txids are displayed reversed, so we count from the end of the hash bytes.
pub fn count_leading_zeros(hash: &[u8; 32]) -> u8 {
    let mut zeros = 0u8;
    for &byte in hash.iter().rev() {
        if byte == 0 {
            zeros = zeros.saturating_add(2);
            continue;
        }

        if byte >> 4 == 0 {
            zeros = zeros.saturating_add(1);
        }
        break;
    }
    zeros
}

/// Quick check that a hash meets a target number of leading zeros (txid view).
pub fn hash_meets_target(hash: &[u8; 32], target_zeros: u8) -> bool {
    if target_zeros == 0 {
        return true;
    }
    if target_zeros > 64 {
        return false;
    }

    let full_zero_bytes = (target_zeros / 2) as usize;
    let needs_half_nibble = target_zeros % 2 == 1;

    // Check required full zero bytes from the end.
    if hash.iter().rev().take(full_zero_bytes).any(|b| *b != 0) {
        return false;
    }

    if needs_half_nibble {
        let idx = match hash.len().checked_sub(full_zero_bytes + 1) {
            Some(idx) => idx,
            None => return false,
        };
        let byte = hash[idx];
        return byte >> 4 == 0;
    }

    true
}

#[cfg(test)]
mod tests {
    use super::{count_leading_zeros, double_sha256, hash_meets_target};

    #[test]
    fn double_hash_matches_genesis_block_header() {
        // Raw genesis block header (80 bytes), little-endian fields as mined.
        let header = hex::decode(concat!(
            "01000000",
            "0000000000000000000000000000000000000000000000000000000000000000",
            "3ba3edfd7a7b12b27ac72c3e67768f617fc81bc3888a51323a9fb8aa4b1e5e4a",
            "29ab5f49",
            "ffff001d",
            "1dac2b7c"
        ))
        .unwrap();
        let hash = double_sha256(&header);
        assert_eq!(
            hex::encode(hash),
            "6fe28c0ab6f1b372c1a6a246ae63f74f931e8365e15a089c68d6190000000000"
        );
    }

    #[test]
    fn counts_leading_zeros_correctly() {
        let mut hash = [0xffu8; 32];
        assert_eq!(count_leading_zeros(&hash), 0);

        hash[31] = 0x0f; // 1 leading hex zero (low nibble non-zero)
        assert_eq!(count_leading_zeros(&hash), 1);

        hash[31] = 0x00;
        hash[30] = 0x00;
        hash[29] = 0x00;
        hash[28] = 0x00;
        hash[27] = 0x1f;
        assert_eq!(count_leading_zeros(&hash), 8);

        hash.fill(0);
        assert_eq!(count_leading_zeros(&hash), 64);
    }

    #[test]
    fn target_check_matches_expectations() {
        let mut hash = [0u8; 32];
        // All zeros satisfies any valid target up to 64.
        assert!(hash_meets_target(&hash, 64));

        hash[31] = 0x0f; // one leading zero nibble
        assert!(hash_meets_target(&hash, 1));
        assert!(!hash_meets_target(&hash, 2));

        hash[31] = 0x00;
        hash[30] = 0x00;
        hash[29] = 0x00;
        hash[28] = 0x12;
        // 6 zeros -> needs 3 full zero bytes.
        assert!(hash_meets_target(&hash, 6));
        assert!(!hash_meets_target(&hash, 7));
    }
}
