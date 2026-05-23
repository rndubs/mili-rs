# `MiliBuffer<T>` — the shared zero-copy primitive

Every layer holds and passes the same type. This document defines its
contract; the implementation lives in `mili-rs`.

## Why a custom type rather than `Arc<Vec<T>>` or Arrow

- We back results with `mmap` of state files when we can — refcounting
  the mapping is the whole point.
- Endianness mismatches are common in HPC (file written on a
  different-endian host). We need to record that swap obligation
  without rewriting the file.
- Both numpy (via PyO3) and `wgpu` (via vertex buffers) want to view
  the same bytes without copying. `Arc<[T]>` is the right primitive;
  Arrow is overkill at this layer and forces a layout that does not
  match `OBJECT_ORDERED` subrecords.

## Shape

```rust
pub enum Storage {
    Mmap(Arc<memmap2::Mmap>),
    Owned(Arc<[u8]>),
}

pub struct MiliBuffer<T: bytemuck::Pod> {
    storage: Storage,
    offset: usize,
    len: usize,             // count of T, not bytes
    byteswap: bool,         // true if file endianness != host
    _marker: PhantomData<T>,
}
```

Three states are reachable:

1. **Native, mmap-backed** — pure zero-copy. `as_slice()` returns
   `&[T]` straight out of the mapping. The hot path.
2. **Native, owned** — a `Vec<T>` we allocated, e.g. for a gathered
   query result. Still zero-copy across FFI; the `Arc<[u8]>` keeps
   ownership clear.
3. **Byteswap required** — mmap-backed but host endian differs from
   file endian. `as_slice()` is unavailable; callers go through
   `to_owned()` which materializes a swapped copy on demand, or
   `for_each(|x| ...)` which swaps lazily.

## Contract

- `MiliBuffer<T>` is `Send + Sync` when `T: Pod + Send + Sync`.
- Cloning is `Arc::clone` of the storage; cheap.
- `len()` is the count of `T` elements. `byte_len()` is `len() *
  size_of::<T>()`.
- `as_slice() -> Option<&[T]>` returns `Some` iff `!byteswap` and the
  underlying offset is aligned for `T`. The alignment check matters
  because mili packs subrecords without padding (`srec.c:1894-1905`).
  When alignment fails we fall back to a copy.
- `to_owned(&self) -> Vec<T>` always succeeds; performs byteswap if
  needed, copy otherwise.
- The buffer never converts between numeric types. A `MiliBuffer<f32>`
  is `f32` end-to-end; precision selection happens in `mili-rs`'s
  read API based on the on-disk type code.

## Numpy bridge (`mili-py`)

For an aligned, native-endian buffer:

- Wrap as `PyArray` via the `numpy` crate using a capsule destructor
  that drops the `Arc<Storage>`. No copy.

For a byteswap-required or misaligned buffer:

- `to_owned()` into a `Vec<T>`, then transfer ownership into numpy via
  `PyArray::from_owned_array`. One copy, no further allocations.

The decision happens in Rust; Python only sees a numpy array.

## wgpu bridge (`mili-viz-server`)

Vertex and index data for the renderer is built by gathering from
several `MiliBuffer`s into a freshly allocated `Vec<u8>` shaped for
the GPU. We do not try to point `wgpu::Buffer` at an mmap; the
upload-to-GPU step is the natural copy boundary, and we want to
interleave attributes anyway. The buffer's job here is to be a cheap,
typed source for that build step.

## Lifetime / drop order

The `Arc<Storage>` is the root. Every typed view, every numpy capsule,
every gathered output that references the original bytes holds a
clone. The mapping is unmapped only when the last clone drops. This
gives us the property that callers — Python or otherwise — cannot
accidentally outlive the database; they hold their own refcount.

## Resolved

- **Public surface.** `MiliBuffer<T>` stays `pub(crate)`; callers
  consume `ndarray::Array<T,_>` / `ArrayView` returned by
  `Database::query` / `nodes` / `connectivity`. See
  [`../mili-rs/plan.md`](../mili-rs/plan.md) § "FFI integration
  plan".
- **Lazy in-place byteswap.** Not pursued — the byteswap copy is
  amortized over repeated reads of the same buffer when cached.
