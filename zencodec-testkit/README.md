# zencodec-testkit

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

The crate ships two codecs the harness is validated against in this crate's own
tests, both worked examples of implementing the traits: `reference`, a faithful
in-memory codec that round-trips pixels *and* metadata and declares/honors every
capability; and `minimal`, its opposite — one-shot only, declaring every optional
capability false — which exercises the false-direction branches.

[`MetadataPolicy`]: https://docs.rs/zencodec/latest/zencodec/enum.MetadataPolicy.html

## Usage

```rust,ignore
use zencodec_testkit as tk;

#[test]
fn my_codec_is_conformant() {
    // One call runs every check with default inputs.
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
