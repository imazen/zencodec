# Native spatial contract experiments

This is an executable candidate, not a public spatial API. It selects a main
TIFF page, follows an explicit path of reduced SubIFDs, and returns a region in
stored coordinates. Native sample bytes and orientation remain unchanged.
TIFF parsing and coding-unit decoding come from the pinned native codec.

Missing qualification: full color/ICC interpretation, auxiliary-image dependencies,
whole-container privacy, live HTTP version binding, total native workspace bounds,
cross-unit batched scheduling, wide samples and strips. The current external
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

## API implications

`ReadAtReader` is a small synchronous `Read + Seek` bridge with its own cursor.
It adds no prefetch, retry or async suspension. End-relative seeks call `len`,
which may consume a sequential source. A file, range cache or HTTP adapter owns
those policies; adapting a parser does not make the parser resumable.

Pages, reduced levels, stored regions, and coding units have separate identities.
An arbitrary SubIFD is not automatically a pyramid level. An output-byte bound
does not bound metadata, native workspace or transport staging. The eventual
shared request needs executable choices and the existing policy resolver;
this experiment's mechanical cancellation check is not a second policy system.
