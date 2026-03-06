# I/O Abstraction Ideas for zencodec-types

## Current state

Every decode entry point takes `data: &[u8]` — the full file must be in memory.
Every encode exit point returns `EncodeOutput` wrapping a `Vec<u8>`.
There are 22 call sites taking `data: &[u8]` across `Decoder`, `DecodeJob`,
`DecoderConfig`, and `FrameDecoder`.

## The no_std ecosystem

| Crate | Approach | no_std | Seek | Error type | Status |
|---|---|---|---|---|---|
| `embedded-io` 0.7 | Associated-error Read/Write/Seek | Yes | Yes | Associated | Active, 37M dl |
| `no-std-io` 0.6 | Clone of std::io API | Yes | Yes | Concrete struct | Stable, stale |
| `bytes` 1.11 | Buf/BufMut forward cursors | Yes | No | Panics/infallible | Active, 573M dl |
| `core2` 0.4 | Clone of std::io API | Yes | Yes | Concrete struct | Dead |
| `ciborium-io` 0.2 | read_exact/write_all only | Yes | No | Associated | Stable, 124M dl |

`core::io` is not happening. No stabilization plans on the 2025h1 or 2026
roadmaps. Blocked on making `io::Error` work without std.

## Why Read+Seek is the wrong abstraction

Image codecs don't do sequential reads. They do **random access**:

- **JPEG**: Scan for markers (FF xx), jump to SOF, jump to SOS, read entropy data.
- **AVIF/HEIF**: Parse ISOBMFF boxes — read headers, skip to next box, jump into `mdat`.
- **WebP**: Parse RIFF chunks at known offsets.
- **JXL**: Container has a table of contents with offsets.
- **PNG**: Sequential chunks, but read 8-byte header then skip or read the body.
- **GIF**: Sequential, LZW blocks have length prefixes for skipping.

`Read + Seek` carries mutable cursor state that `&[u8]` doesn't need and that
complicates concurrent access. The right primitive is **positioned reads** —
stateless, like Unix `pread()`.

## Approach 1: Positioned-read trait (ByteSource)

```rust
/// Stateless random-access byte source.
///
/// Unlike Read+Seek, reads are positioned and &self — no cursor state.
/// &[u8] implements this with zero overhead (direct slice indexing).
pub trait ByteSource {
    type Error;

    /// Total length in bytes, if known.
    fn len(&self) -> Option<u64>;

    /// Read bytes starting at `offset` into `buf`. Returns count read.
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize, Self::Error>;

    /// Contiguous view of all bytes, if available.
    ///
    /// Returns Some for in-memory sources (&[u8], mmap).
    /// Codecs use this to skip read_at entirely — direct indexing.
    fn as_contiguous(&self) -> Option<&[u8]> { None }
}

impl ByteSource for [u8] {
    type Error = core::convert::Infallible;

    fn len(&self) -> Option<u64> { Some(self.len() as u64) }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize, Infallible> {
        let off = offset as usize;
        if off >= self.len() { return Ok(0); }
        let n = buf.len().min(self.len() - off);
        buf[..n].copy_from_slice(&self[off..off + n]);
        Ok(n)
    }

    fn as_contiguous(&self) -> Option<&[u8]> { Some(self) }
}
```

**Pros:**
- `&[u8]` path is truly zero overhead — `as_contiguous()` returns `Some(self)`,
  codec uses direct indexing, `read_at` is never called
- `&self` not `&mut self` — immutable, shareable, no cursor state
- Memory-mapped files implement this trivially (they're a `&[u8]` underneath)
- File-backed impl does `pread()` — no cursor state shared between threads
- Associated error type — `Infallible` for `&[u8]`, `io::Error` for files
- Fits the actual random-access pattern codecs use

**Cons:**
- Novel pattern — nobody has impls ready
- Generics on decode methods make the traits non-object-safe, or you need
  `dyn ByteSource<Error = SomeError>` which loses the infallible optimization
- Codecs doing `data[offset..offset+len]` need to branch on `as_contiguous()`
  or call `read_at()` — moderate refactor

**Object-safety problem:** `zencodecs` needs runtime dispatch across codecs.
If `Decoder::decode` becomes `fn decode<S: ByteSource>(self, input: &S)`,
it can't be called through a trait object. Solutions:
1. Erase to `dyn ByteSource<Error = Box<dyn Error>>` — allocates for common case
2. Keep `&[u8]` on traits, add generic `_from` methods on concrete types only
3. Use an enum instead of a trait (see Approach 2)

## Approach 2: Enum input (InputData)

```rust
pub enum InputData<'a> {
    /// Complete data in memory. Zero-cost path.
    Slice(&'a [u8]),
    /// Positioned reader (file, network buffer, etc.)
    Source(&'a dyn DynByteSource),
}

/// Object-safe positioned reader.
trait DynByteSource {
    fn len(&self) -> Option<u64>;
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize, SourceError>;
}
```

**Pros:**
- Object-safe — no generics on trait methods
- Pattern match on `Slice` for the fast path
- Simple API: `decoder.decode(data.into())`
- Codec internals: `match input { Slice(d) => d[off..], Source(s) => s.read_at(...) }`

**Cons:**
- Dynamic dispatch for the Source path (vtable + type-erased error)
- Every call site in every codec handles both variants
- `SourceError` must be generic enough — probably needs allocation in `no_std`

## Approach 3: Keep &[u8], add parallel _from methods

```rust
// Existing trait surface unchanged:
trait Decoder {
    fn decode(self, data: &[u8]) -> Result<DecodeOutput, Self::Error>;
}

// Concrete codec types add:
impl JpegDecoder {
    fn decode_from<S: ByteSource>(self, source: S) -> Result<DecodeOutput, JpegError>;
}
```

**Pros:**
- Zero breakage, zero overhead for the existing path
- Generic `_from` methods on concrete types — generics work fine
- Codecs opt in when ready
- No trait changes needed in zencodec-types

**Cons:**
- Can't use `_from` methods through the generic trait
- Duplicated implementation in each codec
- Not discoverable — callers need to know about concrete type's extra methods

## Output side (encoding)

Less urgent. Codecs build output in `Vec<u8>` internally. A streaming
output trait would allow writing directly to a file:

```rust
pub trait ByteSink {
    type Error;
    fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error>;
    fn flush(&mut self) -> Result<(), Self::Error>;
}
```

`Vec<u8>` implements this with `Error = Infallible`. `embedded-io::Write`
is essentially this trait and could be used directly.

Requires codecs to restructure their output paths significantly. Big
refactor, moderate payoff (only matters for large images/animations where
the full encoded output doesn't fit comfortably in memory).

## Recommendation

**Start with positioned reads (ByteSource), but don't put it on the trait
surface yet.**

The `&[u8]` path covers 95%+ of use cases. Files get mmap'd or read_to_vec'd.
Most codecs need random access anyway, so they'd buffer internally even with
streaming input.

Concrete plan:

1. **Define `ByteSource` in zencodec-types** with `[u8]` impl
2. **Don't add it to the decode traits** — add to concrete codec types as
   `decode_from<S: ByteSource>()`
3. **Prove it out across 2-3 codecs** before promoting to the trait surface
4. **Use `embedded-io::Write` for the output side** if streaming output
   becomes a priority

The positioned-read pattern (`read_at` with `&self`) is better than
`Read + Seek` for image codecs. It matches how codecs access data, it's
naturally concurrent, and it's zero-cost for `&[u8]`. But it's novel enough
to validate before baking into the shared trait surface.

## embedded-io as a dependency?

`embedded-io` 0.7 is the best-designed no_std I/O trait crate:
- Zero dependencies, actively maintained
- Associated error types (no allocating `io::Error`)
- Has Seek
- `&[u8]` implements Read + BufRead directly

But it uses `Read + Seek` (cursor-based), not positioned reads. For the
decode/input side, a custom `ByteSource` trait is better. For the
encode/output side, `embedded-io::Write` would work well — writing is
inherently sequential.

Possible: depend on `embedded-io` for Write, define our own ByteSource
for Read. Or define both ourselves to avoid the dependency. The traits
are small enough that owning them is fine.
