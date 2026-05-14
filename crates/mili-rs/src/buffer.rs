//! `MiliBuffer<T>` — the shared zero-copy primitive.
//!
//! Carries a refcounted handle to either an `mmap` of the source file
//! or an owned byte buffer, plus an `(offset, len, byteswap)` view
//! over it. Three states are reachable:
//!
//! 1. **Native, mmap-backed**: [`Self::as_slice`] returns `Some` and
//!    hands out a borrowed `&[T]` straight from the mapping.
//! 2. **Native, owned**: same as above but the storage is a heap
//!    allocation we own (e.g. a gathered query result).
//! 3. **Byteswap required, or misaligned**: [`Self::as_slice`] returns
//!    `None`; callers materialise a [`Vec`] via [`Self::to_owned`] or
//!    iterate without allocating via [`Self::for_each`].
//!
//! The alignment check matters because mili packs subrecords without
//! padding (`reference/mili/src/srec.c:1894-1905`).
//!
//! Kept `pub(crate)` until a downstream crate (`mili-py`, `mili-viz`)
//! has a concrete need for the raw view — `planning/mili-rs/status.md`
//! § "Open questions" tracks the decision.

#![allow(dead_code)]

use std::marker::PhantomData;
use std::sync::Arc;

use memmap2::Mmap;

use crate::endian::{for_each_swap, ByteSwap};

#[derive(Clone)]
pub(crate) enum Storage {
    Mmap(Arc<Mmap>),
    Owned(Arc<[u8]>),
}

impl Storage {
    fn bytes(&self) -> &[u8] {
        match self {
            Storage::Mmap(m) => m,
            Storage::Owned(b) => b,
        }
    }
}

pub(crate) struct MiliBuffer<T: ByteSwap> {
    storage: Storage,
    offset: usize,
    len: usize,
    byteswap: bool,
    _marker: PhantomData<T>,
}

impl<T: ByteSwap> Clone for MiliBuffer<T> {
    fn clone(&self) -> Self {
        Self {
            storage: self.storage.clone(),
            offset: self.offset,
            len: self.len,
            byteswap: self.byteswap,
            _marker: PhantomData,
        }
    }
}

// `Storage` is `Arc<Mmap>` or `Arc<[u8]>`, both `Send + Sync` when the
// payload is. `PhantomData<T>` carries through `T`'s Send/Sync bounds.
unsafe impl<T: ByteSwap + Send + Sync> Send for MiliBuffer<T> {}
unsafe impl<T: ByteSwap + Send + Sync> Sync for MiliBuffer<T> {}

impl<T: ByteSwap> MiliBuffer<T> {
    pub fn from_mmap(mmap: Arc<Mmap>, offset: usize, len: usize, byteswap: bool) -> Self {
        Self {
            storage: Storage::Mmap(mmap),
            offset,
            len,
            byteswap,
            _marker: PhantomData,
        }
    }

    pub fn from_owned(bytes: Arc<[u8]>, offset: usize, len: usize, byteswap: bool) -> Self {
        Self {
            storage: Storage::Owned(bytes),
            offset,
            len,
            byteswap,
            _marker: PhantomData,
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn byte_len(&self) -> usize {
        self.len * std::mem::size_of::<T>()
    }

    pub fn is_byteswap(&self) -> bool {
        self.byteswap
    }

    fn raw(&self) -> &[u8] {
        &self.storage.bytes()[self.offset..self.offset + self.byte_len()]
    }

    /// Borrow the underlying bytes as `&[T]` if and only if no
    /// transformation is required: bytes are in host endianness and
    /// the view's start is aligned for `T`.
    pub fn as_slice(&self) -> Option<&[T]> {
        if self.byteswap {
            return None;
        }
        let bytes = self.raw();
        if !(bytes.as_ptr() as usize).is_multiple_of(std::mem::align_of::<T>()) {
            return None;
        }
        Some(bytemuck::cast_slice(bytes))
    }

    /// Materialise an owned `Vec<T>`. Performs the byteswap if
    /// required; otherwise either a plain copy from the borrowed slice
    /// (when aligned) or an unaligned-read copy (when misaligned).
    pub fn to_owned(&self) -> Vec<T> {
        if let Some(slice) = self.as_slice() {
            return slice.to_vec();
        }
        let mut out = Vec::with_capacity(self.len);
        for_each_swap::<T, _>(self.raw(), self.byteswap, |v| out.push(v));
        out
    }

    /// Iterate the buffer as a stream of `T`, applying the byteswap on
    /// the fly. No `Vec` is allocated.
    pub fn for_each<F: FnMut(T)>(&self, f: F) {
        for_each_swap::<T, _>(self.raw(), self.byteswap, f);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owned_buf<T: ByteSwap>(values: &[T], byteswap: bool, prepad: usize) -> MiliBuffer<T> {
        let mut bytes = vec![0u8; prepad];
        for v in values {
            let raw: &[u8] = bytemuck::bytes_of(v);
            bytes.extend_from_slice(raw);
        }
        let arc: Arc<[u8]> = Arc::from(bytes.into_boxed_slice());
        MiliBuffer::<T>::from_owned(arc, prepad, values.len(), byteswap)
    }

    #[test]
    fn as_slice_aligned_native_returns_some() {
        let xs = [1i32, 2, 3, 4];
        let buf = owned_buf(&xs, false, 0);
        let s = buf.as_slice().expect("aligned native");
        assert_eq!(s, &xs);
    }

    #[test]
    fn as_slice_byteswap_returns_none() {
        let xs = [1i32, 2, 3];
        let buf = owned_buf(&xs, true, 0);
        assert!(buf.as_slice().is_none());
    }

    #[test]
    fn as_slice_misaligned_returns_none() {
        // Prepad 1 byte → an f32 view at offset=1 cannot be 4-byte aligned.
        let xs = [1.5_f32, -2.0, 7.0];
        let buf = owned_buf(&xs, false, 1);
        // Note: depending on the heap allocator's alignment, offset=1 from
        // a typical malloc address (≥ 4-byte aligned) is guaranteed
        // misaligned for f32.
        assert!(
            buf.as_slice().is_none(),
            "expected misaligned view to refuse as_slice"
        );
    }

    #[test]
    fn to_owned_native_aligned() {
        let xs = [10i32, -20, 30];
        let buf = owned_buf(&xs, false, 0);
        assert_eq!(buf.to_owned(), xs);
    }

    #[test]
    fn to_owned_native_misaligned() {
        let xs = [1.0_f32, 2.5, -3.0];
        let buf = owned_buf(&xs, false, 1);
        assert_eq!(buf.to_owned(), xs);
    }

    #[test]
    fn to_owned_byteswap_aligned() {
        let xs = [1i32, 2, 3];
        let bytes: Vec<u8> = xs.iter().flat_map(|v| v.to_ne_bytes()).collect();
        let arc: Arc<[u8]> = Arc::from(bytes.into_boxed_slice());
        let buf = MiliBuffer::<i32>::from_owned(arc, 0, 3, true);
        let expect: Vec<i32> = xs.iter().copied().map(ByteSwap::swap_bytes).collect();
        assert_eq!(buf.to_owned(), expect);
    }

    #[test]
    fn to_owned_byteswap_misaligned() {
        let xs = [1.0_f32, 2.5];
        let mut bytes = vec![0u8];
        for x in &xs {
            bytes.extend_from_slice(&x.to_ne_bytes());
        }
        let arc: Arc<[u8]> = Arc::from(bytes.into_boxed_slice());
        let buf = MiliBuffer::<f32>::from_owned(arc, 1, 2, true);
        let expect: Vec<f32> = xs.iter().copied().map(ByteSwap::swap_bytes).collect();
        let got = buf.to_owned();
        // bit-equal comparison so NaN payload differences (if any) are
        // visible, though no NaNs in this fixture.
        for (g, e) in got.iter().zip(expect.iter()) {
            assert_eq!(g.to_bits(), e.to_bits());
        }
    }

    #[test]
    fn to_owned_mmap_native_aligned() {
        // Build a small temp file and mmap it.
        use std::io::Write;
        let dir = tempdir();
        let path = dir.join("buf.bin");
        let xs = [11i32, 22, 33, 44];
        let bytes: Vec<u8> = xs.iter().flat_map(|v| v.to_ne_bytes()).collect();
        {
            let mut f = std::fs::File::create(&path).unwrap();
            f.write_all(&bytes).unwrap();
        }
        let file = std::fs::File::open(&path).unwrap();
        let mmap = unsafe { Mmap::map(&file) }.unwrap();
        let buf = MiliBuffer::<i32>::from_mmap(Arc::new(mmap), 0, xs.len(), false);
        assert_eq!(buf.to_owned(), xs);
        // mmap pages are page-aligned, so the slice path must be reachable.
        assert!(buf.as_slice().is_some());
    }

    #[test]
    fn to_owned_mmap_misaligned() {
        use std::io::Write;
        let dir = tempdir();
        let path = dir.join("buf-mis.bin");
        let xs = [1.5_f32, 2.5, -3.0, 4.0];
        let mut bytes = vec![0u8];
        for x in &xs {
            bytes.extend_from_slice(&x.to_ne_bytes());
        }
        {
            let mut f = std::fs::File::create(&path).unwrap();
            f.write_all(&bytes).unwrap();
        }
        let file = std::fs::File::open(&path).unwrap();
        let mmap = unsafe { Mmap::map(&file) }.unwrap();
        let buf = MiliBuffer::<f32>::from_mmap(Arc::new(mmap), 1, xs.len(), false);
        assert!(buf.as_slice().is_none());
        assert_eq!(buf.to_owned(), xs);
    }

    #[test]
    fn to_owned_mmap_byteswap() {
        use std::io::Write;
        let dir = tempdir();
        let path = dir.join("buf-swap.bin");
        let xs = [1.5_f32, -2.5];
        let bytes: Vec<u8> = xs.iter().flat_map(|v| v.to_ne_bytes()).collect();
        {
            let mut f = std::fs::File::create(&path).unwrap();
            f.write_all(&bytes).unwrap();
        }
        let file = std::fs::File::open(&path).unwrap();
        let mmap = unsafe { Mmap::map(&file) }.unwrap();
        let buf = MiliBuffer::<f32>::from_mmap(Arc::new(mmap), 0, xs.len(), true);
        assert!(buf.as_slice().is_none());
        let expect: Vec<f32> = xs.iter().copied().map(ByteSwap::swap_bytes).collect();
        let got = buf.to_owned();
        for (g, e) in got.iter().zip(expect.iter()) {
            assert_eq!(g.to_bits(), e.to_bits());
        }
    }

    #[test]
    fn for_each_matches_to_owned() {
        let xs = [3i64, -7, 0x0102_0304_0506_0708, i64::MIN];
        let bytes: Vec<u8> = xs.iter().flat_map(|v| v.to_ne_bytes()).collect();
        let arc: Arc<[u8]> = Arc::from(bytes.into_boxed_slice());
        for &byteswap in &[false, true] {
            let buf = MiliBuffer::<i64>::from_owned(arc.clone(), 0, xs.len(), byteswap);
            let owned = buf.to_owned();
            let mut iterated = Vec::new();
            buf.for_each(|v| iterated.push(v));
            assert_eq!(iterated, owned);
        }
    }

    #[test]
    fn clone_shares_storage() {
        let xs = [9i32, 8, 7];
        let buf = owned_buf(&xs, false, 0);
        let buf2 = buf.clone();
        assert_eq!(buf2.to_owned(), xs);
        assert_eq!(buf.len(), buf2.len());
    }

    #[test]
    fn byte_len_matches_count_times_width() {
        let xs = [1.0_f64, 2.0, 3.0];
        let buf = owned_buf(&xs, false, 0);
        assert_eq!(buf.len(), 3);
        assert_eq!(buf.byte_len(), 24);
    }

    fn tempdir() -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        let nonce = format!(
            "mili-rs-buf-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        p.push(nonce);
        std::fs::create_dir_all(&p).unwrap();
        p
    }
}
