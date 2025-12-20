// WebGPU compute shader: double-SHA256 over (prefix || nonce || suffix).
// Supports raw big-endian nonces and CBOR-encoded nonces (major type 0).

const WORKGROUP_SIZE: u32 = 256u;
const MAX_RESULTS: u32 = 8u;

struct MiningParams {
    start_nonce_lo: u32,
    start_nonce_hi: u32,
    batch_size: u32,
    target_zeros: u32,
    prefix_len: u32,
    suffix_len: u32,
    nonce_len: u32,
    use_cbor_nonce: u32,
    _pad2: u32,
    _pad3: u32,
    _pad4: u32,
    _pad5: u32,
};

struct ResultEntry {
    nonce_lo: u32,
    nonce_hi: u32,
    txid: array<u32, 8>,
    _tail_pad_0: u32,
    _tail_pad_1: u32,
};

struct ResultBuffer {
    found_count: atomic<u32>,
    _pad: u32,
    _align_pad_0: u32,
    _align_pad_1: u32,
    results: array<ResultEntry, MAX_RESULTS>,
    _tail_pad_0: u32,
    _tail_pad_1: u32,
    _final_pad_0: u32,
    _final_pad_1: u32,
};

@group(0) @binding(0)
var<storage, read> PREFIX: array<u32>;
@group(0) @binding(1)
var<storage, read> SUFFIX: array<u32>;
@group(0) @binding(2)
var<uniform> PARAMS: MiningParams;
@group(0) @binding(3)
var<storage, read_write> RESULTS: ResultBuffer;

fn rotr(x: u32, n: u32) -> u32 {
    return (x >> n) | (x << (32u - n));
}

fn ch(x: u32, y: u32, z: u32) -> u32 {
    return (x & y) ^ (~x & z);
}

fn maj(x: u32, y: u32, z: u32) -> u32 {
    return (x & y) ^ (x & z) ^ (y & z);
}

fn bsig0(x: u32) -> u32 {
    return rotr(x, 2u) ^ rotr(x, 13u) ^ rotr(x, 22u);
}

fn bsig1(x: u32) -> u32 {
    return rotr(x, 6u) ^ rotr(x, 11u) ^ rotr(x, 25u);
}

fn ssig0(x: u32) -> u32 {
    return rotr(x, 7u) ^ rotr(x, 18u) ^ (x >> 3u);
}

fn ssig1(x: u32) -> u32 {
    return rotr(x, 17u) ^ rotr(x, 19u) ^ (x >> 10u);
}

fn raw_nonce_bytes(nonce: vec2<u32>) -> array<u32, 9> {
    return array<u32, 9>(
        0u,
        (nonce.y >> 24u) & 0xffu,
        (nonce.y >> 16u) & 0xffu,
        (nonce.y >> 8u) & 0xffu,
        nonce.y & 0xffu,
        (nonce.x >> 24u) & 0xffu,
        (nonce.x >> 16u) & 0xffu,
        (nonce.x >> 8u) & 0xffu,
        nonce.x & 0xffu,
    );
}

fn raw_nonce_len(nonce: vec2<u32>) -> u32 {
    var bytes = raw_nonce_bytes(nonce);
    for (var i: u32 = 0u; i < 9u; i = i + 1u) {
        if bytes[i] != 0u {
            return 9u - i;
        }
    }
    return 1u;
}

fn cbor_nonce_len(nonce: vec2<u32>) -> u32 {
    if nonce.y != 0u {
        return 9u;
    }
    let lo = nonce.x;
    if lo <= 23u {
        return 1u;
    }
    if lo <= 255u {
        return 2u;
    }
    if lo <= 65535u {
        return 3u;
    }
    if lo <= 0xFFFFFFFFu {
        return 5u;
    }
    return 9u;
}

fn cbor_nonce_bytes(nonce: vec2<u32>, nonce_len_val: u32) -> array<u32, 9> {
    var out = array<u32, 9>(0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u);
    if nonce_len_val == 1u {
        out[8] = nonce.x & 0xffu;
        return out;
    }
    if nonce_len_val == 2u {
        out[7] = 0x18u;
        out[8] = nonce.x & 0xffu;
        return out;
    }
    if nonce_len_val == 3u {
        out[6] = 0x19u;
        out[7] = (nonce.x >> 8u) & 0xffu;
        out[8] = nonce.x & 0xffu;
        return out;
    }
    if nonce_len_val == 5u {
        out[4] = 0x1au;
        out[5] = (nonce.x >> 24u) & 0xffu;
        out[6] = (nonce.x >> 16u) & 0xffu;
        out[7] = (nonce.x >> 8u) & 0xffu;
        out[8] = nonce.x & 0xffu;
        return out;
    }
    // 9-byte encoding
    out[0] = 0x1bu;
    out[1] = (nonce.y >> 24u) & 0xffu;
    out[2] = (nonce.y >> 16u) & 0xffu;
    out[3] = (nonce.y >> 8u) & 0xffu;
    out[4] = nonce.y & 0xffu;
    out[5] = (nonce.x >> 24u) & 0xffu;
    out[6] = (nonce.x >> 16u) & 0xffu;
    out[7] = (nonce.x >> 8u) & 0xffu;
    out[8] = nonce.x & 0xffu;
    return out;
}

fn nonce_bytes(nonce: vec2<u32>, nonce_len_val: u32) -> array<u32, 9> {
    if PARAMS.use_cbor_nonce != 0u {
        return cbor_nonce_bytes(nonce, nonce_len_val);
    }
    return raw_nonce_bytes(nonce);
}

fn read_prefix_byte(idx: u32) -> u32 {
    let word_idx = idx / 4u;
    let shift = (idx % 4u) * 8u;
    return (PREFIX[word_idx] >> shift) & 0xffu;
}

fn read_suffix_byte(idx: u32) -> u32 {
    let word_idx = idx / 4u;
    let shift = (idx % 4u) * 8u;
    return (SUFFIX[word_idx] >> shift) & 0xffu;
}

fn nonce_byte(nonce: vec2<u32>, nonce_len_val: u32, idx: u32) -> u32 {
    var bytes = nonce_bytes(nonce, nonce_len_val);
    return bytes[9u - nonce_len_val + idx];
}

fn message_byte(msg_idx: u32, nonce: vec2<u32>, nonce_len_val: u32) -> u32 {
    if msg_idx < PARAMS.prefix_len {
        return read_prefix_byte(msg_idx);
    }

    let after_prefix = msg_idx - PARAMS.prefix_len;
    if after_prefix < nonce_len_val {
        return nonce_byte(nonce, nonce_len_val, after_prefix);
    }

    return read_suffix_byte(after_prefix - nonce_len_val);
}

fn padded_len(msg_len: u32) -> u32 {
    let base = msg_len + 9u;
    let rem = base % 64u;
    return select(base + (64u - rem), base, rem == 0u);
}

fn length_byte_from_msg_len(tail_idx: u32, msg_len: u32) -> u32 {
    // Encode 64-bit bit-length without shifting by >= 32 (WGSL forbids that).
    let bit_len_lo = msg_len << 3u;     // low 32 bits of msg_len * 8
    let bit_len_hi = msg_len >> 29u;    // high 32 bits of msg_len * 8

    if tail_idx < 4u {
        return (bit_len_lo >> (tail_idx * 8u)) & 0xffu;
    }
    let shift = (tail_idx - 4u) * 8u;
    return (bit_len_hi >> shift) & 0xffu;
}

fn padded_byte(idx: u32, msg_len: u32, total_len: u32, nonce: vec2<u32>, nonce_len_val: u32) -> u32 {
    if idx < msg_len {
        return message_byte(idx, nonce, nonce_len_val);
    }
    if idx == msg_len {
        return 0x80u;
    }
    if idx >= total_len - 8u {
        let tail_idx = total_len - 1u - idx;
        return length_byte_from_msg_len(tail_idx, msg_len);
    }
    return 0u;
}

fn k(i: u32) -> u32 {
    var table: array<u32, 64> = array<u32, 64>(
        0x428a2f98u, 0x71374491u, 0xb5c0fbcfu, 0xe9b5dba5u, 0x3956c25bu, 0x59f111f1u, 0x923f82a4u,
        0xab1c5ed5u, 0xd807aa98u, 0x12835b01u, 0x243185beu, 0x550c7dc3u, 0x72be5d74u, 0x80deb1feu,
        0x9bdc06a7u, 0xc19bf174u, 0xe49b69c1u, 0xefbe4786u, 0x0fc19dc6u, 0x240ca1ccu, 0x2de92c6fu,
        0x4a7484aau, 0x5cb0a9dcu, 0x76f988dau, 0x983e5152u, 0xa831c66du, 0xb00327c8u, 0xbf597fc7u,
        0xc6e00bf3u, 0xd5a79147u, 0x06ca6351u, 0x14292967u, 0x27b70a85u, 0x2e1b2138u, 0x4d2c6dfcu,
        0x53380d13u, 0x650a7354u, 0x766a0abbu, 0x81c2c92eu, 0x92722c85u, 0xa2bfe8a1u, 0xa81a664bu,
        0xc24b8b70u, 0xc76c51a3u, 0xd192e819u, 0xd6990624u, 0xf40e3585u, 0x106aa070u, 0x19a4c116u,
        0x1e376c08u, 0x2748774cu, 0x34b0bcb5u, 0x391c0cb3u, 0x4ed8aa4au, 0x5b9cca4fu, 0x682e6ff3u,
        0x748f82eeu, 0x78a5636fu, 0x84c87814u, 0x8cc70208u, 0x90befffau, 0xa4506cebu, 0xbef9a3f7u,
        0xc67178f2u
    );
    return table[i];
}

fn sha256_first(msg_len: u32, nonce: vec2<u32>, nonce_len_val: u32) -> array<u32, 8> {
    var h: array<u32, 8> = array<u32, 8>(
        0x6a09e667u, 0xbb67ae85u, 0x3c6ef372u, 0xa54ff53au,
        0x510e527fu, 0x9b05688cu, 0x1f83d9abu, 0x5be0cd19u,
    );

    let total_len = padded_len(msg_len);
    var offset: u32 = 0u;
    loop {
        if offset >= total_len {
            break;
        }

        var w: array<u32, 64>;
        for (var i: u32 = 0u; i < 16u; i = i + 1u) {
            let b0 = padded_byte(offset + i * 4u + 0u, msg_len, total_len, nonce, nonce_len_val);
            let b1 = padded_byte(offset + i * 4u + 1u, msg_len, total_len, nonce, nonce_len_val);
            let b2 = padded_byte(offset + i * 4u + 2u, msg_len, total_len, nonce, nonce_len_val);
            let b3 = padded_byte(offset + i * 4u + 3u, msg_len, total_len, nonce, nonce_len_val);
            w[i] = (b0 << 24u) | (b1 << 16u) | (b2 << 8u) | b3;
        }
        for (var i: u32 = 16u; i < 64u; i = i + 1u) {
            w[i] = ssig1(w[i - 2u]) + w[i - 7u] + ssig0(w[i - 15u]) + w[i - 16u];
        }

        var a = h[0];
        var b = h[1];
        var c = h[2];
        var d = h[3];
        var e = h[4];
        var f = h[5];
        var g = h[6];
        var vv = h[7];

        for (var i: u32 = 0u; i < 64u; i = i + 1u) {
            let t1 = vv + bsig1(e) + ch(e, f, g) + k(i) + w[i];
            let t2 = bsig0(a) + maj(a, b, c);
            vv = g;
            g = f;
            f = e;
            e = d + t1;
            d = c;
            c = b;
            b = a;
            a = t1 + t2;
        }

        h[0] = h[0] + a;
        h[1] = h[1] + b;
        h[2] = h[2] + c;
        h[3] = h[3] + d;
        h[4] = h[4] + e;
        h[5] = h[5] + f;
        h[6] = h[6] + g;
        h[7] = h[7] + vv;

        offset = offset + 64u;
    }

    return h;
}

fn padded_byte_from_data(idx: u32, msg_len: u32, total_len: u32, data: ptr<function, array<u32, 32>>) -> u32 {
    if idx < msg_len {
        return (*data)[idx];
    }
    if idx == msg_len {
        return 0x80u;
    }
    if idx >= total_len - 8u {
        let tail_idx = total_len - 1u - idx;
        return length_byte_from_msg_len(tail_idx, msg_len);
    }
    return 0u;
}

fn sha256_bytes(data: array<u32, 32>, msg_len: u32) -> array<u32, 8> {
    var h: array<u32, 8> = array<u32, 8>(
        0x6a09e667u, 0xbb67ae85u, 0x3c6ef372u, 0xa54ff53au,
        0x510e527fu, 0x9b05688cu, 0x1f83d9abu, 0x5be0cd19u,
    );

    let total_len = padded_len(msg_len);
    var data_var = data;
    var offset: u32 = 0u;
    loop {
        if offset >= total_len {
            break;
        }

        var w: array<u32, 64>;
        for (var i: u32 = 0u; i < 16u; i = i + 1u) {
            let base = offset + i * 4u;
            let b0 = padded_byte_from_data(base + 0u, msg_len, total_len, &data_var);
            let b1 = padded_byte_from_data(base + 1u, msg_len, total_len, &data_var);
            let b2 = padded_byte_from_data(base + 2u, msg_len, total_len, &data_var);
            let b3 = padded_byte_from_data(base + 3u, msg_len, total_len, &data_var);
            w[i] = (b0 << 24u) | (b1 << 16u) | (b2 << 8u) | b3;
        }

        for (var i: u32 = 16u; i < 64u; i = i + 1u) {
            w[i] = ssig1(w[i - 2u]) + w[i - 7u] + ssig0(w[i - 15u]) + w[i - 16u];
        }

        var a = h[0];
        var b = h[1];
        var c = h[2];
        var d = h[3];
        var e = h[4];
        var f = h[5];
        var g = h[6];
        var vv = h[7];

        for (var i: u32 = 0u; i < 64u; i = i + 1u) {
            let t1 = vv + bsig1(e) + ch(e, f, g) + k(i) + w[i];
            let t2 = bsig0(a) + maj(a, b, c);
            vv = g;
            g = f;
            f = e;
            e = d + t1;
            d = c;
            c = b;
            b = a;
            a = t1 + t2;
        }

        h[0] = h[0] + a;
        h[1] = h[1] + b;
        h[2] = h[2] + c;
        h[3] = h[3] + d;
        h[4] = h[4] + e;
        h[5] = h[5] + f;
        h[6] = h[6] + g;
        h[7] = h[7] + vv;

        offset = offset + 64u;
    }

    return h;
}

fn double_sha(nonce: vec2<u32>, nonce_len_val: u32, msg_len: u32) -> array<u32, 8> {
    var first = sha256_first(msg_len, nonce, nonce_len_val);
    var bytes: array<u32, 32>;
    for (var i: u32 = 0u; i < 8u; i = i + 1u) {
        let word = first[i];
        bytes[i * 4u + 0u] = (word >> 24u) & 0xffu;
        bytes[i * 4u + 1u] = (word >> 16u) & 0xffu;
        bytes[i * 4u + 2u] = (word >> 8u) & 0xffu;
        bytes[i * 4u + 3u] = word & 0xffu;
    }
    return sha256_bytes(bytes, 32u);
}

fn hash_to_txid_bytes(hash_words: array<u32, 8>) -> array<u32, 32> {
    // Copy to a mutable array to allow runtime indexing (Naga requires a
    // variable, not a value parameter, for dynamic indices).
    var words = hash_words;
    var bytes: array<u32, 32>;
    // Convert to txid view (little-endian overall, i.e., reverse of big-end digest).
    for (var i: u32 = 0u; i < 8u; i = i + 1u) {
        let word = words[i];
        let base = (7u - i) * 4u;
        bytes[base + 0u] = word & 0xffu;
        bytes[base + 1u] = (word >> 8u) & 0xffu;
        bytes[base + 2u] = (word >> 16u) & 0xffu;
        bytes[base + 3u] = (word >> 24u) & 0xffu;
    }
    return bytes;
}

fn leading_zeros(hash_words: array<u32, 8>) -> u32 {
    // Copy to a mutable array so we can use dynamic indexing (Naga requires a
    // var when the index is not a compile-time constant).
    var bytes = hash_to_txid_bytes(hash_words);

    var zeros: u32 = 0u;
    for (var i: u32 = 0u; i < 32u; i = i + 1u) {
        let byte = bytes[i];
        if byte == 0u {
            zeros = zeros + 2u;
            continue;
        }
        if (byte >> 4u) == 0u {
            zeros = zeros + 1u;
        }
        break;
    }
    return zeros;
}

fn meets_target(hash_words: array<u32, 8>, target_zeros: u32) -> bool {
    if target_zeros == 0u {
        return true;
    }
    if target_zeros > 64u {
        return false;
    }
    return leading_zeros(hash_words) >= target_zeros;
}

fn store_result(slot: u32, nonce: vec2<u32>, hash_words: array<u32, 8>) {
    RESULTS.results[slot].nonce_lo = nonce.x;
    RESULTS.results[slot].nonce_hi = nonce.y;
    var hash = hash_words;
    for (var i: u32 = 0u; i < 8u; i = i + 1u) {
        RESULTS.results[slot].txid[i] = hash[i];
    }
}

@compute @workgroup_size(WORKGROUP_SIZE, 1, 1)
fn main(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let idx = global_id.x;
    if idx >= PARAMS.batch_size {
        return;
    }

    var nonce = vec2<u32>(PARAMS.start_nonce_lo + idx, PARAMS.start_nonce_hi);
    if nonce.x < PARAMS.start_nonce_lo {
        nonce.y = nonce.y + 1u;
    }
    // Enforce minimal nonce width; avoid overflow when nonce grows.
    let nonce_len_val = PARAMS.nonce_len;
    if nonce_len_val == 0u || nonce_len_val > 9u {
        return;
    }
    // WGSL `if` is statement-only; use select for expression form.
    let min_nonce_len = select(
        raw_nonce_len(nonce),
        cbor_nonce_len(nonce),
        PARAMS.use_cbor_nonce != 0u,
    );
    if min_nonce_len != nonce_len_val {
        return;
    }
    let msg_len = PARAMS.prefix_len + nonce_len_val + PARAMS.suffix_len;

    let hash_words = double_sha(nonce, nonce_len_val, msg_len);
    if meets_target(hash_words, PARAMS.target_zeros) {
        let pos = atomicAdd(&RESULTS.found_count, 1u);
        if pos < MAX_RESULTS {
            store_result(pos, nonce, hash_words);
        }
    }
}

