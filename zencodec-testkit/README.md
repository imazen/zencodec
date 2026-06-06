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
  diffs the results byte-for-byte.
- **Capability honesty.** `EncodeCapabilities` / `DecodeCapabilities` are
  load-bearing — callers branch on them. `check_capability_honesty` confirms a
  declared capability actually works and an undeclared one cleanly returns
  `UnsupportedOperation` rather than panicking or silently misbehaving.

The crate ships a `reference` codec — a faithful in-memory codec that round-trips
both pixels and metadata — that the harness is validated against in this crate's
own tests, and which doubles as a worked example of implementing the traits.

[`MetadataPolicy`]: https://docs.rs/zencodec/latest/zencodec/enum.MetadataPolicy.html

## Usage

```rust,ignore
use zencodec_testkit as tk;

#[test]
fn my_codec_is_conformant() {
    let img = tk::TestImage::rgba8_gradient(64, 48);
    tk::check_cross_path_pixel_equivalence(MyEncoderConfig::new(), MyDecoderConfig, &img)
        .expect("cross-path pixels diverge");
    tk::check_metadata_no_leak(MyEncoderConfig::new(), MyDecoderConfig, &img)
        .expect("metadata leaked past the policy");
}
```

## License

Apache-2.0 OR MIT.
