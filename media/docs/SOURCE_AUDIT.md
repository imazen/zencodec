# Source audit and rejected assumptions

The original design document is a question list, not an implementation contract.
These corrections come from backend source and executable checks. Repository
revisions used by the integrated build are in `integration/repos.lock.json`.

| Assumption | Source finding and resulting contract |
|---|---|
| An image decoder accepts seekable input because its backend does | The shared image trait accepts bytes. The experimental layer provides bounded byte collection and immutable spill-to-disk snapshots. Its AV1/IVF decoder instead consumes `Read`; the presentation index requires `Read + Seek`. Each adapter must advertise its actual buffering. |
| A decoded video frame must borrow the decoder | rav1d pictures own reference-counted storage; mapped plane guards can own their frame lifetime. Managed frames remain valid across later decoding and decoder destruction. No unsafe lifetime extension is needed. |
| AV1 color configuration changes in a frame header | AV1 `color_config()` belongs to the sequence header. Each emitted frame retains its applicable sequence metadata; do not invent a frame-header color field. See the [pinned AV1 syntax](https://github.com/AOMediaCodec/av1-spec/blob/5e04f3f75e73a5898d7616c47c52f032144b8f80/06.bitstream.syntax.md). |
| Every visible keyframe is a complete random-access point | Sequence initialization, decode dependencies, hidden frames, and `show_existing_frame` matter. The index records display order and actual keyframe provenance. Extraction initializes fresh decoder state and reports actual timestamp/ordinal and decode work; a keyframe flag alone is not an access guarantee. |
| Integer microseconds necessarily accumulate playback drift | Independent rounding of every duration can accumulate drift. Rounding cumulative endpoints bounds that error; this layer keeps rational timing and makes any destination-clock quantization explicit. |
| `u32` API clock fields imply every `u32` value is encodable | JPEG XL clocks use numerator 1..=2^30 and denominator 1..=1024. The native encoder now rejects overflow; the adapter factors tick periods within those bounds before accepting frames. Verified against [libjxl 0.12 header serialization](https://github.com/libjxl/libjxl/blob/a7a9c787341cf703dede03c2009fa460cae5e5df/lib/jxl/headers.cc) and [encoder PR #125](https://github.com/imazen/jxl-encoder/pull/125). |
| A requested AVIF depth proves the output depth | The native animation path ignored requested 12 bits and emitted 10. Tests now inspect bitstream/container signaling and compare every authored RGB/alpha code after decoding. [Native fix](https://github.com/imazen/cavif-rs/pull/7). |
| GIF transparent pixels erase prior content | On the wire they skip writes. Full-canvas input needs disposal chosen with one-frame lookahead. Transparent background disposal also needs a transparent index in the disposed frame: [FFmpeg decoder source](https://github.com/FFmpeg/FFmpeg/blob/d62aef2e50434cc31eccf236ae67d864becb7068/libavcodec/gifdec.c). The command-level corpus oracle catches missing signaling independently of zengif's compositor. |
| Animation frames returned by every adapter are already composed | APNG previously returned raw rectangles through the full-canvas trait. The replacement compositor applies blend/disposal, including across skipped displays, at native 8/16-bit precision. [PNG PR #21](https://github.com/imazen/zenpng/pull/21). |
| SMPTE 240M can use the BT.709 transfer enum | H.273 transfer code 7 has different coefficients and breakpoints. zenpixels 0.3 must preserve it as unsupported/unknown until a real implementation exists. Raw CICP remains available. [H.273 July 2024, table 3](https://www.itu.int/rec/dologin_pub.asp?id=T-REC-H.273-202407-I%21%21PDF-E&lang=e&type=items). |
| ICC bytes imply lossy input has already been transformed | ICC embedding and source transfer interpretation are separate operations. Native JPEG XL animation previously dropped ICC in lossless mode and used sRGB preparation for some declared non-sRGB integer inputs. The request path now shares still-image preparation. [Encoder PR #124](https://github.com/imazen/jxl-encoder/pull/124). |
| Alpha attachment preserves the decoded descriptor | AVIF typed-buffer allocation lost transfer signaling after adding alpha. Pixel-side ColorContext also retained a consumed YCbCr matrix/range. Native final descriptors are now stamped after conversion, with RGB/full-range context and raw source provenance kept separately. Two executed negative controls and the cross-format corpus found this. [AVIF PR #51](https://github.com/imazen/zenavif/pull/51). |

## Precision and color boundaries

`SampleEncoding` describes current integer code bits and placement in a storage
word. It does not describe source precision, transfer, matrix, signal range, or
alpha association. Packed image U16 retains zenpixels' full 0..65535 contract;
native 10/12-bit planes are explicitly tagged codes. A conversion must choose
the normalization and quantization, rather than relabeling a buffer.

YUV reconstruction returns source-encoded RGB. Display conversion is a separate
operation with explicit luminance assumptions. HLG requires a coupled OOTF and
display parameters; PQ uses absolute luminance. Chroma siting, matrix, clipping,
alpha handling, tone mapping, and gamut policy are independent choices. Unknown
claims stay unknown. See [conversion evidence](COLOR.md).

## Evidence boundaries

The synthetic corpus compares against authored pixels and independently computed
reference values. Encode/decode agreement alone is insufficient: the GIF wire
oracle and the libjxl 0.12 sample checks specifically guard shared mistakes.
Test counts, ignored external corpora, buffering, and cancellation granularity
must remain separate claims. A borrowed token checked between native packets is
not evidence of cancellation inside a codec kernel.

## Metric input is an explicit numerical contract

Source read at zenmetrics `0a61830db89a768ab9462f166d1412e948e4efa9`:
`crates/zenmetrics-api/src/metric.rs` and `hdr.rs`. The CPU arm of
`Metric::compute_pixels` converts to packed sRGB8; its convenience API is not
a general HDR-preserving media ingress. The dedicated
`compute_pu_nits_interleaved_multi` accepts absolute cd/m² for the metrics
that implement that feeding. `compute_from_linear_interleaved` has different,
metric-specific scaling. These entries must not be interchanged just because
both accept `&[f32]`. The new media display conversion spells out luminance,
primaries, and transfer rather than relying on automatic metric conversion.
No metric training/model coefficients or established comparison goldens were
changed for this corpus.
