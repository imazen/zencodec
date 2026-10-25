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

Workspace controls exercise allocation-count and byte-cap contention across
threads, failed-admission rollback, arithmetic overflow, unwind cleanup, nested
subviews, empty views, packet side data, old-epoch frame retention and exact
strided 8/10-bit samples. These establish ownership and admission mechanics;
they do not measure native codec memory. Run `just io-contract-check`.

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
