# Reimagined codec contract: retained documents

An unpublished design candidate. The existing codec API remains under the
repository root. This standalone workspace derives from the reimagined source
capture recorded in [source-capture.json](docs/source-capture.json).

The proposed split is **document discovery → exact selection → the existing
pixel decoder**. A document retains its index and source binding while callers
browse pages, native levels and channels, then request repeated regions. Pixel
reconstruction continues through `Decoder::step` and `Sink`; selection does not
introduce another typed/erased trait hierarchy.

Read [the design and coverage ledger](docs/document-design.md) for the comparison
with main and PR208/209, what executes, and what still needs qualification.
The [driver example](zencodec-io/src/document.rs) shows the caller's path.

From this directory:

```sh
just test
just clippy
just check-core
just docs
just tiff-test
just tiff-clippy
```

Use the normal resource wrapper and an isolated `CARGO_TARGET_DIR` when running
builds on a shared host. Set `TMPDIR` to a disk-backed directory before the native
TIFF tests. They require ImageMagick with its libtiff delegate, and preserve their
small authored TIFF fixtures there. The native tool is outside the three-crate
workspace and is not a shipped dependency.

The synthetic controls cover discovery and exact sample selection. The native
witness covers one uncompressed tiled Gray8 TIFF with two root directories and
a reduced SubIFD. EXR, JXL and IMIJ have no native adapter in this candidate.
See [validation](docs/validation-2026-11-11.md) for receipts and failed attempts.

[The ablation evidence ledger](docs/ablation-evidence-2026-11-11.md) identifies
which older responsibilities remain necessary, which mechanisms can be
consolidated, and what the paired driver/heaptrack study actually establishes.

The [readable API and cost comparison](docs/resident-input.html) includes the
native RGB8 QOI/BMP/JPEG trial and the `&[u8]` / `&dyn Pulse` cost control.
Allocation parity is established for that control; both timing studies remain
inconclusive. The boxed protocol and WholeImage Pulse hook still need API work.
