# Native spatial contract experiments

This is an executable candidate, not a public spatial API. It selects a main
TIFF page, follows an explicit path of reduced SubIFDs, and returns a region in
stored coordinates. Native sample bytes and orientation remain unchanged.
TIFF parsing and coding-unit decoding come from the pinned native codec.

Missing qualification: full color/ICC interpretation, auxiliary-image dependencies,
whole-container privacy, live HTTP version binding, total native workspace bounds,
cross-unit batched scheduling, wide planar samples and independent wide-sample
readback. The current external
oracle covers the first classic RGB8 page, with both chunky and planar tiles;
it does not establish those missing contracts.

## Run

Use an export outside workspace path overrides. Install the oracle in an isolated
Python environment using `just spatial-oracle-install <python>`, then:

```sh
just spatial-validation-setup
run-heavy --mem 12G --jobs 4 -- timeout --kill-after=15s 5m \
  just spatial-validation-check <fresh-persistent-artifact-dir> <python>
```

Both oracle and artifact arguments are required. Missing tools fail the gate.
Each external oracle process also has a 30-second deadline. The harness uses
Pillow 13.0.0 with libtiff explicitly selected, checks that selection, and compares
its full-page bytes against the original fixture samples. The native encoder
writes the fixture grammar. No TIFF parser or compression implementation is
duplicated here.

## Measured coverage, 2026-10-25

The initial matrix covers 24 combinations: classic/BigTIFF, chunky/planar,
direct/4096-byte `BufReader`, and two independent pages plus one reduced SubIFD.
A 6×5 region at (13,12) crosses four 16×16 tiles, including the reduced image's
partial right/bottom tiles. Every returned sample matches the original raster.
The selected SubIFD also agrees with the native full-image route.

Fault cases cover partial writes followed by read errors, source replacement,
final cancellation, invalid reduced-image roles, discovery/output/unit/tag
allowances, and cursor/seek overflow. Concrete source errors survive TIFF,
standard I/O and framework error inspection. Output stays private until success;
an earlier successful result remains valid after a later failure.

Actual call and byte counts are in
[`spatial_tiff_reads_2026-10-25.tsv`](../../benchmarks/spatial_tiff_reads_2026-10-25.tsv).
For classic chunky page 0, buffering changes 68 reads / 3,020 bytes into
9 reads / 34,390 bytes. For its planar reduced SubIFD, the change is 229 reads /
2,228 bytes into 21 reads / 84,038 bytes. These are source-call counts and bytes
copied, including rereads. No disk throughput, network latency or codec CPU
performance was measured. A fixed `BufReader` alone is not evidence of a good
network plan; byte admission and round trips must be considered together.

Command on r7900x, Rust 1.99.1, isolated export with committed dependency lock:

```sh
run-heavy --mem 12G --jobs 4 -- timeout --kill-after=15s 5m \
  env TMPDIR=$HOME/tmp \
  CARGO_TARGET_DIR=$HOME/tmp/zencodec-io-experiments-2026-10-24/target \
  just spatial-validation-check \
  /mnt/v/output/zencodec/spatial-2026-10-25/run4 \
  $HOME/tmp/spatial-oracle-2026-10-25/bin/python
```

Tests and strict all-target Clippy passed. Full local log:
`~/tmp/zencodec-spatial-tiff4-2026-10-25.log`.

```text
run-heavy: done rc=0 1s | peak-RSS 0.19GiB | min-avail 16698MiB | peak-load 0.61
```

That process-tree peak includes compilation, tests, oracle and Clippy. It is not
a codec workspace measurement. Artifacts remain in the directory above with
`SHA256SUMS`; independent readback recorded Pillow 13.0.0 / libtiff 4.7.2.

## Strip and sample extension

`strip_tests.rs` adds 22 native layouts: Gray unsigned/signed 8/16/32/64 and
float 32/64; RGB/RGBA unsigned 8/16/32/64 and float 32/64. All run with classic
TIFF and BigTIFF, uncompressed/PackBits/LZW/Deflate, and no predictor versus the
applicable horizontal or floating-point predictor. Each checks a middle region,
the final partial strip and the complete image against original sample bits.
All eight orientations are reported without transforming stored coordinates.
Float fixtures include signed zero, infinities, subnormals and NaN payloads;
comparison is byte exact, including payloads. This is native fixture parity,
not an independent wide-sample oracle.

The same full gate passed with artifact argument `run5`. Full local log:
`~/tmp/zencodec-spatial-wide2-2026-10-25.log`.

```text
run-heavy: done rc=0 3s | peak-RSS 0.37GiB | min-avail 16559MiB | peak-load 0.45
```

This includes compilation and the original external RGB8 gate. GrayAlpha,
half-float TIFF, wide planar tiles and opposite-endian files remain unqualified.

## API implications

`zencodec_io::ReadAtReader` is the shared synchronous `Read + Seek` bridge used
by the native candidate, with its own cursor.
It adds no prefetch, retry or async suspension. End-relative seeks call `len`,
which may consume a sequential source. A file, range cache or HTTP adapter owns
those policies; adapting a parser does not make the parser resumable.

Pages, reduced levels, stored regions, and coding units have separate identities.
An arbitrary SubIFD is not automatically a pyramid level. An output-byte bound
does not bound metadata, native workspace or transport staging. The eventual
shared request needs executable choices and the existing policy resolver;
this experiment's mechanical cancellation check is not a second policy system.

The promoted bridge repeats this entire native matrix (`run6`, log
`~/tmp/zencodec-spatial-shared-bridge1-2026-10-25.log`): rc=0, 3 s, peak RSS
0.40 GiB, minimum available 16526 MiB, peak load 0.51 under `run-heavy`.
The I/O crate's full all-feature tests, strict Clippy and no-default-feature
check also pass through `just io-reader-check`. Direct native I/O values keep
their kind, OS code and payload; generic errors keep their concrete context.
Additional controls cover invalid counts, failed end-relative spool discovery,
EOF, standard buffered reads and no bridge-level retry on `Interrupted`.
