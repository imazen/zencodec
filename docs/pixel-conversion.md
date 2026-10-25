# Pixel conversion in the migration guide

Verified 2026-10-25. The [compiled examples](examples/pixel_conversion.rs) use
real JPEG through the pinned [native harness](../perf/jpeg-policy-validation/Cargo.toml).
`zenpixels` supplies views, stride and interpretation; `zenpixels-convert`
performs conversion; `zencodec` dispatches codecs. Conversion does not belong in
the shared trait crate's default dependency graph.

## Implemented recipes

`decode_for_display` registers a JPEG decoder in `CodecSet`, attaches input
limits, decodes and converts the owned buffer to BGRA8 sRGB. It returns the
residual orientation alongside the pixels. `convert_to` allocates a destination;
it is not an in-place operation or a bounded streaming claim. This path uses
the converter's current color-context handling. Unsupported ICC transforms
remain errors; callers must retain the error rather than relabel the pixels.

`encode_framebuffer` accepts an already rendered BGRA8 sRGB framebuffer with
no attached color context. It rejects unknown interpretation and attached
profiles before conversion. It checks opacity across every row before allowing
alpha removal, prepares a `RowConverter`, and converts into RGB8 before JPEG
encoding. Both input padding and a final row without trailing padding work.
The RGB conversion is exact channel selection; JPEG encoding is lossy.

This deliberately small adapter still materializes a full RGB image. It is a
usable whole-image recipe, not proof that an encoder can release raw storage
while a network writer waits. A production row-to-encoder adapter needs the
encoder's actual pause/release contract and admission for all retained storage.

## Release distinction

On this date crates.io has zencodec 0.1.26 and zenpixels-convert 0.2.16.
`PixelBufferConvertExt::convert_to`, `ConvertOptions` and
`RowConverter::new_explicit` already exist in those conversion sources.
`CodecSet` is on zencodec main; `RowConverter::prepare` / `try_convert_row`
are on the converter's unpublished bridge. These complete recipes require the
harness's pinned sources, not a manifest containing only the published versions.
The converter pin is `1d3ddd2131b22e9e3754c765d4ce45ce3901de59` and the JPEG pin
is `a2a128ab0bc36f01338225f61c285f462ae67ca6`.

## Qualification

Run `just jpeg-policy-sync <source-export>` and, in that export,
`just jpeg-policy-check`. The export avoids workspace path overrides. The
[conversion tests](../perf/jpeg-policy-validation/src/conversion.rs) check:

- Baseline and progressive JPEG decode: every converted BGRA sample against
  the native RGB result, dimensions and residual EXIF orientation.
- Dense, padded and trimmed-final-row framebuffers: exact RGB channels and
  byte-identical JPEG output, also decoded by independent `djpeg`.
- A transparent last pixel, unknown transfer and attached color context:
  rejection, preserving the concrete conversion or application error.

The full native harness and Clippy passed. Existing dependency HDR alias
deprecations remain visible. Wrapper receipt (test/build process, not an image
memory bound): `rc=0 2s | peak-RSS 0.25GiB | min-avail 19128MiB | peak-load 0.37`.

This does not qualify arbitrary ICC CMS execution, HDR tone mapping, planar
video conversion, image orientation transforms, resize, or streaming admission.
Those are explicit expansion points in the proposed pipeline, not implied by
successful format conversion.
