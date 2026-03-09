# YAGNI Audit — Zero Downstream Usage

Audited 2026-03-09 against all crates in `~/work/zen/`.

Items listed here have zero downstream usage but are kept for now.
They may be useful in the future, or may need a rework before adoption.

## Removed (2026-03-09)

- `Resolution`, `ResolutionUnit` — removed from info.rs
- `DecodeCost`, `EncodeCost` — removed from cost.rs
- `GainMapMetadata` — removed (entire module)
- `SourceColor::color_profile_source()`, `color_context()` — removed
- `ImageInfo::color_profile_source()`, `color_context()` — removed
- `MasteringDisplay::primaries_f64()`, `white_point_f64()` — removed (type now re-exported from zenpixels)
- `ResourceLimits::check_decode_cost()`, `check_encode_cost()` — removed
- `DecodeOutput::color_context()` — removed
- `ContentLightLevel`, `MasteringDisplay` — replaced with re-exports from zenpixels
- `ColorContext`, `ColorProfileSource`, `NamedProfile` — removed from root re-exports

## Remaining — entire types

| Type | File | Notes |
|------|------|-------|
| `DynStreamingDecoder` | traits/dyn_decoding.rs | No consumer uses streaming via dyn dispatch |

## Remaining — free functions

| Function | File | Notes |
|----------|------|-------|
| `best_encode_format()` | negotiate.rs | |
| `is_format_available()` | negotiate.rs | |

## Remaining — ImageInfo methods

| Method | Notes |
|--------|-------|
| `display_width()` / `display_height()` | Callers use `width` + `orientation.swaps_dimensions()` directly |
| `with_source_color()` | Codecs set `source_color` fields individually |
| `with_embedded_metadata()` | Codecs set fields individually |
| `with_warning()` / `with_warnings()` | No codec emits warnings yet |
| `has_warnings()` / `warnings()` | |
| `with_bit_depth()` / `with_channel_count()` | Codecs set `source_color.bit_depth` directly |

## Remaining — ResourceLimits methods

| Method | Notes |
|--------|-------|
| `check_image_info()` | Convenience; codecs use `check_dimensions()` directly |
| `check_output_info()` | |
| `has_any()` | |

## Remaining — Capabilities methods

| Method | Notes |
|--------|-------|
| `EncodeCapabilities::supports()` | Callers check individual flags |
| `DecodeCapabilities::supports()` | |

## Remaining — Output methods

| Method | Notes |
|--------|-------|
| `DecodeOutput::take_source_encoding_details()` | |
| `OwnedFullFrame::as_full_frame()` | |
| `EncodeOutput::with_extension()` | `with_mime_type()` has 1 use (zenjxl) |

## Remaining — Orientation

| Method | Notes |
|--------|-------|
| `Orientation::display_dimensions()` | Callers use `swaps_dimensions()` + manual swap |

## Remaining — Trait methods

| Method | Trait | Notes |
|--------|-------|-------|
| `DecodeJob::extensions_mut()` | DecodeJob | |
| `DecodeJob::dyn_full_frame_decoder()` | DecodeJob | |
| `DecodeJob::dyn_streaming_decoder()` | DecodeJob | |
| `EncodeJob::extensions_mut()` | EncodeJob | |
| `EncodeJob::dyn_full_frame_encoder()` | EncodeJob | |
