//! Bloom filter specialised for 20-byte hash160 keys.
//!
//! False positives are allowed (exact check happens on disk). False negatives
//! are a correctness bug and are covered by tests.

#[derive(Clone, Debug)]
pub struct Bloom {
    bits: Vec<u64>,
    bit_len: u64,
    k: u32,
    bits_per_key: u32,
}

impl Bloom {
    pub fn new(n: usize, bits_per_key: u32) -> Self {
        let bits_per_key = bits_per_key.clamp(8, 32);
        // Keep the filter sized to bits/key rather than rounding up to a
        // power of two (that jump is 128MB vs ~85MB at 44M keys / 16 bits).
        let mut bit_len = (n as u64).saturating_mul(u64::from(bits_per_key)).max(64);
        bit_len = bit_len.div_ceil(64) * 64;
        let words = (bit_len / 64) as usize;
        let k = optimal_k(bits_per_key);
        Self {
            bits: vec![0u64; words.max(1)],
            bit_len: (words.max(1) as u64) * 64,
            k,
            bits_per_key,
        }
    }

    pub fn from_parts(bits: Vec<u64>, k: u32, bits_per_key: u32) -> Self {
        let bit_len = bits.len() as u64 * 64;
        Self {
            bits,
            bit_len,
            k: k.max(1),
            bits_per_key,
        }
    }

    #[inline]
    pub fn insert(&mut self, key: &[u8; 20]) {
        let (h1, h2) = mix(key);
        let bit_len = self.bit_len;
        for i in 0..self.k {
            let bit = nth_bit(h1, h2, i, bit_len);
            let word = (bit / 64) as usize;
            self.bits[word] |= 1u64 << (bit % 64);
        }
    }

    #[inline]
    pub fn maybe_contains(&self, key: &[u8; 20]) -> bool {
        let (h1, h2) = mix(key);
        let bit_len = self.bit_len;
        for i in 0..self.k {
            let bit = nth_bit(h1, h2, i, bit_len);
            let word = (bit / 64) as usize;
            if self.bits[word] & (1u64 << (bit % 64)) == 0 {
                return false;
            }
        }
        true
    }

    pub fn byte_len(&self) -> usize {
        self.bits.len() * 8
    }

    pub fn k(&self) -> u32 {
        self.k
    }

    pub fn bits_per_key(&self) -> u32 {
        self.bits_per_key
    }

    pub fn write_to(&self, out: &mut Vec<u8>) {
        out.reserve(self.byte_len());
        for word in &self.bits {
            out.extend_from_slice(&word.to_le_bytes());
        }
    }

    pub fn from_bytes(bytes: &[u8], k: u32, bits_per_key: u32) -> Result<Self, &'static str> {
        if bytes.len() % 8 != 0 || bytes.is_empty() {
            return Err("bloom bytes must be a non-empty multiple of 8");
        }
        let mut bits = vec![0u64; bytes.len() / 8];
        for (i, chunk) in bytes.chunks_exact(8).enumerate() {
            bits[i] = u64::from_le_bytes(chunk.try_into().unwrap());
        }
        Ok(Self::from_parts(bits, k, bits_per_key))
    }
}

fn optimal_k(bits_per_key: u32) -> u32 {
    // k ≈ ln(2) * bits/key, clamped to a small integer.
    ((f64::from(bits_per_key) * std::f64::consts::LN_2).round() as u32).clamp(4, 24)
}

#[inline]
fn nth_bit(h1: u64, h2: u64, i: u32, bit_len: u64) -> u64 {
    let h = h1.wrapping_add(h2.wrapping_mul(u64::from(i)));
    ((h as u128 * bit_len as u128) >> 64) as u64
}

#[inline]
fn mix(key: &[u8; 20]) -> (u64, u64) {
    let mut h1 = u64::from_le_bytes(key[0..8].try_into().unwrap());
    let mut h2 = u64::from_le_bytes(key[8..16].try_into().unwrap());
    let tail = u32::from_le_bytes(key[16..20].try_into().unwrap()) as u64;
    h1 ^= tail.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    h2 ^= h1.rotate_left(17);
    h1 = h1.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h2 = h2.wrapping_mul(0x94D0_49BB_1331_11EB) | 1;
    (h1, h2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_false_negatives_and_few_false_positives() {
        let mut keys = Vec::new();
        for i in 0..10_000u32 {
            let mut k = [0u8; 20];
            k[0..4].copy_from_slice(&i.to_le_bytes());
            k[4..8].copy_from_slice(&(i.wrapping_mul(0x9e37_79b9)).to_le_bytes());
            keys.push(k);
        }
        let mut bloom = Bloom::new(keys.len(), 16);
        for k in &keys {
            bloom.insert(k);
        }
        for k in &keys {
            assert!(bloom.maybe_contains(k), "false negative");
        }
        let mut fp = 0u32;
        for i in 0..10_000u32 {
            let mut k = [0xffu8; 20];
            k[0..4].copy_from_slice(&i.to_le_bytes());
            if bloom.maybe_contains(&k) {
                fp += 1;
            }
        }
        assert!(fp < 50, "unexpectedly high false-positive count {fp}");
        let mut bytes = Vec::new();
        bloom.write_to(&mut bytes);
        let roundtrip = Bloom::from_bytes(&bytes, bloom.k(), bloom.bits_per_key()).unwrap();
        assert!(roundtrip.maybe_contains(&keys[0]));
    }
}
