#![allow(dead_code)]

//! Byteswap helpers for the four numeric widths mili stores on disk.
//!
//! Mili files written on a host with a different byte order than the
//! one we're reading on stay verbatim on disk; the reader must swap on
//! the fly. These helpers are the single point that decides how to do
//! that, used both by [`crate::buffer::MiliBuffer`] on the copy-fallback
//! path and by the query-layer gather code.

use bytemuck::Pod;

/// A scalar that can be byteswapped to or from disk endianness.
///
/// Implemented for the four mili-relevant widths: `i32`, `i64`, `f32`,
/// `f64`. The integer impls forward to the std primitive `swap_bytes`;
/// the float impls swap the bit pattern, since IEEE 754 has no direct
/// equivalent.
pub trait ByteSwap: Pod {
    fn swap_bytes(self) -> Self;
}

impl ByteSwap for i32 {
    #[inline]
    fn swap_bytes(self) -> Self {
        i32::swap_bytes(self)
    }
}

impl ByteSwap for i64 {
    #[inline]
    fn swap_bytes(self) -> Self {
        i64::swap_bytes(self)
    }
}

impl ByteSwap for f32 {
    #[inline]
    fn swap_bytes(self) -> Self {
        f32::from_bits(self.to_bits().swap_bytes())
    }
}

impl ByteSwap for f64 {
    #[inline]
    fn swap_bytes(self) -> Self {
        f64::from_bits(self.to_bits().swap_bytes())
    }
}

pub fn swap_i32_slice(slice: &mut [i32]) {
    for v in slice {
        *v = ByteSwap::swap_bytes(*v);
    }
}

pub fn swap_i64_slice(slice: &mut [i64]) {
    for v in slice {
        *v = ByteSwap::swap_bytes(*v);
    }
}

pub fn swap_f32_slice(slice: &mut [f32]) {
    for v in slice {
        *v = ByteSwap::swap_bytes(*v);
    }
}

pub fn swap_f64_slice(slice: &mut [f64]) {
    for v in slice {
        *v = ByteSwap::swap_bytes(*v);
    }
}

/// Walk a byte slice as a stream of `T`, yielding each scalar with the
/// byteswap applied if `byteswap` is set, without allocating an
/// intermediate `Vec`. Bytes are read unaligned; trailing bytes that
/// don't fill a full `T` are ignored.
#[inline]
pub fn for_each_swap<T: ByteSwap, F: FnMut(T)>(bytes: &[u8], byteswap: bool, mut f: F) {
    let width = std::mem::size_of::<T>();
    for chunk in bytes.chunks_exact(width) {
        let mut v: T = bytemuck::pod_read_unaligned(chunk);
        if byteswap {
            v = v.swap_bytes();
        }
        f(v);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swap_i32_known_value() {
        let v: i32 = 0x0102_0304;
        assert_eq!(ByteSwap::swap_bytes(v), 0x0403_0201);
    }

    #[test]
    fn swap_i64_known_value() {
        let v: i64 = 0x0102_0304_0506_0708;
        assert_eq!(ByteSwap::swap_bytes(v), 0x0807_0605_0403_0201);
    }

    #[test]
    fn swap_f32_round_trip_via_bits() {
        let v: f32 = 1.5_f32;
        let bytes = v.to_le_bytes();
        let swapped = ByteSwap::swap_bytes(v);
        let expected = f32::from_be_bytes(bytes);
        assert_eq!(swapped.to_bits(), expected.to_bits());
    }

    #[test]
    fn swap_f64_round_trip_via_bits() {
        let v: f64 = -2.5_f64;
        let bytes = v.to_le_bytes();
        let swapped = ByteSwap::swap_bytes(v);
        let expected = f64::from_be_bytes(bytes);
        assert_eq!(swapped.to_bits(), expected.to_bits());
    }

    #[test]
    fn swap_swap_is_identity_i32() {
        for v in [0i32, 1, -1, i32::MIN, i32::MAX, 0x1234_5678] {
            assert_eq!(ByteSwap::swap_bytes(ByteSwap::swap_bytes(v)), v);
        }
    }

    #[test]
    fn swap_swap_is_identity_i64() {
        for v in [
            0i64,
            1,
            -1,
            i64::MIN,
            i64::MAX,
            0x1234_5678_9ABC_DEF0u64 as i64,
        ] {
            assert_eq!(ByteSwap::swap_bytes(ByteSwap::swap_bytes(v)), v);
        }
    }

    #[test]
    fn swap_swap_is_identity_f32() {
        for v in [0.0_f32, 1.0, -1.0, f32::MIN, f32::MAX, std::f32::consts::PI] {
            assert_eq!(
                ByteSwap::swap_bytes(ByteSwap::swap_bytes(v)).to_bits(),
                v.to_bits()
            );
        }
    }

    #[test]
    fn swap_swap_is_identity_f64() {
        for v in [0.0_f64, 1.0, -1.0, f64::MIN, f64::MAX, std::f64::consts::PI] {
            assert_eq!(
                ByteSwap::swap_bytes(ByteSwap::swap_bytes(v)).to_bits(),
                v.to_bits()
            );
        }
    }

    #[test]
    fn swap_i32_slice_matches_scalar() {
        let mut s = [1i32, -2, 0x0102_0304, i32::MIN];
        let expect: Vec<i32> = s.iter().copied().map(ByteSwap::swap_bytes).collect();
        swap_i32_slice(&mut s);
        assert_eq!(s.to_vec(), expect);
    }

    #[test]
    fn swap_i64_slice_matches_scalar() {
        let mut s = [1i64, -2, 0x0102_0304_0506_0708, i64::MIN];
        let expect: Vec<i64> = s.iter().copied().map(ByteSwap::swap_bytes).collect();
        swap_i64_slice(&mut s);
        assert_eq!(s.to_vec(), expect);
    }

    #[test]
    fn swap_f32_slice_matches_scalar() {
        let mut s = [1.0_f32, -2.5, std::f32::consts::PI, 0.0];
        let expect: Vec<u32> = s
            .iter()
            .copied()
            .map(|x| ByteSwap::swap_bytes(x).to_bits())
            .collect();
        swap_f32_slice(&mut s);
        let got: Vec<u32> = s.iter().map(|x| x.to_bits()).collect();
        assert_eq!(got, expect);
    }

    #[test]
    fn swap_f64_slice_matches_scalar() {
        let mut s = [1.0_f64, -2.5, std::f64::consts::PI, 0.0];
        let expect: Vec<u64> = s
            .iter()
            .copied()
            .map(|x| ByteSwap::swap_bytes(x).to_bits())
            .collect();
        swap_f64_slice(&mut s);
        let got: Vec<u64> = s.iter().map(|x| x.to_bits()).collect();
        assert_eq!(got, expect);
    }

    #[test]
    fn for_each_swap_yields_native_when_not_byteswapped() {
        let xs = [10i32, -7, 0x1234_5678];
        let bytes: Vec<u8> = xs.iter().flat_map(|x| x.to_ne_bytes()).collect();
        let mut got = Vec::new();
        for_each_swap::<i32, _>(&bytes, false, |v| got.push(v));
        assert_eq!(got, xs);
    }

    #[test]
    fn for_each_swap_yields_swapped_when_byteswapped() {
        let xs = [10i32, -7, 0x1234_5678];
        let bytes: Vec<u8> = xs.iter().flat_map(|x| x.to_ne_bytes()).collect();
        let mut got = Vec::new();
        for_each_swap::<i32, _>(&bytes, true, |v| got.push(v));
        let expect: Vec<i32> = xs.iter().copied().map(ByteSwap::swap_bytes).collect();
        assert_eq!(got, expect);
    }

    #[test]
    fn for_each_swap_handles_unaligned_input() {
        // Offset the byte stream by 1 so it cannot be 4-byte aligned.
        let xs = [1.5_f32, -3.25, 7.0];
        let mut padded = vec![0u8];
        for x in &xs {
            padded.extend_from_slice(&x.to_ne_bytes());
        }
        let mut got = Vec::new();
        for_each_swap::<f32, _>(&padded[1..], false, |v| got.push(v));
        assert_eq!(got, xs);
    }

    #[test]
    fn for_each_swap_ignores_trailing_partial_chunk() {
        let bytes = [0u8, 0, 0, 1, 0xFF]; // 5 bytes, only 1 full i32
        let mut got = Vec::new();
        for_each_swap::<i32, _>(&bytes, false, |v| got.push(v));
        assert_eq!(got.len(), 1);
    }
}
