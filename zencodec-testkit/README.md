# zencodec-testkit [![CI](https://img.shields.io/github/actions/workflow/status/imazen/zencodec/ci.yml?style=flat-square&label=CI)](https://github.com/imazen/zencodec/actions/workflows/ci.yml) [![crates.io](https://img.shields.io/crates/v/zencodec-testkit?style=flat-square)](https://crates.io/crates/zencodec-testkit) [![lib.rs](https://img.shields.io/crates/v/zencodec-testkit?style=flat-square&label=lib.rs&color=blue)](https://lib.rs/crates/zencodec-testkit) [![docs.rs](https://img.shields.io/docsrs/zencodec-testkit?style=flat-square)](https://docs.rs/zencodec-testkit) [![license](https://img.shields.io/crates/l/zencodec-testkit?style=flat-square)](#license)

The testkit versions independently from zencodec and may make breaking changes
to implementor tests. The upcoming 0.2.0 requires zencodec 0.2.0; construct
reference decode jobs through the config's `job()` method instead of the former
`RefDecodeJob` unit value. See the [changelog](https://github.com/imazen/zencodec/blob/main/CHANGELOG.md#zencodec-testkit).

Conformance test harness for [`zencodec`](https://crates.io/crates/zencodec) codec
implementations. A codec crate (`zenjpeg`, `zenpng`, `zenwebp`, …) adds this as a
`dev-dependency` and runs the checks against its own `EncoderConfig` /
`DecoderConfig` to verify it honors the shared contract.

The checks target the parts of the contract that are easy to get subtly wrong and
expensive to ship wrong:

- **Metadata retention (privacy).** Encoding with a [`MetadataPolicy`] must never
  leak what the policy discards. `check_metadata_no_leak` encodes with a policy,
  decodes the result, and parses the embedded EXIF back to assert that
  GPS/camera/timestamp tags the policy dropped are *actually gone* — not merely
  absent from a struct field.
- **Cross-path pixel equivalence.** A codec usually offers several feeding modes
  (one-shot `encode`, incremental `push_rows`, pull `encode_from`; one-shot
  `decode`, `streaming_decoder`, `push_decoder`). They must all produce identical
  pixels. `check_cross_path_pixel_equivalence` runs every advertised path and
  diffs the results byte-for-byte. `check_animation_cross_path_equivalence` does
  the same for the three animation decode paths (borrowed, owned, push-sink),
  which is where canvas-aliasing and frame-ordering bugs hide.
- **Capability honesty.** `EncodeCapabilities` / `DecodeCapabilities` are
  load-bearing — callers branch on them. `check_capability_honesty` exercises
  them comprehensively, in both directions: every declared capability
  (`push_rows`, `encode_from`, animation, streaming, `lossless`, `cheap_probe`,
  the `icc`/`exif`/`xmp`/`cicp` metadata channels, `native_alpha`) must actually
  work, and every undeclared optional path must decline with `UnsupportedOperation`
  rather than panicking or silently succeeding. All violations are reported
  together, so one run names every dishonest flag. (Cancellation and the lossy
  flag are out of scope — see the function docs for why.)
- **Orientation.** `check_orientation_roundtrip` asserts an EXIF orientation
  survives a keeping policy exactly once — no loss, no double-application.
- **Buffer colour context.** Decoders should attach a class-gated
  `zenpixels::ColorContext` to the buffers they emit so the pixels are
  self-describing. `check_color_context_consistency` asserts whatever is attached
  is class-valid (an ICC only rides a layout its device class describes) and
  identical across the one-shot, streaming, and animation (borrowed + owned)
  paths — strip/scratch buffers are where a context silently dies.
  `check_color_context_attached` (opt-in) is the strict positive direction for
  codecs that have adopted the convention.
- **Native HDR.** When both ends declare `hdr`, `check_native_hdr_roundtrip`
  proves BT.2100 PQ and HLG survive a `PreserveExact` round trip: the CICP, the
  HDR envelope (content light level + mastering display, within container
  fixed-point precision), 16-bit pixels where both ends are natively 16-bit, and
  a decoded buffer *labelled* PQ/HLG rather than sRGB. Phase 0 of the
  gain-map/HDR delivery scope — prove native HDR before adding any gain-map
  encode surface.
- **Gain maps.** `check_gain_map_roundtrip` is the encode/decode contract: an
  encoder without `gain_map` must reject `with_gain_map_pixels` /
  `with_gain_map_encoded` with `UnsupportedOperation::GainMapEncode` (a dropped
  gain map is lost HDR, so silence is the one forbidden outcome); a declared
  encoder embeds 1-channel, 3-channel and backward-direction sub-resolution maps
  and rejects a foreign-format encoded one; a declared decoder reports the map
  from `probe()`, surfaces it only on request (`Components` /
  `with_extract_gain_map`, never under the default `BaseOnly`) with exact ISO
  21496-1 params, falls back honestly on `ReconstructHdr` without
  `reconstructs_hdr`, and the surfaced map transcodes through a fresh
  `with_gain_map_pixels` unchanged.
- **Sans-I/O decoding.** For decoders that declare `incremental`,
  `check_incremental_contract` drives the erased decoder with 1-, 7- and 64-byte
  windows and the whole file and requires the one-shot pixels, `begin` once,
  final rows once each in order, `Done` repeating, unrequested input ignored,
  cancellation within one step and then `Finished`, final rows through
  `provide_next_buffer` even for a sink that accepts provisional rows, and a
  result matching one-shot on truncated input: identical pixels where one-shot
  succeeds, a codec error (never a protocol violation or a hang) where it fails.
  `check_seekable_capability_honesty` covers `push_decoder_from_source`, and
  `check_read_at_contract` checks a `ReadAt` source.
- **Streaming encode output.** `check_streaming_output_honesty`: an encoder
  that declares `streaming_output` has bytes pending before `finish` once
  `begin` has run, draining them leaves the file byte-identical, and the result
  decodes to the `push_rows` pixels; an undeclared one keeps `pending_output`
  empty.
- **Error classification.** `check_decode_truncation_series` feeds a
  deterministic series of truncated prefixes of a known-good image and requires
  every failure to classify as *incomplete input* (never `Internal`, OOM, or an
  I/O error) without panicking. `check_decode_error_envelope` /
  `assert_uses_codec_error_envelope` verify a codec's `ErrorCategory` and codec
  name survive dyn-dispatch type erasure (both opt-in, not part of `check_all`).

The crate ships `reference`, a faithful in-memory codec that round-trips pixels
*and* metadata and declares its supported capabilities while declining unsupported
paths — the harness is validated
against it in this crate's own tests, and it doubles as a worked example of
implementing the traits. (An internal `minimal` codec — one-shot only, every
optional capability declared false — exercises the false-direction branches.)

[`MetadataPolicy`]: https://docs.rs/zencodec/latest/zencodec/enum.MetadataPolicy.html

## Usage

```rust,ignore
use zencodec_testkit as tk;

#[test]
fn my_codec_is_conformant() {
    // Run the default checks; add applicable fixture-driven checks below.
    tk::check_all(MyEncoderConfig::new(), MyDecoderConfig).expect("conformance failed");
}

#[test]
fn my_codec_privacy() {
    // Or call individual checks for control over image sizes / frames.
    let img = tk::TestImage::rgba8_gradient(64, 48);
    tk::check_metadata_no_leak(MyEncoderConfig::new(), MyDecoderConfig, &img)
        .expect("metadata leaked past the policy");
}
```

## License

Apache-2.0 OR MIT.

## Explicit contract fixtures

The fixture-driven checks `check_animation_timing`, `check_animation_durations`,
`check_encode_policy_admission`, `check_decode_policy_admission`,
`check_inventory_input_limit`, and `check_metadata_rewrite` supplement `check_all`.
They require caller-provided expected outcomes: format clocks, supported policy
fields, and rewrite output observations. A successful setter is not proof of
metadata delivery. Rewrite observers should inspect encoded payloads and use an
independent decoder/parser as well as the codec under test.

Animation sink results currently carry no duration or frame index; the testkit
cannot verify information the API does not expose. Coverage is tracked in
[the contract matrix](https://github.com/imazen/zencodec/blob/main/docs/testkit-contract-coverage.md).

## Coverage boundary

`check_all` is the default suite, not complete certification. Use the
[current coverage audit](https://github.com/imazen/zencodec/blob/main/docs/testkit-release-audit.md)
to select additional fixture-driven, native, media and application checks.
Incremental conformance requires pre-cancellation (including timeout) before
sink effects and terminal state afterward, even for tiny inputs. Diagnostic
Pulse controls in `tests/sansio_checks.rs` detect swallowed stops without
asserting machine-dependent timing thresholds. Native checkpoint latency and
progress completeness still require explicit workloads and expected-work oracles.
