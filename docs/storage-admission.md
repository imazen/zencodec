# Storage that remains charged while owned

`zencodec_io::storage::StorageBudget` admits fixed-size payload allocations before
creating their vectors. Its two independent limits count managed vector capacity
in bytes and outstanding allocations. Zero is a real limit. Byte admission that
fails returns the allocation slot it tentatively reserved.

```rust
use zencodec_io::storage::StorageBudget;

let budget = StorageBudget::new(128 * 1024, 8);
let mut incoming = budget.bytes(64 * 1024)?;
// Fill through incoming.as_mut_slice(); it cannot grow the allocation.
let bytes = incoming.freeze();
let packet = bytes.slice(100..200)?;
drop(bytes);
// packet still owns the whole backing allocation: 65,536 bytes stay charged.
assert_eq!(budget.reserved_bytes(), 64 * 1024);
drop(packet);
assert_eq!(budget.reserved_bytes(), 0);
```

`bytes`, `words` and `floats` allocate initialized u8, u16 and f32 storage.
The charge includes row padding and the whole backing vector, even for an empty
subview. `freeze` shares the allocation without a payload copy. Cloning a shared
view keeps the same charge; the last owner frees the data before releasing it.
There is no resize or detach-the-vector method on the admitted types.

This is **managed payload accounting**, not a whole-operation RAM or RSS bound.
Allocator bookkeeping, reference-count headers, transient allocator behavior,
native codec workspace, parser allocations and copies outside this allocator
are excluded. If an allocator exposes more vector capacity than was admitted,
the temporary allocation is freed and refused before any owner is published.
The implementation does not claim that this transient allocator behavior is
bounded by the managed allowance. The small shared-owner header uses `Arc::new`;
the potentially large vector uses fallible reservation.

## Reserving a route before policy

`StorageBudget::prepare_filled(len, value)` reserves bytes and an object slot
without allocating or initializing the array. The returned `PreparedStorage<T>`
holds that allowance across a callback. A rejected route drops its reservation;
a selected route calls `allocate()` and transfers the same charge into its
`OwnedStorage<T>`. Allocation can still fail, releasing the allowance. The stored
fill value itself is outside the array-capacity charge.

This lets a caller reject a proposed 10 GiB destination before it materializes.
It also prevents competing operations from taking the reserved allowance while
the caller decides. The reservation is not a system-allocator guarantee and
holds no budget lock during application code. Cloning is deliberately absent.

The native AV1 conversion witness retains both destination and one-row scratch
reservations during `Question::ImageConversion`, then initializes them only
after selection. Refusal, cancellation, partial admission and callback unwind
release them. Scratch drops after execution; output clones retain only the
completed destination charge. See
[the executing witness](../perf/av1-policy-validation/README.md#prepared-conversion-before-allocation).

## Media owners

`Packet::from_accounted` and `PacketPayload::Accounted` keep a shared byte view.
Taking a packet's payload, cloning a packet or retaining side data preserves
the charge. `PlaneData::{AccountedU8, AccountedU16}` carries admitted storage
through `OwnedPlanes`, `Picture`, frame clones and show-existing presentations.
The 16-bit path keeps native words, sample encoding and strided rows intact.

An old frame can outlive its decoder, queue entry and configuration epoch.
It continues to consume the original budget while a new epoch allocates its
own storage. Dropping the old decoder handle is not permission to reuse that
memory allowance. Mapping borrows the picture owner, so a mapping cannot outlive
the charge it depends on.

Legacy `PacketPayload::into_vec` explicitly creates an ordinary unaccounted
copy for shared/accounted input. Budget-enforcing adapters must retain the
accounted payload or admit any new copy themselves. This extension does not
retrofit allocation enforcement into existing native codec internals.

## Verification and remaining integration

The packaged JXL collection adapter accepts a shared output pool through
`OpenRequest::with_output_storage`. It admits aligned color and auxiliary
payloads before native writes, then gives their owner to
`ImageOutput::from_packed_owner`. Neither publication nor cloning copies samples.
Charges include padding and survive collection, registry and input destruction
until the last returned image drops. This covers output payloads only; a request
for a whole-operation memory limit still receives a typed refusal.

The [ordinary/admitted native matrix](../benchmarks/operation_output_storage_2026-10-26.json)
checks 147 presentation selections and nine raw layers with exact samples,
strides and padding, alongside the existing independent-reader gates. Package
consumer controls also cover partial byte/object admission, cancellation,
callback decline/error, unwind, alignment and cross-operation contention. The
pool is supplied in the common operation request. Unqualified adapters return
`CollectionOpenError::UnsupportedOutputStorage` before affected work; the private
TIFF factory qualifies this refusal before any source query. Separate operations
can use independent pools through one immutable registry. Native-workspace
requirements still refuse, even with an output pool.

Workspace controls exercise allocation-count and byte-cap contention across
threads, failed-admission rollback, arithmetic overflow, unwind cleanup, nested
subviews, empty views, packet side data, old-epoch frame retention and exact
strided 8/10-bit samples. These establish ownership and admission mechanics;
they do not measure native codec memory. Run `just io-contract-check`.

The current profile-aware `zenpixels-convert` finalizer is not an admitted
conversion path. Native CMYK controls prove identity preserves pixels and ICC,
but its `&PixelBuffer` input requires an owned copy from a mapped native image,
and its owned result creates another allocation. Both survive after the decode
pool returns to zero. See the [executing controls](../perf/spatial-validation/README.md#cmyk-output-finalization)
and [existing ownership issue](https://github.com/imazen/zenpixels/issues/69).

The combined review revision's `OutputPlan` supplies a separate, checked path:
the native JXL color-stage prototype borrows mapped input and writes directly
into an admitted, aligned destination. All nine raw layers preserve color bits
and ICC. Refusal, late CMS errors and unwinding release unpublished destination
charges. Checked identity borrowing additionally retains the original image and
all auxiliary planes without another color allocation, provided its frozen
descriptor and color context already match the plan. An identity sample view
with different output signaling cannot simply reuse the unchanged image.
Complete converted auxiliary-plane publication and CMS workspace admission
remain open.
See [the native execution](../perf/spatial-validation/README.md#existing-outputplan-with-admitted-destination-storage).

Policy waiting, conversion scratch and encoded staging must use this ownership
mechanism when they claim a managed payload allowance. A hard whole-operation
limit additionally needs qualified native codec and service bounds. The ordinary
policy resolver must refuse an unavailable bound rather than treating this
allocator's narrower accounting as proof of total memory use.

## Measured allocation cost

[Recorded output](../benchmarks/storage_admission_2026-10-25.txt) at
`42c93a27`, `just storage-cost-bench 4`, Ryzen 9 7900X, Rust 1.99.1,
thin LTO, no target-cpu override. Zenbench reports the median of three passes;
the ± value is inter-pass MAD. The effective benchmark thread set was CPU 4;
frequency was unverified. These are exploratory allocation measurements.

| Payload | Ordinary Vec + Arc | Private admission prototype | Production StorageBudget |
|---|---:|---:|---:|
| 1,024 bytes | 15.4 ±0.5 ns | 55.4 ±0.7 ns | 52.3 ±2.7 ns |
| 65,536 bytes | 852 ±61 ns | 1,228 ±1 ns | 1,239 ±16 ns |

Each case allocates, fills, shares, clones an owner and drops it. The ordinary
case clones an existing vector directly; both admitted cases initialize to zero
before copying the payload. The full difference therefore includes that extra
initialization as well as bookkeeping. It does not isolate the cost of atomics,
and it is not a codec-throughput measurement.

```text
run-heavy: done rc=0 97s | peak-RSS 0.43GiB | min-avail 17430MiB | peak-load 3.10
```

The receipt includes compilation. Native JPEG validation also confirms that an
encode from admitted strided storage produces the same bytes and independent
decoded output as the ordinary raster. Finishing the encode leaves that raster
charged while either caller-owned view remains alive. Encoded staging can only
release storage the finishing producer actually owns.

## Shared ownership location

`zencodec::storage` owns the allocation types so native codecs and coroutine
authors can participate without depending on an I/O driver. The existing
`zencodec_io::storage` path re-exports those same types. An owner created through
either path participates in the same budget and preserves the same lifetime.

`StorageBudget::filled` admits fixed arrays of copyable values, including codec
range indexes. It charges array capacity, not storage indirectly referenced by
array elements. The coroutine admitted path now uses it for its range index.

## Coroutine storage cost

The [recorded run](../benchmarks/coroutine_admission_2026-10-25.txt) compares the
same authored memcpy/control operation with legacy and admitted input/row storage.
It includes construction, sink output and cleanup. Both paths produce the exact
source bytes before timing. These are medians of three passes, with inter-pass
MAD, on CPU thread 4 of a Ryzen 9 7900X. Frequency was unverified and several
cases had high within-pass variation; treat this as exploratory evidence.

| Image | Rows per strip | Input window | Legacy µs | Admitted µs |
|---|---:|---:|---:|---:|
| 1024×1024 | 16 | 1024 | 338.4 ±20.0 | 295.3 ±11.8 |
| 1024×1024 | 16 | 65536 | 381.6 ±3.9 | 361.0 ±15.4 |
| 1024×1024 | 1 | 1024 | 535.9 ±12.7 | 460.0 ±3.3 |
| 1024×1024 | 1 | 65536 | 530.4 ±82.2 | 444.0 ±31.4 |
| 256×256 | 4 | 1024 | 43.1 ±1.3 | 38.1 ±9.0 |
| 256×256 | 4 | 65536 | 58.4 ±17.6 | 40.4 ±5.9 |

The admitted implementation replaces growing input segments with fixed payload
and range-index allocations. These measurements do not isolate accounting cost
or establish a native codec speedup. The path remains opt-in; its fixed input
allocation and 64-span limit are meaningful tradeoffs.

`run-heavy: done rc=0 89s | peak-RSS 0.42GiB | min-avail 17396MiB | peak-load 2.68`

That receipt includes the build and benchmark process, not codec working memory.
