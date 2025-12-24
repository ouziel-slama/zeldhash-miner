//! Minimal CBOR encoding for unsigned integers and arrays.
//!
//! Implements the subset of RFC 8949 (CBOR) needed for ZELD distribution encoding:
//! - Unsigned integers (major type 0)
//! - Arrays (major type 4)

use alloc::{vec, vec::Vec};

/// CBOR major type 0: unsigned integer
const MAJOR_TYPE_UINT: u8 = 0x00;
/// CBOR major type 4: array
const MAJOR_TYPE_ARRAY: u8 = 0x80;

/// Returns the byte length of a CBOR-encoded unsigned integer.
///
/// # Examples
///
/// ```
/// use zeldhash_miner_core::cbor::cbor_uint_length;
///
/// assert_eq!(cbor_uint_length(0), 1);   // 0-23: 1 byte
/// assert_eq!(cbor_uint_length(23), 1);
/// assert_eq!(cbor_uint_length(24), 2);  // 24-255: 2 bytes
/// assert_eq!(cbor_uint_length(255), 2);
/// assert_eq!(cbor_uint_length(256), 3); // 256-65535: 3 bytes
/// assert_eq!(cbor_uint_length(65535), 3);
/// assert_eq!(cbor_uint_length(65536), 5); // 65536-2^32-1: 5 bytes
/// assert_eq!(cbor_uint_length(u32::MAX as u64), 5);
/// assert_eq!(cbor_uint_length(u32::MAX as u64 + 1), 9); // >=2^32: 9 bytes
/// ```
#[inline]
pub fn cbor_uint_length(value: u64) -> usize {
    match value {
        0..=23 => 1,
        24..=255 => 2,
        256..=65535 => 3,
        65536..=0xFFFF_FFFF => 5,
        _ => 9,
    }
}

/// Encode a u64 as a CBOR unsigned integer (major type 0).
///
/// # Examples
///
/// ```
/// use zeldhash_miner_core::cbor::encode_cbor_uint;
///
/// assert_eq!(encode_cbor_uint(0), vec![0x00]);
/// assert_eq!(encode_cbor_uint(23), vec![0x17]);
/// assert_eq!(encode_cbor_uint(24), vec![0x18, 24]);
/// assert_eq!(encode_cbor_uint(255), vec![0x18, 255]);
/// assert_eq!(encode_cbor_uint(256), vec![0x19, 0x01, 0x00]);
/// ```
pub fn encode_cbor_uint(value: u64) -> Vec<u8> {
    encode_cbor_uint_with_major_type(MAJOR_TYPE_UINT, value)
}

/// Internal helper: encode a CBOR value with a given major type.
fn encode_cbor_uint_with_major_type(major_type: u8, value: u64) -> Vec<u8> {
    match value {
        0..=23 => {
            vec![major_type | (value as u8)]
        }
        24..=255 => {
            vec![major_type | 24, value as u8]
        }
        256..=65535 => {
            let mut buf = vec![major_type | 25];
            buf.extend_from_slice(&(value as u16).to_be_bytes());
            buf
        }
        65536..=0xFFFF_FFFF => {
            let mut buf = vec![major_type | 26];
            buf.extend_from_slice(&(value as u32).to_be_bytes());
            buf
        }
        _ => {
            let mut buf = vec![major_type | 27];
            buf.extend_from_slice(&value.to_be_bytes());
            buf
        }
    }
}

/// Encode a CBOR array header for an array of the given length.
///
/// This only encodes the header; elements must be appended separately.
///
/// # Examples
///
/// ```
/// use zeldhash_miner_core::cbor::encode_cbor_array_header;
///
/// assert_eq!(encode_cbor_array_header(0), vec![0x80]);  // empty array
/// assert_eq!(encode_cbor_array_header(4), vec![0x84]);  // 4 elements
/// assert_eq!(encode_cbor_array_header(23), vec![0x97]); // 23 elements
/// assert_eq!(encode_cbor_array_header(24), vec![0x98, 24]); // 24 elements
/// ```
pub fn encode_cbor_array_header(len: usize) -> Vec<u8> {
    encode_cbor_uint_with_major_type(MAJOR_TYPE_ARRAY, len as u64)
}

/// Returns the byte length of a CBOR array header for an array of the given length.
#[inline]
pub fn cbor_array_header_length(len: usize) -> usize {
    cbor_uint_length(len as u64)
}

/// Encode an array of u64 values as a CBOR array.
///
/// # Examples
///
/// ```
/// use zeldhash_miner_core::cbor::encode_cbor_array;
///
/// // Empty array
/// assert_eq!(encode_cbor_array(&[]), vec![0x80]);
///
/// // [600, 300, 100, 42]
/// let encoded = encode_cbor_array(&[600, 300, 100, 42]);
/// assert_eq!(encoded, vec![
///     0x84,                    // array of 4 items
///     0x19, 0x02, 0x58,        // 600
///     0x19, 0x01, 0x2c,        // 300
///     0x18, 0x64,              // 100
///     0x18, 0x2a,              // 42
/// ]);
/// ```
pub fn encode_cbor_array(values: &[u64]) -> Vec<u8> {
    let mut buf = encode_cbor_array_header(values.len());
    for &v in values {
        buf.extend(encode_cbor_uint(v));
    }
    buf
}

/// Calculate the total byte length of a CBOR-encoded array of u64 values.
///
/// This is useful for fee estimation without actually encoding.
pub fn cbor_array_length(values: &[u64]) -> usize {
    cbor_array_header_length(values.len())
        + values.iter().map(|&v| cbor_uint_length(v)).sum::<usize>()
}

/// Byte length of a CBOR-encoded nonce (unsigned integer).
///
/// This mirrors [`cbor_uint_length`] but is provided for clarity when working
/// with nonce segmentation, where CBOR lengths differ from the raw big-endian
/// encoding used elsewhere in the miner.
#[inline]
pub fn cbor_nonce_byte_length(nonce: u64) -> usize {
    cbor_uint_length(nonce)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_cbor_uint_examples() {
        // Examples from todo.md
        assert_eq!(encode_cbor_uint(0), vec![0x00]);
        assert_eq!(encode_cbor_uint(23), vec![0x17]);
        assert_eq!(encode_cbor_uint(24), vec![0x18, 24]);
        assert_eq!(encode_cbor_uint(255), vec![0x18, 255]);
        assert_eq!(encode_cbor_uint(256), vec![0x19, 0x01, 0x00]);
    }

    #[test]
    fn encodes_cbor_uint_boundaries() {
        // 0-23: single byte
        assert_eq!(encode_cbor_uint(0), vec![0x00]);
        assert_eq!(encode_cbor_uint(1), vec![0x01]);
        assert_eq!(encode_cbor_uint(10), vec![0x0a]);
        assert_eq!(encode_cbor_uint(23), vec![0x17]);

        // 24-255: 0x18 + u8
        assert_eq!(encode_cbor_uint(24), vec![0x18, 24]);
        assert_eq!(encode_cbor_uint(100), vec![0x18, 100]);
        assert_eq!(encode_cbor_uint(255), vec![0x18, 255]);

        // 256-65535: 0x19 + u16 BE
        assert_eq!(encode_cbor_uint(256), vec![0x19, 0x01, 0x00]);
        assert_eq!(encode_cbor_uint(1000), vec![0x19, 0x03, 0xe8]);
        assert_eq!(encode_cbor_uint(65535), vec![0x19, 0xff, 0xff]);

        // 65536-2^32-1: 0x1a + u32 BE
        assert_eq!(encode_cbor_uint(65536), vec![0x1a, 0x00, 0x01, 0x00, 0x00]);
        assert_eq!(
            encode_cbor_uint(1_000_000),
            vec![0x1a, 0x00, 0x0f, 0x42, 0x40]
        );
        assert_eq!(
            encode_cbor_uint(0xFFFF_FFFF),
            vec![0x1a, 0xff, 0xff, 0xff, 0xff]
        );

        // >= 2^32: 0x1b + u64 BE
        assert_eq!(
            encode_cbor_uint(0x1_0000_0000),
            vec![0x1b, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00]
        );
        assert_eq!(
            encode_cbor_uint(u64::MAX),
            vec![0x1b, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]
        );
    }

    #[test]
    fn cbor_uint_length_matches_encode() {
        let test_values = [
            0,
            1,
            23,
            24,
            100,
            255,
            256,
            65535,
            65536,
            0xFFFF_FFFF,
            0x1_0000_0000,
            u64::MAX,
        ];
        for v in test_values {
            assert_eq!(
                cbor_uint_length(v),
                encode_cbor_uint(v).len(),
                "mismatch for {v}"
            );
        }
    }

    #[test]
    fn encodes_array_headers() {
        assert_eq!(encode_cbor_array_header(0), vec![0x80]);
        assert_eq!(encode_cbor_array_header(1), vec![0x81]);
        assert_eq!(encode_cbor_array_header(4), vec![0x84]);
        assert_eq!(encode_cbor_array_header(23), vec![0x97]);
        assert_eq!(encode_cbor_array_header(24), vec![0x98, 24]);
        assert_eq!(encode_cbor_array_header(255), vec![0x98, 255]);
        assert_eq!(encode_cbor_array_header(256), vec![0x99, 0x01, 0x00]);
    }

    #[test]
    fn encodes_example_distribution_array() {
        // From todo.md: [600, 300, 100] with nonce 42
        let arr = encode_cbor_array(&[600, 300, 100, 42]);
        assert_eq!(
            arr,
            vec![
                0x84, // array of 4 items
                0x19, 0x02, 0x58, // 600
                0x19, 0x01, 0x2c, // 300
                0x18, 0x64, // 100
                0x18, 0x2a, // 42
            ]
        );
    }

    #[test]
    fn cbor_array_length_matches_encode() {
        let test_arrays: &[&[u64]] = &[
            &[],
            &[0],
            &[23, 24],
            &[600, 300, 100],
            &[600, 300, 100, 42],
            &[u64::MAX, 0, 255, 256],
        ];
        for arr in test_arrays {
            assert_eq!(
                cbor_array_length(arr),
                encode_cbor_array(arr).len(),
                "mismatch for {:?}",
                arr
            );
        }
    }

    #[test]
    fn empty_array() {
        assert_eq!(encode_cbor_array(&[]), vec![0x80]);
        assert_eq!(cbor_array_length(&[]), 1);
    }

    #[test]
    fn cbor_nonce_length_matches_spec() {
        let cases = [
            (0u64, 1usize),
            (23, 1),
            (24, 2),
            (255, 2),
            (256, 3),
            (65_535, 3),
            (65_536, 5),
            (0xFFFF_FFFF, 5),
            (0x1_0000_0000, 9),
            (u64::MAX, 9),
        ];

        for (nonce, expected_len) in cases {
            assert_eq!(
                cbor_nonce_byte_length(nonce),
                expected_len,
                "nonce {nonce} should use {expected_len} CBOR bytes"
            );
        }
    }
}
