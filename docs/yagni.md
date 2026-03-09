# YAGNI Audit — Zero Downstream Usage

Audited 2026-03-09 against all crates in `~/work/zen/`.

Items listed here have zero downstream usage but are kept for now.
They may be useful in the future, or may need a rework before adoption.

## Entire types

| Type | File | Notes |
|------|------|-------|
| `Resolution` | info.rs | No codec reads or writes resolution yet |
| `ResolutionUnit` | info.rs | Tied to `Resolution` |
| `DecodeCost` | cost.rs | Needs rework; codecs profile internally but haven't wired cost estimation |
| `EncodeCost` | cost.rs | Same |
| `DynStreamingDecoder` | traits/dyn_decoding.rs | No consumer uses streaming via dyn dispatch |

## Free functions

| Function | File | Notes |
|----------|------|-------|
| `best_encode_format()` | negotiate.rs | |
| `is_format_available()` | negotiate.rs | |

## ImageInfo methods

| Method | Notes |
|--------|-------|
| `display_width()` / `display_height()` | Callers use `width` + `orientation.swaps_dimensions()` directly |
| `color_context()` | Callers use `PixelBuffer::color_context()` instead |
| `color_profile_source()` | |
| `with_source_color()` | Codecs set `source_color` fields individually |
| `with_embedded_metadata()` | Codecs set fields individually |
| `with_gain_map()` | Bool-only setter; `with_gain_map_metadata()` used instead |
| `with_warning()` / `with_warnings()` | No codec emits warnings yet |
| `has_warnings()` / `warnings()` | |
| `with_bit_depth()` / `with_channel_count()` | Codecs set `source_color.bit_depth` directly |

## SourceColor / MasteringDisplay methods

| Method | Notes |
|--------|-------|
| `SourceColor::color_profile_source()` | |
| `SourceColor::color_context()` | |
| `MasteringDisplay::primaries_f64()` | Convenience f64 getters |
| `MasteringDisplay::white_point_f64()` | |

## ResourceLimits methods

| Method | Notes |
|--------|-------|
| `check_image_info()` | Convenience; codecs use `check_dimensions()` directly |
| `check_output_info()` | |
| `check_decode_cost()` | Tied to unused `DecodeCost` |
| `check_encode_cost()` | Tied to unused `EncodeCost` |
| `has_any()` | |

## Capabilities methods

| Method | Notes |
|--------|-------|
| `EncodeCapabilities::supports()` | Callers check individual flags |
| `DecodeCapabilities::supports()` | |

## Output methods

| Method | Notes |
|--------|-------|
| `DecodeOutput::color_context()` | Delegates to `ImageInfo::color_context()` |
| `DecodeOutput::take_source_encoding_details()` | |
| `OwnedFullFrame::as_full_frame()` | |
| `EncodeOutput::with_extension()` | `with_mime_type()` has 1 use (zenjxl) |

## Orientation

| Method | Notes |
|--------|-------|
| `Orientation::display_dimensions()` | Callers use `swaps_dimensions()` + manual swap |

## GainMapMetadata

| Method | Notes |
|--------|-------|
| `is_uniform()` | |

## Trait methods

| Method | Trait | Notes |
|--------|-------|-------|
| `DecodeJob::extensions_mut()` | DecodeJob | |
| `DecodeJob::dyn_full_frame_decoder()` | DecodeJob | |
| `DecodeJob::dyn_streaming_decoder()` | DecodeJob | |
| `EncodeJob::extensions_mut()` | EncodeJob | |
| `EncodeJob::dyn_full_frame_encoder()` | EncodeJob | |
