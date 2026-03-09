//! Image format detection, metadata, and registry.

use alloc::borrow::Cow;
use alloc::vec::Vec;

pub(crate) mod builtins;

// ===========================================================================
// ImageFormatDefinition
// ===========================================================================

/// Describes an image format's metadata, capabilities, and detection logic.
///
/// Used both for built-in formats (via [`ImageFormatRegistry::common()`]) and
/// for custom formats defined by downstream crates. Define as a `static` and
/// reference via [`ImageFormat::Custom`].
///
/// Identity is based on `name` — two definitions with the same name are
/// considered equal.
///
/// # Example
///
/// ```rust,ignore
/// use zc::{ImageFormatDefinition, ImageFormat, ImageFormatRegistry};
///
/// fn detect_jpeg2000(data: &[u8]) -> bool {
///     data.len() >= 12 && data[..4] == [0x00, 0x00, 0x00, 0x0C]
///         && &data[4..8] == b"jP  "
/// }
///
/// static JPEG2000: ImageFormatDefinition = ImageFormatDefinition::new(
///     "jpeg2000",
///     None,
///     "JPEG 2000",
///     "jp2",
///     &["jp2", "j2k", "jpx"],
///     "image/jp2",
///     &["image/jp2", "image/jpx"],
///     true,  // alpha
///     false, // animation
///     true,  // lossless
///     true,  // lossy
///     12,
///     detect_jpeg2000,
/// );
///
/// // Build a registry with custom + common formats
/// let registry = ImageFormatRegistry::from_vec(vec![&JPEG2000]);
/// let fmt = registry.detect(data);
/// ```
#[non_exhaustive]
pub struct ImageFormatDefinition {
    /// Unique lowercase format identifier (e.g. `"jpeg2000"`, `"dds"`).
    ///
    /// Used for equality comparison and hashing. Must be unique across
    /// all format definitions in use.
    pub name: &'static str,

    /// The corresponding built-in [`ImageFormat`] variant, if any.
    ///
    /// Set to `Some(ImageFormat::Jpeg)` etc. for definitions that describe
    /// built-in formats. Set to `None` for custom formats — the registry
    /// wraps them as [`ImageFormat::Custom`].
    pub image_format: Option<ImageFormat>,

    /// Human-readable format name for display (e.g. `"JPEG 2000"`, `"DDS"`).
    pub display_name: &'static str,

    /// Primary file extension without dot (e.g. `"jp2"`).
    pub preferred_extension: &'static str,

    /// All recognized file extensions (must include `preferred_extension`).
    pub extensions: &'static [&'static str],

    /// Primary MIME type (e.g. `"image/jp2"`).
    pub preferred_mime_type: &'static str,

    /// All recognized MIME types (must include `preferred_mime_type`).
    pub mime_types: &'static [&'static str],

    /// Whether this format supports alpha channel.
    pub supports_alpha: bool,

    /// Whether this format supports animation.
    pub supports_animation: bool,

    /// Whether this format supports lossless encoding.
    pub supports_lossless: bool,

    /// Whether this format supports lossy encoding.
    pub supports_lossy: bool,

    /// Recommended bytes to fetch for reliable format probing.
    ///
    /// The `detect` function must still handle shorter inputs safely
    /// (returning `false` for inconclusive data).
    pub magic_bytes_needed: usize,

    /// Magic byte detection function.
    ///
    /// Returns `true` if the data appears to be this format.
    /// Must handle any data length safely (including empty slices).
    pub detect: fn(&[u8]) -> bool,
}

impl ImageFormatDefinition {
    /// Create a new format definition.
    ///
    /// All fields are required. For built-in formats, set `image_format` to the
    /// corresponding [`ImageFormat`] variant. For custom formats, set it to `None`.
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        name: &'static str,
        image_format: Option<ImageFormat>,
        display_name: &'static str,
        preferred_extension: &'static str,
        extensions: &'static [&'static str],
        preferred_mime_type: &'static str,
        mime_types: &'static [&'static str],
        supports_alpha: bool,
        supports_animation: bool,
        supports_lossless: bool,
        supports_lossy: bool,
        magic_bytes_needed: usize,
        detect: fn(&[u8]) -> bool,
    ) -> Self {
        Self {
            name,
            image_format,
            display_name,
            preferred_extension,
            extensions,
            preferred_mime_type,
            mime_types,
            supports_alpha,
            supports_animation,
            supports_lossless,
            supports_lossy,
            magic_bytes_needed,
            detect,
        }
    }

    /// Convert this definition to the corresponding [`ImageFormat`].
    ///
    /// Returns the built-in variant if `image_format` is `Some`, otherwise
    /// wraps as [`ImageFormat::Custom`].
    pub fn to_image_format(&'static self) -> ImageFormat {
        self.image_format.unwrap_or(ImageFormat::Custom(self))
    }
}

impl PartialEq for ImageFormatDefinition {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
    }
}

impl Eq for ImageFormatDefinition {}

impl core::hash::Hash for ImageFormatDefinition {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.name.hash(state);
    }
}

impl core::fmt::Debug for ImageFormatDefinition {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ImageFormatDefinition")
            .field("name", &self.name)
            .field("display_name", &self.display_name)
            .finish()
    }
}

// ===========================================================================
// ImageFormat enum
// ===========================================================================

/// Supported image formats.
///
/// Includes well-known formats as named variants and a [`Custom`](ImageFormat::Custom)
/// variant for formats defined by downstream crates.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ImageFormat {
    Jpeg,
    Png,
    Gif,
    WebP,
    Avif,
    Jxl,
    Heic,
    Bmp,
    Tiff,
    Ico,
    Pnm,
    Farbfeld,
    Qoi,
    Unknown,
    /// Format not known to zencodec-types.
    ///
    /// Define an [`ImageFormatDefinition`] as a `static` and reference it here.
    /// The definition carries its own metadata (extensions, MIME types,
    /// detection function, capability flags).
    Custom(&'static ImageFormatDefinition),
}

impl ImageFormat {
    /// The [`ImageFormatDefinition`] for this format, if known.
    ///
    /// Returns `None` only for [`Unknown`](ImageFormat::Unknown).
    /// All built-in variants and [`Custom`](ImageFormat::Custom) formats
    /// have definitions.
    pub fn definition(self) -> Option<&'static ImageFormatDefinition> {
        match self {
            ImageFormat::Jpeg => Some(&builtins::JPEG),
            ImageFormat::Png => Some(&builtins::PNG),
            ImageFormat::Gif => Some(&builtins::GIF),
            ImageFormat::WebP => Some(&builtins::WEBP),
            ImageFormat::Avif => Some(&builtins::AVIF),
            ImageFormat::Jxl => Some(&builtins::JXL),
            ImageFormat::Heic => Some(&builtins::HEIC),
            ImageFormat::Bmp => Some(&builtins::BMP),
            ImageFormat::Tiff => Some(&builtins::TIFF),
            ImageFormat::Ico => Some(&builtins::ICO),
            ImageFormat::Pnm => Some(&builtins::PNM),
            ImageFormat::Farbfeld => Some(&builtins::FARBFELD),
            ImageFormat::Qoi => Some(&builtins::QOI),
            ImageFormat::Custom(def) => Some(def),
            ImageFormat::Unknown => None,
        }
    }

    /// Primary MIME type string.
    pub fn mime_type(self) -> &'static str {
        self.definition()
            .map_or("application/octet-stream", |d| d.preferred_mime_type)
    }

    /// All recognized MIME types for this format.
    pub fn mime_types(self) -> &'static [&'static str] {
        self.definition().map_or(&[], |d| d.mime_types)
    }

    /// Primary file extension (without dot).
    pub fn extension(self) -> &'static str {
        self.definition().map_or("bin", |d| d.preferred_extension)
    }

    /// All recognized file extensions.
    pub fn extensions(self) -> &'static [&'static str] {
        self.definition().map_or(&[], |d| d.extensions)
    }

    /// Whether this format supports lossy encoding.
    pub fn supports_lossy(self) -> bool {
        self.definition().is_some_and(|d| d.supports_lossy)
    }

    /// Whether this format supports lossless encoding.
    pub fn supports_lossless(self) -> bool {
        self.definition().is_some_and(|d| d.supports_lossless)
    }

    /// Whether this format supports animation.
    pub fn supports_animation(self) -> bool {
        self.definition().is_some_and(|d| d.supports_animation)
    }

    /// Whether this format supports alpha channel.
    pub fn supports_alpha(self) -> bool {
        self.definition().is_some_and(|d| d.supports_alpha)
    }

    /// Recommended bytes to fetch for probing any format.
    ///
    /// 4096 bytes is enough for all built-in formats including JPEG (which
    /// may have large EXIF/APP segments before the SOF marker).
    pub const RECOMMENDED_PROBE_BYTES: usize = 4096;

    /// Recommended bytes to fetch for reliable format probing.
    pub fn magic_bytes_needed(self) -> usize {
        self.definition().map_or(0, |d| d.magic_bytes_needed)
    }
}

impl core::fmt::Display for ImageFormat {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self.definition() {
            Some(def) => f.write_str(def.display_name),
            None => f.write_str("Unknown"),
        }
    }
}

// ===========================================================================
// ImageFormatRegistry
// ===========================================================================

/// A collection of [`ImageFormatDefinition`]s with lookup methods.
///
/// Use [`common()`](ImageFormatRegistry::common) for the default registry
/// containing all built-in formats. Use [`with()`](ImageFormatRegistry::with)
/// to add custom formats.
///
/// # Example
///
/// ```rust,ignore
/// use zc::{ImageFormatRegistry, ImageFormatDefinition};
///
/// // Default: all built-in formats
/// let reg = ImageFormatRegistry::common();
/// assert_eq!(reg.detect(jpeg_bytes), Some(ImageFormat::Jpeg));
///
/// // Custom: build your own
/// let reg = ImageFormatRegistry::from_vec(vec![&JPEG2000]);
/// ```
#[derive(Clone, Debug)]
pub struct ImageFormatRegistry {
    formats: Cow<'static, [&'static ImageFormatDefinition]>,
}

impl ImageFormatRegistry {
    /// Registry containing all built-in format definitions.
    ///
    /// Detection order follows priority: JPEG, PNG, GIF, WebP, AVIF, JXL,
    /// HEIC, BMP, farbfeld, PNM, TIFF, ICO, QOI. AVIF is checked before
    /// HEIC so that ambiguous ISOBMFF containers (mif1/msf1 with both
    /// brands) resolve to AVIF.
    ///
    /// Zero allocation — backed by a static slice.
    pub fn common() -> Self {
        Self {
            formats: Cow::Borrowed(builtins::ALL),
        }
    }

    /// Registry backed by a static slice. Zero allocation.
    pub fn from_static(defs: &'static [&'static ImageFormatDefinition]) -> Self {
        Self {
            formats: Cow::Borrowed(defs),
        }
    }

    /// Create a registry from an owned list of definitions.
    pub fn from_vec(defs: Vec<&'static ImageFormatDefinition>) -> Self {
        Self {
            formats: Cow::Owned(defs),
        }
    }

    /// The format definitions in this registry, in detection priority order.
    pub fn formats(&self) -> &[&'static ImageFormatDefinition] {
        &self.formats
    }

    /// Detect format from magic bytes.
    ///
    /// Checks definitions in order, returns the first match. Returns `None`
    /// if no definition matches.
    pub fn detect(&self, data: &[u8]) -> Option<ImageFormat> {
        for def in self.formats.iter() {
            if (def.detect)(data) {
                return Some(def.image_format.unwrap_or(ImageFormat::Custom(def)));
            }
        }
        None
    }

    /// Detect format from file extension (case-insensitive).
    pub fn from_extension(&self, ext: &str) -> Option<ImageFormat> {
        let ext_bytes = ext.as_bytes();
        for def in self.formats.iter() {
            for &def_ext in def.extensions {
                if ext_bytes.len() == def_ext.len()
                    && ext_bytes
                        .iter()
                        .zip(def_ext.as_bytes())
                        .all(|(&a, &b)| a.to_ascii_lowercase() == b)
                {
                    return Some(def.image_format.unwrap_or(ImageFormat::Custom(def)));
                }
            }
        }
        None
    }

    /// Detect format from MIME type (case-insensitive).
    pub fn from_mime_type(&self, mime: &str) -> Option<ImageFormat> {
        for def in self.formats.iter() {
            for &def_mime in def.mime_types {
                if mime.eq_ignore_ascii_case(def_mime) {
                    return Some(def.image_format.unwrap_or(ImageFormat::Custom(def)));
                }
            }
        }
        None
    }
}

impl Default for ImageFormatRegistry {
    /// Returns [`common()`](ImageFormatRegistry::common).
    fn default() -> Self {
        Self::common()
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn reg() -> ImageFormatRegistry {
        ImageFormatRegistry::common()
    }

    #[test]
    fn detect_jpeg() {
        assert_eq!(
            reg().detect(&[0xFF, 0xD8, 0xFF, 0xE0]),
            Some(ImageFormat::Jpeg)
        );
    }

    #[test]
    fn detect_png() {
        assert_eq!(
            reg().detect(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]),
            Some(ImageFormat::Png)
        );
    }

    #[test]
    fn detect_gif() {
        assert_eq!(reg().detect(b"GIF89a\x00\x00"), Some(ImageFormat::Gif));
    }

    #[test]
    fn detect_webp() {
        assert_eq!(
            reg().detect(b"RIFF\x00\x00\x00\x00WEBP"),
            Some(ImageFormat::WebP)
        );
    }

    #[test]
    fn detect_avif() {
        assert_eq!(
            reg().detect(b"\x00\x00\x00\x18ftypavif"),
            Some(ImageFormat::Avif)
        );
    }

    #[test]
    fn detect_jxl_codestream() {
        assert_eq!(reg().detect(&[0xFF, 0x0A]), Some(ImageFormat::Jxl));
    }

    #[test]
    fn detect_jxl_container() {
        assert_eq!(
            reg().detect(&[
                0x00, 0x00, 0x00, 0x0C, b'J', b'X', b'L', b' ', 0x0D, 0x0A, 0x87, 0x0A
            ]),
            Some(ImageFormat::Jxl)
        );
    }

    #[test]
    fn detect_unknown() {
        assert_eq!(reg().detect(b"nope"), None);
        assert_eq!(reg().detect(&[]), None);
    }

    #[test]
    fn from_extension_case_insensitive() {
        assert_eq!(reg().from_extension("JPG"), Some(ImageFormat::Jpeg));
        assert_eq!(reg().from_extension("WebP"), Some(ImageFormat::WebP));
        assert_eq!(reg().from_extension("unknown"), None);
    }

    #[test]
    fn mime_types_primary() {
        assert_eq!(ImageFormat::Jpeg.mime_type(), "image/jpeg");
        assert_eq!(ImageFormat::Jxl.mime_type(), "image/jxl");
    }

    #[test]
    fn mime_types_all() {
        assert_eq!(ImageFormat::Jpeg.mime_types(), &["image/jpeg"]);
        assert!(ImageFormat::Heic.mime_types().contains(&"image/heif"));
        assert!(ImageFormat::Heic.mime_types().contains(&"image/heic"));
    }

    #[test]
    fn probe_constants() {
        assert_eq!(ImageFormat::RECOMMENDED_PROBE_BYTES, 4096);
        assert!(ImageFormat::Jpeg.magic_bytes_needed() > ImageFormat::Png.magic_bytes_needed());
    }

    #[test]
    fn display_format() {
        assert_eq!(alloc::format!("{}", ImageFormat::Jpeg), "JPEG");
        assert_eq!(alloc::format!("{}", ImageFormat::WebP), "WebP");
        assert_eq!(alloc::format!("{}", ImageFormat::Gif), "GIF");
        assert_eq!(alloc::format!("{}", ImageFormat::Png), "PNG");
        assert_eq!(alloc::format!("{}", ImageFormat::Avif), "AVIF");
        assert_eq!(alloc::format!("{}", ImageFormat::Jxl), "JPEG XL");
    }

    #[test]
    fn from_extension_all_variants() {
        assert_eq!(reg().from_extension("jpg"), Some(ImageFormat::Jpeg));
        assert_eq!(reg().from_extension("jpeg"), Some(ImageFormat::Jpeg));
        assert_eq!(reg().from_extension("jpe"), Some(ImageFormat::Jpeg));
        assert_eq!(reg().from_extension("jfif"), Some(ImageFormat::Jpeg));
        assert_eq!(reg().from_extension("JPEG"), Some(ImageFormat::Jpeg));
        assert_eq!(reg().from_extension("webp"), Some(ImageFormat::WebP));
        assert_eq!(reg().from_extension("gif"), Some(ImageFormat::Gif));
        assert_eq!(reg().from_extension("png"), Some(ImageFormat::Png));
        assert_eq!(reg().from_extension("avif"), Some(ImageFormat::Avif));
        assert_eq!(reg().from_extension("jxl"), Some(ImageFormat::Jxl));
    }

    #[test]
    fn from_extension_edge_cases() {
        assert_eq!(reg().from_extension(""), None);
        assert_eq!(reg().from_extension("tiff"), Some(ImageFormat::Tiff));
        assert_eq!(reg().from_extension("very_long_extension"), None);
    }

    #[test]
    fn capabilities() {
        assert!(ImageFormat::Jpeg.supports_lossy());
        assert!(!ImageFormat::Jpeg.supports_lossless());
        assert!(!ImageFormat::Jpeg.supports_animation());
        assert!(!ImageFormat::Jpeg.supports_alpha());

        assert!(ImageFormat::Png.supports_lossless());
        assert!(!ImageFormat::Png.supports_lossy());
        assert!(ImageFormat::Png.supports_alpha());
        assert!(ImageFormat::Png.supports_animation());

        assert!(ImageFormat::WebP.supports_lossy());
        assert!(ImageFormat::WebP.supports_lossless());
        assert!(ImageFormat::WebP.supports_animation());
        assert!(ImageFormat::WebP.supports_alpha());

        assert!(ImageFormat::Gif.supports_animation());
        assert!(ImageFormat::Gif.supports_lossless());
        assert!(ImageFormat::Gif.supports_alpha());

        assert!(ImageFormat::Jxl.supports_lossy());
        assert!(ImageFormat::Jxl.supports_lossless());
        assert!(ImageFormat::Jxl.supports_animation());
    }

    #[test]
    fn extensions() {
        assert!(ImageFormat::Jpeg.extensions().contains(&"jpg"));
        assert!(ImageFormat::Jpeg.extensions().contains(&"jpeg"));
        assert_eq!(ImageFormat::Png.extensions(), &["png"]);
    }

    #[test]
    fn detect_pnm_p5() {
        assert_eq!(reg().detect(b"P5\n3 2\n255\n"), Some(ImageFormat::Pnm));
    }

    #[test]
    fn detect_pnm_p6() {
        assert_eq!(reg().detect(b"P6\n3 2\n255\n"), Some(ImageFormat::Pnm));
    }

    #[test]
    fn detect_pnm_p7() {
        assert_eq!(reg().detect(b"P7\nWIDTH 2\n"), Some(ImageFormat::Pnm));
    }

    #[test]
    fn detect_pnm_pfm_color() {
        assert_eq!(reg().detect(b"PF\n3 2\n"), Some(ImageFormat::Pnm));
    }

    #[test]
    fn detect_pnm_pfm_gray() {
        assert_eq!(reg().detect(b"Pf\n3 2\n"), Some(ImageFormat::Pnm));
    }

    #[test]
    fn detect_pnm_p1_ascii() {
        assert_eq!(reg().detect(b"P1\n3 2\n"), Some(ImageFormat::Pnm));
    }

    #[test]
    fn from_extension_pnm_variants() {
        assert_eq!(reg().from_extension("pnm"), Some(ImageFormat::Pnm));
        assert_eq!(reg().from_extension("ppm"), Some(ImageFormat::Pnm));
        assert_eq!(reg().from_extension("pgm"), Some(ImageFormat::Pnm));
        assert_eq!(reg().from_extension("pbm"), Some(ImageFormat::Pnm));
        assert_eq!(reg().from_extension("pam"), Some(ImageFormat::Pnm));
        assert_eq!(reg().from_extension("pfm"), Some(ImageFormat::Pnm));
        assert_eq!(reg().from_extension("PNM"), Some(ImageFormat::Pnm));
    }

    #[test]
    fn pnm_capabilities() {
        assert!(!ImageFormat::Pnm.supports_lossy());
        assert!(ImageFormat::Pnm.supports_lossless());
        assert!(!ImageFormat::Pnm.supports_animation());
        assert!(ImageFormat::Pnm.supports_alpha());
    }

    #[test]
    fn pnm_mime_type() {
        assert_eq!(ImageFormat::Pnm.mime_type(), "image/x-portable-anymap");
    }

    #[test]
    fn pnm_extensions() {
        let exts = ImageFormat::Pnm.extensions();
        assert!(exts.contains(&"pnm"));
        assert!(exts.contains(&"ppm"));
        assert!(exts.contains(&"pgm"));
        assert!(exts.contains(&"pbm"));
        assert!(exts.contains(&"pam"));
        assert!(exts.contains(&"pfm"));
    }

    #[test]
    fn pnm_display() {
        assert_eq!(alloc::format!("{}", ImageFormat::Pnm), "PNM");
    }

    #[test]
    fn pnm_magic_bytes_needed() {
        assert_eq!(ImageFormat::Pnm.magic_bytes_needed(), 20);
    }

    #[test]
    fn detect_bmp() {
        assert_eq!(reg().detect(b"BM\x00\x00"), Some(ImageFormat::Bmp));
    }

    #[test]
    fn detect_farbfeld() {
        assert_eq!(
            reg().detect(b"farbfeld\x00\x00\x00\x01\x00\x00\x00\x01"),
            Some(ImageFormat::Farbfeld)
        );
    }

    #[test]
    fn from_extension_bmp() {
        assert_eq!(reg().from_extension("bmp"), Some(ImageFormat::Bmp));
        assert_eq!(reg().from_extension("BMP"), Some(ImageFormat::Bmp));
    }

    #[test]
    fn from_extension_farbfeld() {
        assert_eq!(reg().from_extension("ff"), Some(ImageFormat::Farbfeld));
    }

    #[test]
    fn bmp_capabilities() {
        assert!(!ImageFormat::Bmp.supports_lossy());
        assert!(ImageFormat::Bmp.supports_lossless());
        assert!(!ImageFormat::Bmp.supports_animation());
        assert!(ImageFormat::Bmp.supports_alpha());
    }

    #[test]
    fn farbfeld_capabilities() {
        assert!(!ImageFormat::Farbfeld.supports_lossy());
        assert!(ImageFormat::Farbfeld.supports_lossless());
        assert!(!ImageFormat::Farbfeld.supports_animation());
        assert!(ImageFormat::Farbfeld.supports_alpha());
    }

    #[test]
    fn bmp_display() {
        assert_eq!(alloc::format!("{}", ImageFormat::Bmp), "BMP");
    }

    #[test]
    fn farbfeld_display() {
        assert_eq!(alloc::format!("{}", ImageFormat::Farbfeld), "farbfeld");
    }

    #[test]
    fn bmp_mime_type() {
        assert_eq!(ImageFormat::Bmp.mime_type(), "image/bmp");
    }

    #[test]
    fn farbfeld_mime_type() {
        assert_eq!(ImageFormat::Farbfeld.mime_type(), "image/x-farbfeld");
    }

    #[test]
    fn bmp_extensions() {
        assert_eq!(ImageFormat::Bmp.extensions(), &["bmp"]);
    }

    #[test]
    fn farbfeld_extensions() {
        assert_eq!(ImageFormat::Farbfeld.extensions(), &["ff"]);
    }

    // --- HEIC tests ---

    #[test]
    fn detect_heic() {
        let mut data = vec![0u8; 20];
        data[0..4].copy_from_slice(&20u32.to_be_bytes());
        data[4..8].copy_from_slice(b"ftyp");
        data[8..12].copy_from_slice(b"heic");
        data[12..16].copy_from_slice(&[0, 0, 0, 0]);
        assert_eq!(reg().detect(&data), Some(ImageFormat::Heic));
    }

    #[test]
    fn detect_heic_heix_brand() {
        let mut data = vec![0u8; 20];
        data[0..4].copy_from_slice(&20u32.to_be_bytes());
        data[4..8].copy_from_slice(b"ftyp");
        data[8..12].copy_from_slice(b"heix");
        data[12..16].copy_from_slice(&[0, 0, 0, 0]);
        assert_eq!(reg().detect(&data), Some(ImageFormat::Heic));
    }

    #[test]
    fn detect_heic_hevc_brand() {
        let mut data = vec![0u8; 20];
        data[0..4].copy_from_slice(&20u32.to_be_bytes());
        data[4..8].copy_from_slice(b"ftyp");
        data[8..12].copy_from_slice(b"hevc");
        data[12..16].copy_from_slice(&[0, 0, 0, 0]);
        assert_eq!(reg().detect(&data), Some(ImageFormat::Heic));
    }

    #[test]
    fn detect_avif_still_works() {
        let mut data = vec![0u8; 20];
        data[0..4].copy_from_slice(&20u32.to_be_bytes());
        data[4..8].copy_from_slice(b"ftyp");
        data[8..12].copy_from_slice(b"avif");
        data[12..16].copy_from_slice(&[0, 0, 0, 0]);
        assert_eq!(reg().detect(&data), Some(ImageFormat::Avif));

        data[8..12].copy_from_slice(b"avis");
        assert_eq!(reg().detect(&data), Some(ImageFormat::Avif));
    }

    #[test]
    fn detect_mif1_with_heic_compat() {
        let mut data = vec![0u8; 24];
        data[0..4].copy_from_slice(&24u32.to_be_bytes());
        data[4..8].copy_from_slice(b"ftyp");
        data[8..12].copy_from_slice(b"mif1");
        data[12..16].copy_from_slice(&[0, 0, 0, 0]);
        data[16..20].copy_from_slice(b"heic");
        data[20..24].copy_from_slice(b"hevx");
        assert_eq!(reg().detect(&data), Some(ImageFormat::Heic));
    }

    #[test]
    fn detect_mif1_with_avif_compat() {
        let mut data = vec![0u8; 24];
        data[0..4].copy_from_slice(&24u32.to_be_bytes());
        data[4..8].copy_from_slice(b"ftyp");
        data[8..12].copy_from_slice(b"mif1");
        data[12..16].copy_from_slice(&[0, 0, 0, 0]);
        data[16..20].copy_from_slice(b"avif");
        data[20..24].copy_from_slice(b"heic");
        assert_eq!(reg().detect(&data), Some(ImageFormat::Avif));
    }

    #[test]
    fn detect_mif1_no_known_compat() {
        let mut data = vec![0u8; 20];
        data[0..4].copy_from_slice(&20u32.to_be_bytes());
        data[4..8].copy_from_slice(b"ftyp");
        data[8..12].copy_from_slice(b"mif1");
        data[12..16].copy_from_slice(&[0, 0, 0, 0]);
        data[16..20].copy_from_slice(b"xxxx");
        assert_eq!(reg().detect(&data), None);
    }

    #[test]
    fn from_extension_heic() {
        assert_eq!(reg().from_extension("heic"), Some(ImageFormat::Heic));
        assert_eq!(reg().from_extension("heif"), Some(ImageFormat::Heic));
        assert_eq!(reg().from_extension("hif"), Some(ImageFormat::Heic));
        assert_eq!(reg().from_extension("HEIC"), Some(ImageFormat::Heic));
        assert_eq!(reg().from_extension("HEIF"), Some(ImageFormat::Heic));
    }

    #[test]
    fn heic_capabilities() {
        assert!(ImageFormat::Heic.supports_lossy());
        assert!(!ImageFormat::Heic.supports_lossless());
        assert!(!ImageFormat::Heic.supports_animation());
        assert!(ImageFormat::Heic.supports_alpha());
    }

    #[test]
    fn heif_display() {
        assert_eq!(alloc::format!("{}", ImageFormat::Heic), "HEIC");
    }

    #[test]
    fn heif_mime_type() {
        assert_eq!(ImageFormat::Heic.mime_type(), "image/heif");
    }

    #[test]
    fn heic_extensions() {
        let exts = ImageFormat::Heic.extensions();
        assert!(exts.contains(&"heic"));
        assert!(exts.contains(&"heif"));
        assert!(exts.contains(&"hif"));
    }

    #[test]
    fn heic_min_probe_bytes() {
        assert_eq!(ImageFormat::Heic.magic_bytes_needed(), 512);
    }

    // --- msf1 HEIF sequence tests ---

    #[test]
    fn detect_msf1_with_heic_compat() {
        let mut data = vec![0u8; 24];
        data[0..4].copy_from_slice(&24u32.to_be_bytes());
        data[4..8].copy_from_slice(b"ftyp");
        data[8..12].copy_from_slice(b"msf1");
        data[12..16].copy_from_slice(&[0, 0, 0, 0]);
        data[16..20].copy_from_slice(b"hevc");
        data[20..24].copy_from_slice(b"heic");
        assert_eq!(reg().detect(&data), Some(ImageFormat::Heic));
    }

    #[test]
    fn detect_msf1_with_avif_compat() {
        let mut data = vec![0u8; 24];
        data[0..4].copy_from_slice(&24u32.to_be_bytes());
        data[4..8].copy_from_slice(b"ftyp");
        data[8..12].copy_from_slice(b"msf1");
        data[12..16].copy_from_slice(&[0, 0, 0, 0]);
        data[16..20].copy_from_slice(b"avis");
        assert_eq!(reg().detect(&data), Some(ImageFormat::Avif));
    }

    // --- Custom format tests ---

    fn detect_test_format(data: &[u8]) -> bool {
        data.len() >= 4 && data[..4] == *b"TEST"
    }

    static TEST_FORMAT: ImageFormatDefinition = ImageFormatDefinition {
        name: "testformat",
        image_format: None,
        display_name: "Test Format",
        preferred_extension: "test",
        extensions: &["test", "tst"],
        preferred_mime_type: "image/x-test",
        mime_types: &["image/x-test", "application/x-test"],
        supports_alpha: true,
        supports_animation: false,
        supports_lossless: true,
        supports_lossy: false,
        magic_bytes_needed: 4,
        detect: detect_test_format,
    };

    static TEST_FORMAT_2: ImageFormatDefinition = ImageFormatDefinition {
        name: "testformat",
        image_format: None,
        display_name: "Test Format 2",
        preferred_extension: "tf2",
        extensions: &["tf2"],
        preferred_mime_type: "image/x-test2",
        mime_types: &["image/x-test2"],
        supports_alpha: false,
        supports_animation: false,
        supports_lossless: false,
        supports_lossy: false,
        magic_bytes_needed: 0,
        detect: |_| false,
    };

    #[test]
    fn custom_format_metadata() {
        let fmt = ImageFormat::Custom(&TEST_FORMAT);
        assert_eq!(fmt.mime_type(), "image/x-test");
        assert_eq!(fmt.mime_types(), &["image/x-test", "application/x-test"]);
        assert_eq!(fmt.extension(), "test");
        assert_eq!(fmt.extensions(), &["test", "tst"]);
        assert!(fmt.supports_alpha());
        assert!(!fmt.supports_animation());
        assert!(fmt.supports_lossless());
        assert!(!fmt.supports_lossy());
        assert_eq!(fmt.magic_bytes_needed(), 4);
    }

    #[test]
    fn custom_format_display() {
        let fmt = ImageFormat::Custom(&TEST_FORMAT);
        assert_eq!(alloc::format!("{fmt}"), "Test Format");
    }

    #[test]
    fn custom_format_detect() {
        assert!((TEST_FORMAT.detect)(b"TESTdata"));
        assert!(!(TEST_FORMAT.detect)(b"NOPE"));
    }

    #[test]
    fn custom_format_equality_by_name() {
        // Same name -> equal, even though other fields differ
        let a = ImageFormat::Custom(&TEST_FORMAT);
        let b = ImageFormat::Custom(&TEST_FORMAT_2);
        assert_eq!(a, b);

        // Different name -> not equal
        static OTHER: ImageFormatDefinition = ImageFormatDefinition {
            name: "other",
            image_format: None,
            display_name: "Other",
            preferred_extension: "oth",
            extensions: &["oth"],
            preferred_mime_type: "image/x-other",
            mime_types: &["image/x-other"],
            supports_alpha: false,
            supports_animation: false,
            supports_lossless: false,
            supports_lossy: false,
            magic_bytes_needed: 0,
            detect: |_| false,
        };
        assert_ne!(a, ImageFormat::Custom(&OTHER));
    }

    #[test]
    fn custom_format_hash() {
        use core::hash::{Hash, Hasher};
        struct SimpleHasher(u64);
        impl Hasher for SimpleHasher {
            fn finish(&self) -> u64 {
                self.0
            }
            fn write(&mut self, bytes: &[u8]) {
                for &b in bytes {
                    self.0 = self.0.wrapping_mul(31).wrapping_add(b as u64);
                }
            }
        }
        fn hash_of(fmt: ImageFormat) -> u64 {
            let mut hasher = SimpleHasher(0);
            fmt.hash(&mut hasher);
            hasher.finish()
        }
        // Same name -> same hash
        assert_eq!(
            hash_of(ImageFormat::Custom(&TEST_FORMAT)),
            hash_of(ImageFormat::Custom(&TEST_FORMAT_2))
        );
    }

    #[test]
    fn custom_not_equal_to_builtin() {
        let custom = ImageFormat::Custom(&TEST_FORMAT);
        assert_ne!(custom, ImageFormat::Jpeg);
        assert_ne!(custom, ImageFormat::Unknown);
    }

    #[test]
    fn custom_format_is_copy() {
        let a = ImageFormat::Custom(&TEST_FORMAT);
        let b = a; // Copy
        assert_eq!(a, b);
    }

    #[test]
    fn to_image_format_builtin() {
        let fmt = builtins::JPEG.to_image_format();
        assert_eq!(fmt, ImageFormat::Jpeg);
    }

    #[test]
    fn to_image_format_custom() {
        let fmt = TEST_FORMAT.to_image_format();
        assert_eq!(fmt, ImageFormat::Custom(&TEST_FORMAT));
    }

    // --- from_mime_type tests ---

    #[test]
    fn from_mime_type_builtin() {
        assert_eq!(reg().from_mime_type("image/jpeg"), Some(ImageFormat::Jpeg));
        assert_eq!(reg().from_mime_type("image/heif"), Some(ImageFormat::Heic));
        assert_eq!(reg().from_mime_type("image/heic"), Some(ImageFormat::Heic));
        assert_eq!(reg().from_mime_type("video/mp4"), None);
    }

    // --- Registry tests ---

    #[test]
    fn registry_common_detect() {
        let reg = ImageFormatRegistry::common();
        assert_eq!(
            reg.detect(&[0xFF, 0xD8, 0xFF, 0xE0]),
            Some(ImageFormat::Jpeg)
        );
        assert_eq!(
            reg.detect(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]),
            Some(ImageFormat::Png)
        );
        assert_eq!(reg.detect(b"nope"), None);
    }

    #[test]
    fn registry_common_from_extension() {
        let reg = ImageFormatRegistry::common();
        assert_eq!(reg.from_extension("jpg"), Some(ImageFormat::Jpeg));
        assert_eq!(reg.from_extension("PNG"), Some(ImageFormat::Png));
        assert_eq!(reg.from_extension("unknown"), None);
    }

    #[test]
    fn registry_common_from_mime_type() {
        let reg = ImageFormatRegistry::common();
        assert_eq!(reg.from_mime_type("image/jpeg"), Some(ImageFormat::Jpeg));
        assert_eq!(reg.from_mime_type("image/webp"), Some(ImageFormat::WebP));
        assert_eq!(reg.from_mime_type("video/mp4"), None);
    }

    fn reg_with_test_format() -> ImageFormatRegistry {
        let mut defs: Vec<&'static ImageFormatDefinition> = builtins::ALL.to_vec();
        defs.push(&TEST_FORMAT);
        ImageFormatRegistry::from_vec(defs)
    }

    #[test]
    fn registry_from_vec_custom() {
        let reg = reg_with_test_format();
        // Custom format detected
        assert_eq!(
            reg.detect(b"TESTdata"),
            Some(ImageFormat::Custom(&TEST_FORMAT))
        );
        // Built-in still works
        assert_eq!(reg.detect(&[0xFF, 0xD8, 0xFF]), Some(ImageFormat::Jpeg));
    }

    #[test]
    fn registry_from_vec_custom_extension() {
        let reg = reg_with_test_format();
        assert_eq!(
            reg.from_extension("test"),
            Some(ImageFormat::Custom(&TEST_FORMAT))
        );
        assert_eq!(
            reg.from_extension("TST"),
            Some(ImageFormat::Custom(&TEST_FORMAT))
        );
        assert_eq!(reg.from_extension("jpg"), Some(ImageFormat::Jpeg));
    }

    #[test]
    fn registry_from_vec_custom_mime_type() {
        let reg = reg_with_test_format();
        assert_eq!(
            reg.from_mime_type("image/x-test"),
            Some(ImageFormat::Custom(&TEST_FORMAT))
        );
        assert_eq!(
            reg.from_mime_type("application/x-test"),
            Some(ImageFormat::Custom(&TEST_FORMAT))
        );
    }

    #[test]
    fn registry_from_static() {
        static DEFS: &[&ImageFormatDefinition] = &[&builtins::PNG, &builtins::JPEG];
        let reg = ImageFormatRegistry::from_static(DEFS);
        assert_eq!(reg.formats().len(), 2);
        assert_eq!(
            reg.detect(&[0xFF, 0xD8, 0xFF, 0xE0]),
            Some(ImageFormat::Jpeg)
        );
        assert_eq!(reg.detect(b"GIF89a\x00\x00"), None); // GIF not in this registry
    }

    #[test]
    fn registry_from_static_custom_only() {
        static DEFS: &[&ImageFormatDefinition] = &[&TEST_FORMAT];
        let reg = ImageFormatRegistry::from_static(DEFS);
        assert_eq!(
            reg.detect(b"TESTdata"),
            Some(ImageFormat::Custom(&TEST_FORMAT))
        );
        assert_eq!(reg.detect(&[0xFF, 0xD8, 0xFF]), None); // no JPEG
        assert_eq!(reg.formats().len(), 1);
    }

    #[test]
    fn registry_formats_list() {
        let reg = ImageFormatRegistry::common();
        assert_eq!(reg.formats().len(), 13);
        assert_eq!(reg.formats()[0].name, "jpeg");
    }

    #[test]
    fn registry_default_is_common() {
        let def = ImageFormatRegistry::default();
        let com = ImageFormatRegistry::common();
        assert_eq!(def.formats().len(), com.formats().len());
    }

    #[test]
    fn registry_new_from_vec() {
        let reg = ImageFormatRegistry::from_vec(vec![&builtins::PNG, &builtins::JPEG]);
        assert_eq!(reg.formats().len(), 2);
        // PNG is first, so PNG-like data matches PNG
        assert_eq!(
            reg.detect(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]),
            Some(ImageFormat::Png)
        );
        assert_eq!(reg.detect(&[0xFF, 0xD8, 0xFF]), Some(ImageFormat::Jpeg));
        // GIF not in registry
        assert_eq!(reg.detect(b"GIF89a\x00\x00"), None);
    }
}
