# Media validation provenance

This corpus is **validation only**. It contains original synthetic samples and
analytic values, with no downloaded artwork, photographs, or training images.
It is not a metric training set. Do not mix future metric outputs from different
codec, conversion, display, or scorer revisions without recording those inputs.

## Native AV1 corpus

`scripts/generate_av1_corpus.py` produces 48 lossless files covering 8/10/12-bit,
mono/4:2:0/4:2:2/4:4:4, full/narrow range, 32×24 and 17×13 dimensions. Each has
four frames at the exact 1001/30000-second clock and a two-frame keyframe period.
The pattern is deterministic integer arithmetic, including code endpoints and
ceiling chroma dimensions. Values and commands are in the generator and
`corpus/av1-manifest.json`.

Each current file is encoded with native zenrav1e and decoded with native
rav1d-safe in strict mode. Generation fails unless every decoded code equals the
original source in both the Rust generator and an independent Python generator.
The schema-2 manifest records the clean source commit, generator source hashes,
compiled binary hash, Rust toolchain, exact locked codec sources, encoder
settings, geometry, timing, color, and source/decoded/encoded SHA-256 hashes.

The initial historical corpus at `976aaa7` used libaom/FFmpeg and libdav1d. Its
manifest and bytes remain in git history and were preserved locally before
replacement. The native regeneration does not relabel those older artifacts.
No external codec executable is needed for current generation or checks.

`--check` verifies committed bytes and independently regenerated source hashes.
`tests/av1_corpus.rs` compares native rav1d output against mathematical source
samples and verifies storage, range, timing, chroma dimensions, visible rows,
fragmented input, cancellation, and owner/guard lifetimes.

## Analytic conversion references

`scripts/generate_reference_vectors.py` uses Python Decimal at 70-digit precision.
`corpus/reference-vectors.csv` has 219 cases: full/narrow NCL YCbCr at 8/10/12/16
bits with BT.709, BT.601 and BT.2020 coefficients, and sRGB, PQ and inverse HLG
OETF samples. The independent Rust f64 reference is in `zencodec-media-testkit`; the
production crate must never call that reference implementation.

YCbCr reference outputs are source-encoded RGB without clipping. PQ output is
absolute cd/m². Inverse HLG OETF output is scene-relative light, **not** display
nits; a display rendering needs additional assumptions. Alpha source-over in
the reference operates in an explicitly caller-selected numerical domain.

Additional independent Decimal generators record their code beside the vectors:
`generate_display_references.py` emits 720 display-light and 576 primary-matrix
cases; `generate_encode_color_references.py` emits 352 exact integer forward
quantization cases. Primary transforms are solved by Gauss-Jordan elimination
from chromaticities, independently of the production closed-form inverse.
Display curves include nonzero black and colored HLG, not only neutral ramps.
The production matrix test initially rejected the subtract-rounded-luma formula
at saturated-yellow's half-code boundary; the difference-form implementation
passes without changing golden values or permitting an extra code of error.

Reference sources:

- [ITU-R BT.2100](https://www.itu.int/rec/R-REC-BT.2100): PQ and HLG definitions.
- [Khronos Data Format 1.4](https://registry.khronos.org/DataFormat/specs/1.4/dataformat.1.4.html): transfer definitions and numerical constants.
- [AV1 bitstream semantics](https://github.com/AOMediaCodec/av1-spec/blob/5e04f3f75e73a5898d7616c47c52f032144b8f80/07.bitstream.semantics.md): color, chroma position and metadata units.

## Upstream source audit, 2026-09-27

Fresh shallow checkouts were read, rather than treating an earlier design draft
as evidence. Source revisions used for the initial container/decoder audit:

| Project | Commit | Relevant files |
|---|---|---|
| AV1 specification | `5e04f3f75e73a5898d7616c47c52f032144b8f80` | `07.bitstream.semantics.md` |
| dav1d | `bf5a8792744ee78c977dfb16503f9156dde6401d` | `include/dav1d/dav1d.h`, `headers.h` |
| FFmpeg | `d62aef2e50434cc31eccf236ae67d864becb7068` | `libavformat/ivfdec.c`, `ivfenc.c` |
| libvpx | `5e680f30801d03c21078f8c4b772464752516211` | `ivfenc.c`, `ivfdec.c` |

The IVF implementation preserves the declared time base; it does not copy old
libvpx frame-rate guessing. It accepts zero and all-ones unknown frame counts
(the latter is FFmpeg's nonseekable-output placeholder). Signed timestamp bytes
match the libvpx/FFmpeg APIs. The rav1d adapter explicitly rejects a known
`i64::MIN` timestamp because that backend reserves it for unknown time; the
generic timestamp and IVF types themselves preserve the complete i64 domain.

Codec revisions for the coordinated implementation are pinned by the integration
manifest. Large future assets belong outside git with hashes and retrieval
instructions; the small committed corpus is sufficient for the default tests.

## Native animation corpus

`corpus/native-animation/manifest.json` records all thirty encoded files with
SHA-256 hashes, dimensions, exact duration fractions, total plays and decoded
visible-pixel hashes. `generate_animation_corpus.py --check` regenerates them
with the pinned native codecs and requires byte-identical files and manifest.
Original integer-authored patterns cover three geometries and binary alpha;
no external encoder or downloaded asset is used. High-precision and ICC tests
create their distinct inputs in memory and do not claim those cases are in the
committed 8-bit corpus. The ICC test compares routing against a direct MoxCMS
call; it is not an independent color-science oracle.

## MP4 demux fixtures

`corpus/mp4/` holds two byte-identical-content 1-second samples generated by
FFmpeg 8.0.1 (`testsrc2` 160×96@15fps video + 440 Hz sine at 44.1 kHz mono,
libx264 + native AAC; 15 video samples, 45 audio packets).
`h264_aac_faststart.mp4` places `moov` before `mdat` (`-movflags +faststart`);
`h264_aac_moovlast.mp4` uses the default layout, exercising the seek path.
Both carry b-frame reorder (`ctts`) so pts ≠ dts on at least one sample.
They are validation fixtures only, exercising table parsing and exact packet
accounting — no artwork. SHA-256:

- `h264_aac_faststart.mp4` / `h264_aac_moovlast.mp4`: recorded in git (small,
  committed binary fixtures).
