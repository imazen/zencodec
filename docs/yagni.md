# YAGNI Audit — Not Yet Adopted by Codecs

Audited 2026-03-09 against all crates in `~/work/zen/`.

Items listed here are legitimate API surface that codecs or consumers
should adopt but haven't yet. They are NOT removal candidates — they
represent the gap between the API design and current codec implementations.

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

## Not yet adopted — format negotiation

| Function | File | Notes |
|----------|------|-------|
| `best_encode_format()` | negotiate.rs | Consumers should use for codec-agnostic format selection |
| `is_format_available()` | negotiate.rs | |

## Not yet adopted — ImageInfo builders

| Method | Notes |
|--------|-------|
| `with_source_color()` | Codecs set `source_color` fields individually instead |
| `with_embedded_metadata()` | Codecs set fields individually instead |
| `with_warning()` / `with_warnings()` | No codec emits warnings yet |
| `has_warnings()` / `warnings()` | |

## Not yet adopted — ResourceLimits

| Method | Notes |
|--------|-------|
| `check_image_info()` | Codecs use `check_dimensions()` directly |
| `check_output_info()` | |

## Not yet adopted — capabilities

| Method | Notes |
|--------|-------|
| `EncodeCapabilities::supports()` | Callers check individual flags; useful for codec-agnostic dispatch |
| `DecodeCapabilities::supports()` | |

## Not yet adopted — output

| Method | Notes |
|--------|-------|
| `DecodeOutput::take_source_encoding_details()` | For re-encode pipelines |

## Not yet adopted — trait methods

| Method | Trait | Notes |
|--------|-------|-------|
| `DecodeJob::extensions_mut()` | DecodeJob | |
| `EncodeJob::extensions_mut()` | EncodeJob | |
