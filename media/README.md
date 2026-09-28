# zencodec-media

Experimental animation and video integration for the zen codec ecosystem.

This experimental workspace lives in the zencodec repository and validates
media interfaces independently of the established image API. It is not yet a published or stable API.

The working foundation includes checked native component planes, exact rational
timestamps, bounded sequential AV1 IVF input/output, and owned rav1d frame
mapping, plus checked native YUV reconstruction with explicit chroma siting and
crop phase. Incremental native AV1 encoding supports bounded input queues,
exact presentation timestamps, cooperative cancellation and non-seekable output.
The corpus contains 48 original lossless AV1 files (192 presentations)
with every decoded sample independently checked, plus 219 reconstruction,
720 display-light, 576 primary-matrix, and 352 forward-quantization reference cases. [Provenance and source audit](DATA_PROVENANCE.md).

```sh
bash scripts/check.sh
```

Dependencies are pinned to the coordinated PR commits, so this command does not
require sibling repositories or locally published crates. To test all the
changes as ordinary checkouts together, start from a clean committed checkout:

```sh
python3 scripts/checkout_series.py ~/tmp/zencodec-media-series-check --check
```

The destination must not exist. The runner creates isolated ordinary clones and
a local Cargo patch configuration; it preserves existing working directories.
[The manifest](integration/repos.lock.json) records the exact PR revisions.

Exact timestamp extraction and bounded HTTP range input are implemented; see
[seeking and network contracts](docs/SEEKING.md).

Explicit display conversion, packed RGB-to-native components, bounded input
snapshots, and drift-free animation clock adaptation are implemented. See
[color contracts](docs/COLOR.md) and [input/output requirements](docs/INPUT_OUTPUT.md).
GIF, APNG, WebP, JPEG XL, and AVIF animation transcodes have executable
all-pairs coverage, including exact timing, loops, transparency, and native
high-precision color where each format supports it. See the
[implementation ledger](docs/IMPLEMENTATION.md) for verified behavior and work
remaining; reference math alone is not a claim that those paths are supported.

The `media` example extracts PNG frames from local or HTTP-backed AV1 IVF and
transcodes the five animation formats. `transcode` preserves native decoded
precision and rejects unsupported destinations; `transcode-srgb` explicitly
converts current ICC/CICP color to straight RGBA8 sRGB. See
[executable workflows](docs/WORKFLOWS.md) for commands and limitations.
