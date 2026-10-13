# Multi-Image Design: ImageSequence, Supplements, and Probe Semantics

## Problem

zencodec's current `ImageInfo` has `has_animation: bool` and `frame_count: Option<u32>`.
This conflates "multiple images in a file" with "temporal animation frames."

TIFF, HEIF, DICOM, and ICO contain multiple independent images that are NOT animation.
Forcing them through `AnimationFrameDecoder` causes data corruption: compositing applied to
independent pages, fixed canvas cropping variable-size pages, format negotiation committed
once for heterogeneous content.

JPEG, AVIF, JXL, and HEIF increasingly carry gain maps (SDR-to-HDR tone mapping data)
as supplemental images. These are not pages and not animation frames — they modify the
primary image.

## New Types

### ImageSequence

Replaces `has_animation: bool` + `frame_count: Option<u32>` on `ImageInfo`.

```rust
/// What kind of image sequence the file contains.
///
/// Determines which decoder trait is appropriate:
/// - `Single` → `Decode`
/// - `Animation` → `AnimationFrameDecoder`
/// - `Multi` → future `MultiPageDecoder` (or `Decode` for primary only)
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ImageSequence {
    /// Single image. `Decode` returns it.
    Single,

    /// Temporal animation: frames share a canvas size, have durations,
    /// and may use compositing (disposal, blending, reference slots).
    ///
    /// Use `AnimationFrameDecoder`.
    Animation {
        /// Number of displayed frames. `None` if unknown without full parse
        /// (e.g., GIF requires scanning all frames to count them).
        frame_count: Option<u32>,
        /// Loop count: 0 = infinite, N = play N times. `None` = unspecified.
        loop_count: Option<u32>,
        /// Whether frame N can be rendered without decoding frames 0..N-1.
        ///
        /// True when all frames are full-canvas replacements (no disposal
        /// dependencies). False for GIF/APNG with inter-frame disposal.
        /// JXL is typically true (keyframe-based).
        random_access: bool,
    },

    /// Multiple independent images in a single container.
    ///
    /// Pages may differ in dimensions, pixel format, color space, and
    /// metadata. `Decode` returns the primary image only. Other images
    /// require a `MultiPageDecoder` (future) or the codec's native API.
    ///
    /// Examples: multi-page TIFF, HEIF collections, ICO sizes, DICOM slices,
    /// GeoTIFF spectral bands.
    ///
    /// `Multi` does not encode *why* images are multiple — document pages,
    /// spectral bands, Z-stack slices, and stereo pairs are all `Multi`.
    /// The semantic role is domain-specific; zencodec only describes
    /// what's in the container, not what it means. A GeoTIFF library
    /// knows bands should be stacked; zencodec just knows there are N images.
    Multi {
        /// Number of primary-level images, excluding thumbnails, masks,
        /// and pyramid levels (those are reported via `Supplements`).
        ///
        /// For TIFF: counts IFDs where `NewSubfileType` indicates
        /// full-resolution content. Reduced-resolution and mask IFDs are
        /// excluded. When `NewSubfileType` is absent (old TIFFs), all
        /// top-level IFDs are counted — the probe cannot distinguish
        /// pages from thumbnails without the tag.
        ///
        /// `None` if unknown without full parse (e.g., chained TIFF IFDs
        /// must be walked to count).
        image_count: Option<u32>,
        /// Whether image N can be decoded without decoding images 0..N-1.
        ///
        /// True for most container formats (TIFF IFDs, HEIF items, ICO
        /// entries) where each image is independently addressable.
        /// False would be unusual but possible for stream-oriented containers.
        ///
        /// Note: for TIFF, the data is always independently decodable, but
        /// finding IFD N requires walking the IFD chain from 0 (each IFD
        /// has a next-pointer). This is a container navigation cost, not a
        /// decode dependency — `random_access` is still `true` because
        /// decoding image N does not require decoding images 0..N-1.
        random_access: bool,
    },
}
```

**Key invariant**: the variant tells you which decoder trait applies. Code that sees `Multi`
knows not to use `AnimationFrameDecoder`. Code that sees `Animation` knows `MultiPageDecoder`
is wrong. `Single` means only `Decode` is needed.

**Scope boundary**: `Multi` tells callers *what's there* (N decodable images). It does not
tell them *what it means* (document pages vs spectral bands vs Z-stack slices). Domain
semantics belong to domain libraries (GeoTIFF, OME-TIFF) that use the codec's native API
for band stacking, spatial indexing, etc. The generic `MultiPageDecoder` exposes each image
as independently decodable — the caller decides how to interpret them.

### Supplements

Orthogonal to `ImageSequence`. Describes auxiliary content that accompanies the primary
image(s) — not independent images, but data that modifies or augments the primary content.

```rust
/// Supplemental content that accompanies the primary image(s).
///
/// These are not independent viewable images — they modify or augment
/// the primary content. Each supplement type implies a distinct access
/// pattern and a future accessor trait.
///
/// Populated during probe. May be incomplete from `probe()` (cheap) and
/// more complete from `probe_full()` (expensive).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Supplements {
    /// Reduced-resolution versions (image pyramid, thumbnails).
    ///
    /// TIFF pyramids, HEIF thumbnails, JPEG JFIF thumbnails.
    /// Future: `PyramidDecoder` to select resolution level.
    pub pyramid: bool,

    /// HDR gain map for SDR/HDR tone mapping.
    ///
    /// JPEG Ultra HDR (ISO 21496-1), AVIF gain map, JXL gain map,
    /// HEIF gain map. The gain map is a separate grayscale or multi-channel
    /// image with tone-mapping metadata.
    /// Future: `GainMapDecoder` to extract map + tone mapping parameters.
    pub gain_map: bool,

    /// Depth map (portrait mode, 3D reconstruction).
    ///
    /// HEIF depth maps, Google Camera depth in JPEG.
    /// Future: `AuxiliaryDecoder` with typed auxiliary selection.
    pub depth_map: bool,

    /// Other auxiliary images not covered by named fields.
    ///
    /// Alpha planes stored as separate images (HEIF), transparency masks
    /// (TIFF), vendor-specific auxiliary data.
    pub auxiliary: bool,
}
```

**Non-overlapping with ImageSequence**: `Multi` covers multiple primary-level images
(document pages). `Supplements` covers data that modifies/augments the primary image(s).
A 200-page TIFF with a pyramid for page 1 is `Multi { image_count: 200 }` +
`Supplements { pyramid: true }`. No redundancy.

## Changes to ImageInfo

```rust
pub struct ImageInfo {
    /// Width of the primary image in pixels.
    pub width: u32,
    /// Height of the primary image in pixels.
    pub height: u32,
    /// Detected image format.
    pub format: ImageFormat,
    /// Whether the primary image has an alpha channel.
    pub has_alpha: bool,

    // REMOVED: has_animation: bool
    // REMOVED: frame_count: Option<u32>

    /// What kind of image sequence the file contains.
    ///
    /// For `Single`, all fields describe the one image.
    /// For `Animation`, `width`/`height` are the canvas size.
    /// For `Multi`, `width`/`height` describe the primary image only —
    /// other images may have different dimensions.
    pub sequence: ImageSequence,

    /// Supplemental content alongside the primary image(s).
    pub supplements: Supplements,

    pub orientation: Orientation,
    pub source_color: SourceColor,
    pub embedded_metadata: EmbeddedMetadata,
    pub source_encoding: Option<Arc<dyn SourceEncodingDetails>>,
    pub warnings: Vec<String>,
}
```

### Scope of fields

| Field | Single | Animation | Multi |
|-------|--------|-----------|-------|
| `width`, `height` | The image | Canvas size | Primary image only |
| `has_alpha` | The image | Canvas alpha | Primary image only |
| `orientation` | The image | Canvas orientation | Primary image only |
| `source_color` | The image | Overall color info | Primary image only |
| `embedded_metadata` | The image | Container-level metadata | Primary image only |
| `sequence` | `Single` | `Animation { .. }` | `Multi { .. }` |
| `supplements` | May have gain map etc. | May have gain map etc. | May have pyramid etc. |

For `Multi`, other images may have completely different dimensions, color spaces,
and metadata. This is only discoverable via per-image probing (future `MultiPageDecoder`).

## Probe Semantics

### `probe(&self, data: &[u8]) -> Result<ImageInfo, E>`

**Cost**: O(header). Parses container headers only, never decodes pixel data.

**Contract**:
- `width`/`height` = primary image stored dimensions
- `sequence` variant is always correct (Single/Animation/Multi)
- `sequence` counts (`frame_count`, `image_count`) may be `None` — filling them in
  may require walking all IFDs/frames, which is not O(header)
- `supplements` is best-effort — may be incomplete if discovery requires full parse
  (e.g., a gain map signaled only in EXIF APP1 that wasn't fully parsed)
- `source_color` populated to the extent the header reveals (bit depth, ICC if near
  header, CICP if in container box). May be incomplete.
- `embedded_metadata` may be incomplete (EXIF/XMP at end of file not read)
- `random_access` on sequence variants should be correct — this is typically a
  format-level property, not a per-file discovery

**When `cheap_probe` capability is true**: `probe()` is guaranteed fast (sub-millisecond
for typical files). The header is small and fixed-layout.

**When `cheap_probe` capability is false**: `probe()` may still do work proportional
to header size, but not pixel-proportional. Some formats have large headers
(e.g., HEIF with many items, TIFF with many IFDs chained).

### `probe_full(&self, data: &[u8]) -> Result<ImageInfo, E>`

**Cost**: O(container), may be expensive. Parses the full container structure without
decoding pixels.

**Contract**:
- Everything `probe()` returns, plus:
- `frame_count` / `image_count` filled in (walked all frames/IFDs/items)
- `supplements` complete (all auxiliary items discovered)
- `embedded_metadata` complete (EXIF/XMP found even at end of file)
- `source_color` as complete as possible without decoding

**When to use**: When the caller needs accurate counts or complete metadata.
Callers building a page navigator, counting animation frames for progress bars,
or checking for gain maps should use `probe_full()`.

**Default**: delegates to `probe()`. Codecs where full parse is trivially cheap
(PNG, JPEG, BMP) don't need to override.

### Relationship between probe and decode

```
probe()         → ImageInfo (cheap, possibly incomplete)
probe_full()    → ImageInfo (complete, possibly expensive)
output_info()   → OutputInfo (post-hint dimensions + format for primary image)
decode()        → DecodeOutput (pixels + ImageInfo for primary image)
```

`output_info()` and `decode()` always operate on the **primary image**. For multi-image
files, the primary image is:
- **TIFF**: first IFD that is a full-resolution image (not a thumbnail/mask)
- **HEIF**: the item marked as primary (`pitm` box)
- **ICO**: the largest/highest-quality entry
- **GIF/APNG/WebP animation**: frame 0

The `ImageInfo` returned by `decode()` may be more complete than `probe()` — the decoder
extracts ICC/EXIF/XMP during pixel decode. The `sequence` and `supplements` fields should
match between probe and decode.

## TIFF Scenarios

How various real-world TIFF files map to the model:

| TIFF file | `sequence` | `supplements` | Notes |
|-----------|-----------|---------------|-------|
| Single image | `Single` | `default()` | Trivial case |
| Single + pyramid SubIFDs | `Single` | `pyramid: true` | SubIFDs are resolution levels, not pages |
| Single + transparency mask IFD | `Single` | `auxiliary: true` | `NewSubfileType` bit 2 = mask |
| 3-page scan (same dims) | `Multi { count: 3, ra: true }` | `default()` | Clean multi-page |
| 2 pages + per-page thumbnails | `Multi { count: 2, ra: true }` | `pyramid: true` | `image_count` = 2 (pages only), thumbnails are supplements |
| GeoTIFF, 4 spectral bands | `Multi { count: 4, ra: true }` | `default()` | Bands are independently decodable images; stacking is domain logic |
| OME-TIFF, 200 Z-slices | `Multi { count: 200, ra: true }` | `default()` | Same — dimensional interpretation is caller's job |
| Old TIFF, 2 IFDs, no NewSubfileType | `Multi { count: 2, ra: true }` | `default()` | Can't distinguish page from thumbnail without the tag |
| Image + pyramid + mask | `Single` | `pyramid: true, auxiliary: true` | Primary is the full-res; pyramid and mask are supplements |

**Domain-specific access**: GeoTIFF callers that need band metadata, spatial
reference systems, or band stacking should use zentiff's native API (raw IFD
access with per-IFD tag queries). The generic `MultiPageDecoder` treats each
image as independently decodable without domain interpretation.

## Changes to DecodeCapabilities

Replace the `animation: bool` field:

```rust
pub struct DecodeCapabilities {
    // REMOVED: animation: bool

    /// Whether this decoder supports `AnimationFrameDecoder` (animation decode).
    ///
    /// True only for codecs with temporal animation (GIF, APNG, WebP anim,
    /// AVIF sequence, JXL animation). False for multi-image containers
    /// that are not animation (TIFF, HEIF collections, ICO).
    animation: bool,

    /// Whether this decoder supports `MultiPageDecoder` (multi-image decode).
    ///
    /// True for codecs with independently-addressable images: TIFF (IFDs),
    /// HEIF (items), ICO (entries), DICOM (slices).
    /// False for single-image and animation-only codecs.
    multi_image: bool,

    // ... existing fields unchanged ...
}
```

And corresponding `UnsupportedOperation` variants:

```rust
pub enum UnsupportedOperation {
    // EXISTING (unchanged):
    AnimationDecode,    // AnimationFrameDecoder methods

    // NEW:
    MultiImageDecode,   // Future MultiPageDecoder methods
    // ...
}
```

## Changes to AnimationFrameDecoder

No structural changes. Documentation clarified:

```rust
/// Full-frame composited **animation** decode.
///
/// ONLY for `ImageSequence::Animation`. The decoder composites internally
/// (handling disposal, blending, sub-canvas positioning) and yields
/// full-canvas frames ready for display.
///
/// **Not for multi-image containers.** Files with `ImageSequence::Multi`
/// contain independent images that may differ in dimensions and format.
/// Using `AnimationFrameDecoder` for such files would apply compositing to
/// independent images, force a fixed canvas size, and silently destroy data.
/// Use `MultiPageDecoder` (future) or the codec's native API instead.
```

`AnimationFrame` keeps `duration_ms` — this is animation-specific and reinforces
that `AnimationFrameDecoder` is for temporal content.

## Future: MultiPageDecoder (not implemented now)

Design space reserved by `ImageSequence::Multi` and `multi_image` capability.
Sketch for context — not part of this change:

```rust
/// Multi-image decode for containers with independent images.
///
/// Each image has its own `ImageInfo` with potentially different dimensions,
/// pixel format, color space, and metadata. No compositing — images are
/// independent documents, resolution levels, or data planes.
///
/// Access pattern depends on `ImageSequence::Multi::random_access`:
/// - `true`: `page_info(index)` and `decode_page(index, ..)` work for any index
/// - `false`: only sequential `next_page()` iteration is safe
pub trait MultiPageDecoder: Sized {
    type Error: core::error::Error + Send + Sync + 'static;

    /// Number of images, if known.
    fn image_count(&self) -> Option<u32>;

    /// Metadata for a specific image.
    ///
    /// Returns `Err` if `random_access` is false and index != next sequential.
    fn page_info(&mut self, index: u32) -> Result<ImageInfo, Self::Error>;

    /// Decode a specific image.
    ///
    /// `preferred` is per-page — each page may produce a different format.
    /// Returns `Err` if `random_access` is false and index != next sequential.
    fn decode_page(
        &mut self,
        index: u32,
        preferred: &[PixelDescriptor],
        stop: Option<&dyn Stop>,
    ) -> Result<DecodeOutput, Self::Error>;
}
```

Key differences from `AnimationFrameDecoder`:
- No canvas size — each page has its own dimensions
- No compositing — pages are independent
- No `duration_ms` — pages are not temporal
- Per-page format negotiation via `preferred`
- Per-page `ImageInfo` with independent metadata
- Random access (when available) — no sequential dependency
- `page_info()` without decode — cheap per-page probing

## Future: Supplement Accessors (not implemented now)

Each supplement type implies a distinct future accessor:

**Gain map**: Extract the gain map image + tone mapping metadata.
The gain map is a separate decode (grayscale or RGB, possibly different resolution
from the primary). Tone mapping parameters (headroom, offset, gamma) are metadata.
```rust
trait GainMapDecoder {
    fn gain_map_info(&self) -> Result<GainMapInfo>;
    fn decode_gain_map(&mut self, preferred: &[PixelDescriptor]) -> Result<DecodeOutput>;
}
```

**Pyramid**: Select resolution level before decode.
```rust
trait PyramidDecoder {
    fn level_count(&self) -> u32;
    fn level_info(&self, level: u32) -> Result<ImageInfo>;
    fn decode_level(&mut self, level: u32, preferred: &[PixelDescriptor]) -> Result<DecodeOutput>;
}
```

**Depth map, auxiliary**: Similar pattern with typed selection.

All of these are independent from `Decode`, `AnimationFrameDecoder`, and `MultiPageDecoder`.
`Supplements` tells callers what's available; the accessor traits provide access.

## Tangent: OutputInfo dual-role cleanup

`OutputInfo` currently serves two roles that want different things:

1. **Prediction** (`DecodeJob::output_info()`) — "what will decode produce?" Caller needs
   `crop_applied`, `orientation_applied` to know which hints were honored. Called before
   decode to allocate buffers.

2. **Report** (`push_decoder()`, `render_next_frame_to_sink()`) — "what did I write to
   your sink?" Caller just needs width/height/format. The hint feedback fields are dead
   weight here.

Not blocking for multi-image, but worth splitting in a future cleanup:
- `OutputInfo` stays as the prediction type (keeps hint feedback fields)
- A lighter `SinkResult` or similar for the report role (just dimensions + descriptor)

## Migration

### Breaking changes

- `ImageInfo::has_animation` removed → use `matches!(info.sequence, Animation { .. })`
- `ImageInfo::frame_count` removed → use `info.sequence.frame_count()` helper
- `ImageInfo::with_animation()` removed → use `with_sequence(ImageSequence::Animation { .. })`
- `ImageInfo::with_frame_count()` removed → set count on sequence variant

### Compatibility helpers

```rust
impl ImageInfo {
    /// Whether this file contains animation.
    ///
    /// Convenience for `matches!(self.sequence, ImageSequence::Animation { .. })`.
    pub fn is_animation(&self) -> bool {
        matches!(self.sequence, ImageSequence::Animation { .. })
    }

    /// Whether this file contains multiple independent images.
    pub fn is_multi_image(&self) -> bool {
        matches!(self.sequence, ImageSequence::Multi { .. })
    }

    /// Whether `Decode` returns only one of multiple images in this file.
    ///
    /// True for both animation and multi-image. When true, `Decode` returns
    /// the primary image and additional images require specialized decoders.
    pub fn has_additional_images(&self) -> bool {
        !matches!(self.sequence, ImageSequence::Single)
    }
}

impl ImageSequence {
    /// Frame/image count if known.
    pub fn count(&self) -> Option<u32> {
        match self {
            Self::Single => Some(1),
            Self::Animation { frame_count, .. } => *frame_count,
            Self::Multi { image_count, .. } => *image_count,
        }
    }

    /// Whether individual frames/images can be accessed without decoding all priors.
    pub fn random_access(&self) -> bool {
        match self {
            Self::Single => true,
            Self::Animation { random_access, .. } => *random_access,
            Self::Multi { random_access, .. } => *random_access,
        }
    }
}
```

## Examples

### Codec probe implementations

**zentiff**:
```rust
fn probe(&self, data: &[u8]) -> Result<ImageInfo> {
    let reader = tiff::decoder::Decoder::new(Cursor::new(data))?;
    let (width, height) = reader.dimensions()?;

    // Count IFDs (cheap — just follows next-IFD pointers)
    let ifd_count = count_ifds(&reader);

    let sequence = if ifd_count <= 1 {
        ImageSequence::Single
    } else {
        ImageSequence::Multi {
            image_count: Some(ifd_count),
            random_access: true, // TIFF IFDs are independent
        }
    };

    let supplements = Supplements {
        pyramid: has_reduced_resolution_ifds(&reader),
        ..Default::default()
    };

    Ok(ImageInfo::new(width, height, ImageFormat::Tiff)
        .with_sequence(sequence)
        .with_supplements(supplements))
}
```

**zengif**:
```rust
fn probe(&self, data: &[u8]) -> Result<ImageInfo> {
    let (width, height) = read_gif_header(data)?;
    Ok(ImageInfo::new(width, height, ImageFormat::Gif)
        .with_sequence(ImageSequence::Animation {
            frame_count: None, // unknown without full parse
            loop_count: None,  // in NETSCAPE extension, may not be in header
            random_access: false, // GIF frames have disposal dependencies
        }))
}

fn probe_full(&self, data: &[u8]) -> Result<ImageInfo> {
    let (width, height, frame_count, loop_count) = parse_full_gif(data)?;
    Ok(ImageInfo::new(width, height, ImageFormat::Gif)
        .with_sequence(ImageSequence::Animation {
            frame_count: Some(frame_count),
            loop_count: Some(loop_count),
            random_access: false,
        }))
}
```

**zenjpeg** (with Ultra HDR gain map):
```rust
fn probe(&self, data: &[u8]) -> Result<ImageInfo> {
    let header = parse_jpeg_header(data)?;
    let supplements = Supplements {
        gain_map: header.has_mpf_gain_map || header.has_iso21496_gain_map,
        ..Default::default()
    };
    Ok(ImageInfo::new(header.width, header.height, ImageFormat::Jpeg)
        .with_sequence(ImageSequence::Single)
        .with_supplements(supplements))
}
```

### Caller decision logic

```rust
let info = config.job().probe(data)?;

// Primary image — always available
let primary = config.job().decoder(data.into(), &preferred)?.decode()?;

// What else is in this file?
match &info.sequence {
    ImageSequence::Single => { /* done */ }

    ImageSequence::Animation { random_access, .. } => {
        let mut dec = config.job().animation_frame_decoder(data.into(), &preferred)?;
        while let Some(frame) = dec.render_next_frame(None)? {
            process_frame(frame.pixels(), frame.duration_ms());
        }
    }

    ImageSequence::Multi { image_count, random_access, .. } => {
        if *random_access {
            // Future: can decode any page by index
            // let page_5 = multi.decode_page(5, &preferred, None)?;
        }
        println!("File has {} images, decoded primary only",
            image_count.map_or("unknown".into(), |n| n.to_string()));
    }
}

// Supplements are orthogonal
if info.supplements.gain_map {
    // Future: extract gain map for HDR rendering
    // let gm = gain_map_decoder.decode_gain_map(&preferred)?;
}
```

## Summary of Changes

| Component | Change | Breaking? |
|-----------|--------|-----------|
| `ImageSequence` enum | New type | No (additive) |
| `Supplements` struct | New type | No (additive) |
| `ImageInfo.has_animation` | Remove, replace with `sequence` | Yes |
| `ImageInfo.frame_count` | Remove, now in `ImageSequence` variant | Yes |
| `ImageInfo.sequence` | New field | Yes (struct is `#[non_exhaustive]` but manual `PartialEq`) |
| `ImageInfo.supplements` | New field | Yes (same) |
| `ImageInfo::with_animation()` | Remove | Yes |
| `ImageInfo::with_frame_count()` | Remove | Yes |
| `ImageInfo::with_sequence()` | New builder method | No |
| `ImageInfo::with_supplements()` | New builder method | No |
| `DecodeCapabilities.multi_image` | New field | No (`#[non_exhaustive]`, getter-based) |
| `UnsupportedOperation::MultiImageDecode` | New variant | No (`#[non_exhaustive]`) |
| `AnimationFrameDecoder` docs | Clarify animation-only | No |
| `DecodeJob` docs | Clarify primary-image semantics | No |
| `probe()` docs | Clarify scope and cost | No |

New code: ~100 lines of type definitions + ~50 lines of helpers + docs.
No new traits. No new methods on existing traits.
