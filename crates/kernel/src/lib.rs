#![cfg_attr(target_arch = "spirv", no_std)]
#![allow(clippy::too_many_arguments)]

use spirv_std::{glam::UVec3, memory::Scope, spirv};

const MAX_RESULTS: usize = 8;

#[repr(C)]
pub struct MiningParams {
    pub start_nonce_lo: u32,
    pub start_nonce_hi: u32,
    pub batch_size: u32,
    pub target_zeros: u32,
    pub prefix_len: u32,
    pub suffix_len: u32,
    pub nonce_len: u32,
    pub _pad: u32,
    pub _pad2: u32,
    pub _pad3: u32,
    pub _pad4: u32,
    pub _pad5: u32,
}

#[repr(C)]
pub struct ResultEntry {
    pub nonce_lo: u32,
    pub nonce_hi: u32,
    pub txid: [u32; 8],
    // Match WGSL/storage layout: pad to 48 bytes per entry.
    pub _tail_pad: [u32; 2],
}

#[repr(C, align(16))]
pub struct ResultBuffer {
    pub found_count: u32,
    pub _pad: u32,
    // Keep 16-byte header alignment consistent with WGSL.
    pub _align_pad: [u32; 2],
    pub results: [ResultEntry; MAX_RESULTS],
    // Tail padding keeps the buffer size a multiple of 16 bytes.
    pub _tail_pad: [u32; 2],
    pub _final_pad: [u32; 2],
}

// Layout checks to keep the SPIR-V kernel in sync with the host/WGSL structs.
const _: [(); 48] = [(); core::mem::size_of::<ResultEntry>()];
const _: [(); 416] = [(); core::mem::size_of::<ResultBuffer>()];

#[inline]
fn rotr(x: u32, n: u32) -> u32 {
    (x >> n) | (x << (32 - n))
}

#[inline]
fn bsig0(x: u32) -> u32 {
    rotr(x, 2) ^ rotr(x, 13) ^ rotr(x, 22)
}

#[inline]
fn bsig1(x: u32) -> u32 {
    rotr(x, 6) ^ rotr(x, 11) ^ rotr(x, 25)
}

#[inline]
fn ssig0(x: u32) -> u32 {
    rotr(x, 7) ^ rotr(x, 18) ^ (x >> 3)
}

#[inline]
fn ssig1(x: u32) -> u32 {
    rotr(x, 17) ^ rotr(x, 19) ^ (x >> 10)
}

#[inline]
fn ch(x: u32, y: u32, z: u32) -> u32 {
    (x & y) ^ (!x & z)
}

#[inline]
fn maj(x: u32, y: u32, z: u32) -> u32 {
    (x & y) ^ (x & z) ^ (y & z)
}

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

const INIT: [u32; 8] = [
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
];

#[inline]
fn nonce_bytes(nonce_hi: u32, nonce_lo: u32, out: &mut [u8; 8]) -> usize {
    // big-endian minimal
    let mut full = ((nonce_hi as u64) << 32) | nonce_lo as u64;
    if full == 0 {
        out[7] = 0;
        return 1;
    }
    let mut len = 0;
    while full != 0 {
        len += 1;
        full >>= 8;
    }
    full = ((nonce_hi as u64) << 32) | nonce_lo as u64;
    let mut idx = 0;
    while idx < len {
        let shift = (len - 1 - idx) * 8;
        out[idx] = (full >> shift) as u8;
        idx += 1;
    }
    len
}

#[inline]
fn read_byte(words: &[u32], idx: u32) -> u8 {
    let w = idx / 4;
    let s = (idx % 4) * 8;
    ((words[w as usize] >> s) & 0xff) as u8
}

#[inline]
fn message_byte(
    idx: u32,
    params: &MiningParams,
    prefix: &[u32],
    suffix: &[u32],
    nonce: &[u8; 8],
    nonce_len: u32,
) -> u8 {
    if idx < params.prefix_len {
        return read_byte(prefix, idx);
    }
    let after_prefix = idx - params.prefix_len;
    if after_prefix < nonce_len {
        return nonce[after_prefix as usize];
    }
    read_byte(suffix, after_prefix - nonce_len)
}

#[inline]
fn padded_len(msg_len: u32) -> u32 {
    let base = msg_len + 9;
    let rem = base % 64;
    if rem == 0 {
        base
    } else {
        base + (64 - rem)
    }
}

#[inline]
fn padded_byte(
    idx: u32,
    msg_len: u32,
    total_len: u32,
    params: &MiningParams,
    prefix: &[u32],
    suffix: &[u32],
    nonce: &[u8; 8],
    nonce_len: u32,
) -> u8 {
    if idx < msg_len {
        return message_byte(idx, params, prefix, suffix, nonce, nonce_len);
    }
    if idx == msg_len {
        return 0x80;
    }
    if idx >= total_len - 8 {
        let bit_len = msg_len * 8;
        let shift = (total_len - 1 - idx) * 8;
        return ((bit_len >> shift) & 0xff) as u8;
    }
    0
}

#[inline]
fn sha256_block(
    w: &mut [u32; 64],
    h: &mut [u32; 8],
    params: &MiningParams,
    prefix: &[u32],
    suffix: &[u32],
    nonce: &[u8; 8],
    nonce_len: u32,
    msg_len: u32,
    offset: u32,
    total_len: u32,
) {
    for i in 0..16 {
        let base = offset + i * 4;
        let b0 = padded_byte(
            base, msg_len, total_len, params, prefix, suffix, nonce, nonce_len,
        );
        let b1 = padded_byte(
            base + 1,
            msg_len,
            total_len,
            params,
            prefix,
            suffix,
            nonce,
            nonce_len,
        );
        let b2 = padded_byte(
            base + 2,
            msg_len,
            total_len,
            params,
            prefix,
            suffix,
            nonce,
            nonce_len,
        );
        let b3 = padded_byte(
            base + 3,
            msg_len,
            total_len,
            params,
            prefix,
            suffix,
            nonce,
            nonce_len,
        );
        w[i as usize] =
            ((b0 as u32) << 24) | ((b1 as u32) << 16) | ((b2 as u32) << 8) | (b3 as u32);
    }
    for i in 16..64 {
        let s1 = ssig1(w[i - 2]);
        let s0 = ssig0(w[i - 15]);
        w[i] = w[i - 7]
            .wrapping_add(w[i - 16])
            .wrapping_add(s0)
            .wrapping_add(s1);
    }

    let mut a = h[0];
    let mut b = h[1];
    let mut c = h[2];
    let mut d = h[3];
    let mut e = h[4];
    let mut f = h[5];
    let mut g = h[6];
    let mut hh = h[7];

    for i in 0..64 {
        let t1 = hh
            .wrapping_add(bsig1(e))
            .wrapping_add(ch(e, f, g))
            .wrapping_add(K[i])
            .wrapping_add(w[i]);
        let t2 = bsig0(a).wrapping_add(maj(a, b, c));
        hh = g;
        g = f;
        f = e;
        e = d.wrapping_add(t1);
        d = c;
        c = b;
        b = a;
        a = t1.wrapping_add(t2);
    }

    h[0] = h[0].wrapping_add(a);
    h[1] = h[1].wrapping_add(b);
    h[2] = h[2].wrapping_add(c);
    h[3] = h[3].wrapping_add(d);
    h[4] = h[4].wrapping_add(e);
    h[5] = h[5].wrapping_add(f);
    h[6] = h[6].wrapping_add(g);
    h[7] = h[7].wrapping_add(hh);
}

#[inline]
fn sha256(
    params: &MiningParams,
    prefix: &[u32],
    suffix: &[u32],
    nonce: &[u8; 8],
    nonce_len: u32,
    out: &mut [u32; 8],
) {
    let msg_len = params.prefix_len + nonce_len + params.suffix_len;
    let total_len = padded_len(msg_len);
    let mut h = INIT;
    let mut w = [0u32; 64];

    let mut offset = 0;
    while offset < total_len {
        sha256_block(
            &mut w, &mut h, params, prefix, suffix, nonce, nonce_len, msg_len, offset, total_len,
        );
        offset += 64;
    }
    *out = h;
}

#[inline]
fn sha256_digest_32(input: &[u8; 32], out: &mut [u32; 8]) {
    // Fixed-size SHA256 over a 32-byte buffer (used for the second pass).
    let mut w = [0u32; 64];

    // First 32 bytes of the message.
    let mut i = 0;
    while i < 8 {
        let base = i * 4;
        w[i] = ((input[base] as u32) << 24)
            | ((input[base + 1] as u32) << 16)
            | ((input[base + 2] as u32) << 8)
            | (input[base + 3] as u32);
        i += 1;
    }

    // Padding: 0x80 followed by zeros, then 256-bit length.
    w[8] = 0x8000_0000;
    w[9] = 0;
    w[10] = 0;
    w[11] = 0;
    w[12] = 0;
    w[13] = 0;
    w[14] = 0;
    w[15] = 256; // 32 bytes * 8

    for i in 16..64 {
        let s1 = ssig1(w[i - 2]);
        let s0 = ssig0(w[i - 15]);
        w[i] = w[i - 7]
            .wrapping_add(w[i - 16])
            .wrapping_add(s0)
            .wrapping_add(s1);
    }

    let mut h = INIT;
    let mut a = h[0];
    let mut b = h[1];
    let mut c = h[2];
    let mut d = h[3];
    let mut e = h[4];
    let mut f = h[5];
    let mut g = h[6];
    let mut hh = h[7];

    for i in 0..64 {
        let t1 = hh
            .wrapping_add(bsig1(e))
            .wrapping_add(ch(e, f, g))
            .wrapping_add(K[i])
            .wrapping_add(w[i]);
        let t2 = bsig0(a).wrapping_add(maj(a, b, c));
        hh = g;
        g = f;
        f = e;
        e = d.wrapping_add(t1);
        d = c;
        c = b;
        b = a;
        a = t1.wrapping_add(t2);
    }

    h[0] = h[0].wrapping_add(a);
    h[1] = h[1].wrapping_add(b);
    h[2] = h[2].wrapping_add(c);
    h[3] = h[3].wrapping_add(d);
    h[4] = h[4].wrapping_add(e);
    h[5] = h[5].wrapping_add(f);
    h[6] = h[6].wrapping_add(g);
    h[7] = h[7].wrapping_add(hh);

    *out = h;
}

#[inline]
fn double_sha256(
    params: &MiningParams,
    prefix: &[u32],
    suffix: &[u32],
    nonce: &[u8; 8],
    nonce_len: u32,
    out: &mut [u32; 8],
) {
    // First SHA256 pass over the transaction bytes.
    let mut first = [0u32; 8];
    sha256(params, prefix, suffix, nonce, nonce_len, &mut first);

    // Convert the first hash to canonical big-endian bytes.
    let mut first_bytes = [0u8; 32];
    let mut idx = 0;
    while idx < 8 {
        let word = first[idx];
        let base = idx * 4;
        first_bytes[base] = (word >> 24) as u8;
        first_bytes[base + 1] = (word >> 16) as u8;
        first_bytes[base + 2] = (word >> 8) as u8;
        first_bytes[base + 3] = word as u8;
        idx += 1;
    }

    // Second SHA256 pass directly over the 32-byte digest.
    sha256_digest_32(&first_bytes, out);
}

#[inline]
fn leading_zeros_be_words(hash: &[u32; 8]) -> u32 {
    // Convert to txid byte view (little-endian) then count leading hex zeros.
    let mut bytes = [0u8; 32];
    let mut idx = 0;
    while idx < 8 {
        let word = hash[idx];
        let base = idx * 4;
        bytes[base] = (word >> 24) as u8;
        bytes[base + 1] = (word >> 16) as u8;
        bytes[base + 2] = (word >> 8) as u8;
        bytes[base + 3] = word as u8;
        idx += 1;
    }

    let mut zeros = 0;
    let mut idx = 32;
    while idx > 0 {
        idx -= 1;
        let byte = bytes[idx];
        if byte == 0 {
            zeros += 2;
            continue;
        }
        if (byte >> 4) == 0 {
            zeros += 1;
        }
        break;
    }
    zeros
}

#[spirv(compute(threads(256)))]
pub fn main(
    #[spirv(global_invocation_id)] gid: UVec3,
    #[spirv(storage_buffer, descriptor_set = 0, binding = 0)] prefix: &[u32],
    #[spirv(storage_buffer, descriptor_set = 0, binding = 1)] suffix: &[u32],
    #[spirv(uniform, descriptor_set = 0, binding = 2)] params: &MiningParams,
    #[spirv(storage_buffer, descriptor_set = 0, binding = 3)] results: &mut ResultBuffer,
) {
    let idx = gid.x;
    if idx >= params.batch_size {
        return;
    }
    let mut nonce_bytes_buf = [0u8; 8];
    let nonce_lo = params.start_nonce_lo.wrapping_add(idx);
    let mut nonce_hi = params.start_nonce_hi;
    if nonce_lo < params.start_nonce_lo {
        nonce_hi = nonce_hi.wrapping_add(1);
    }
    let nonce_len_min = nonce_bytes(nonce_hi, nonce_lo, &mut nonce_bytes_buf) as u32;
    let target_len = params.nonce_len;
    if target_len == 0 || target_len > 8 || nonce_len_min != target_len {
        return;
    }

    let mut hash_words = [0u32; 8];
    double_sha256(
        params,
        prefix,
        suffix,
        &nonce_bytes_buf,
        target_len,
        &mut hash_words,
    );

    if params.target_zeros > 64 {
        return;
    }
    if params.target_zeros == 0 || leading_zeros_be_words(&hash_words) >= params.target_zeros {
        // We need ACQUIRE_RELEASE to coordinate readers of the result buffer and
        // UNIFORM_MEMORY because the buffer lives in StorageBuffer.
        let slot = unsafe {
            // ACQUIRE_RELEASE | UNIFORM_MEMORY (0x8 | 0x40) as required by SPIR-V.
            spirv_std::arch::atomic_i_add::<u32, { Scope::Device as u32 }, { 0x8 | 0x40 }>(
                &mut results.found_count,
                1,
            )
        };
        if (slot as usize) < MAX_RESULTS {
            results.results[slot as usize].nonce_lo = nonce_lo;
            results.results[slot as usize].nonce_hi = nonce_hi;
            results.results[slot as usize].txid = hash_words;
        }
    }
}
