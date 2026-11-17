# zenpixels PR 160 adaptation

[zencodec PR 212](https://github.com/imazen/zencodec/pull/212) now targets
[zenpixels PR 160](https://github.com/imazen/zenpixels/pull/160) at
`e22e4136daed102379881d09c81e7dd9436087b3`, enabling `features = ["icc", "planar"]`
now that `zenpixels` gates planar carriers off by default. It remains stacked on
[PR 211](https://github.com/imazen/zencodec/pull/211). The earlier
[PR 159 qualification](zenpixels-159.md) is historical.

## What changed upstream

| PR 160 change | Effect on this contract |
|---|---|
| Generic CMYK/CMYKA and explicit ink polarity | Complete descriptors already cross the interface; polarity cannot be substituted without a declared conversion. |
| Parametric profiles with custom primaries, white point, transfer curve and rendering intent | `Raster::of` and owned-frame delivery retain the full `ColorContext` Arc. No ICC synthesis or CICP reconstruction is needed to carry these facts. |
| Perceptual/polar models and additional plane roles | Carriers can describe them. Each codec still admits only the formats it implements. |
| Compact pixel/sample representations | Public carriers compile unchanged. No timing or process-memory improvement is claimed here. |

The three prototype crates and next-side native/synthetic probe types share the
new pin and reviewed locks. The older comparison cohort stays frozen. No codec
trait signature, source borrowing rule or production carrier implementation
needed to change. Local control remains `Option<&dyn Pulse>` and worker control
remains `Option<SharedPulse>`.

## Compatibility evidence

Migration source: `c19db8bf57ca41a32e04aeb6f464b35706dfae28`.
Qualified tests: `24a20afd51a771756968940417348178257b668d`.

`zencodec-io/tests/pixel_color_models.rs` exercises existing contract code:

- Custom RGB primaries, a custom white point and gamma, DCI-P3, and ProPhoto
  survive `Raster::of` and owned collector delivery with the original sample
  allocation and context Arc. Borrowed `PixelEncoding` preserves the parametric
  profile and intent without retaining another Arc.
- Conventional/inverted U8 CMYK and five-channel CMYKA preserve every active
  sample through split strided row delivery, including a final row without
  trailing padding. The frame header retains opaque ICC and explicit intent.
- Required opposite polarity refuses without a declared conversion; preference
  fallback keeps native polarity. `copy_rows` refuses before changing the
  destination. The fixture codec refuses unsupported CMYK and perceptual input.

Existing foundation controls remain enabled: ICC authority/file metadata,
alignment, odd-sized strided planar input, borrowed source windows, optional
Pulse, cancellation and error causes. No established assertion was relaxed.

Local validation passes:

```sh
cd reimagined
just pixel-models gates test-default check-msrv
PROBE_TARGET=/path/to/isolated/output just probe-check
```

This includes strict Clippy/Rustdoc, formatting, WASM and freestanding no_std,
Rust 1.88, and existing native QOI/BMP/JPEG and synthetic probe tests. Probe
tests validate correctness; no benchmark timing run was performed.
All eight standalone CI jobs pass on the qualified test revision:
[run 43839701976](https://github.com/imazen/zencodec/actions/runs/43839701976),
including Windows ARM64, macOS ARM64 and i686 through cross.

CI also exposed a stale zenmeta catalog fingerprint left by the earlier
gain-map fixture maintenance. Regeneration from the frozen root dependency
graph produced byte-identical public declarations and documentation; only the
source fingerprint changed (`057f3ba424519aa1e4ee581f4d028d1d5d08f98b`).
The ownership control, site build and all 27 Node checks pass. The older native
guide dependency-resolution conflict remains outside this adaptation.

Commands, source/lock identities, failed attempts, log hashes and resource lines
are retained in [the receipt](../../benchmarks/zenpixels_pr160_2026-11-16.json).

## Limits

This qualifies declaration and sample carriage, admission, and compilation.
It does not qualify upstream conversion kernels, native file emission of
parametric color or rendering intent, complete native metadata handling, or
every new perceptual model/plane role. The collector retains its existing
U8/U16/F32 planar storage; owned planar delivery and remainder buffering remain
unimplemented. No new native codec support is implied by upstream carriers.

Earlier wall/CPU and allocation receipts keep their original pixel revisions.
PR 160 performance, ROI/tile browsing and concurrent InPlace fairness are not
measured by this update. The older root API retains its existing dependency pin.
