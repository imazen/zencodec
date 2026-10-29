# Collector completion regression

Base eae62eea allowed finish-before-begin, allowed completion after supplying
the same row repeatedly while other rows were absent, and exposed writable
buffers after finish. Three independent controls fail against that base; all
pass with coverage tracking and terminal-write rejection. The missing-coverage
control spans the bitmap boundary at height 65 and preserves out-of-order and
overlapping delivery.

`just collector-coverage-check`: all zencodec-io feature tests/doctests and strict
all-target Clippy pass. Existing assertions remain unchanged. Isolated before
and after exports; full logs retained. Serialized run-heavy 3G/jobs2 timeout900:
rc0/7s, peak RSS 0.30GiB, minimum available 13645MiB. This is validation overhead,
not a controlled throughput or native allocation measurement.

Coverage payload is eight bytes per 64 rows, rounded up, allocated fallibly
before installing sink state. It is separate from raster storage. This gate
does not prove a decoder writes every byte it borrows or verify native metadata
completeness. No public signature changed.

Native follow-through: `just jpeg-decode-admission-check` passes seven native
JPEG baseline/progressive negotiation and completion-owner controls, including
exact rows, metadata/extras and retained pixel pointer identity. Strict harness
Clippy passes. Same isolated after-source, committed dependency lock unchanged;
run-heavy 3G/jobs2 timeout900: rc0/12s, peak RSS 0.70GiB,
minimum available 13243MiB. No native throughput or updated heap profile claimed.
