//! Per-job security policy flags for decode and encode operations.
//!
//! These control what a codec is allowed to do on a given job.
//! All fields default to `None`, meaning the codec uses its own default.
//! `Some(true)` explicitly allows; `Some(false)` explicitly denies.
//!
//! # Named levels
//!
//! - [`DecodePolicy::none()`] / [`EncodePolicy::none()`] — all defaults
//! - [`DecodePolicy::strict()`] — minimal attack surface (no metadata, no progressive, no animation)
//! - [`DecodePolicy::permissive()`] — allow everything
//!
//! Individual flags can be overridden after constructing a named level.

/// Decode security policy.
///
/// Controls what features a decoder is permitted to use when processing
/// untrusted input. Codecs check these flags and skip or reject
/// accordingly; unrecognized flags are ignored.
///
/// # Example
///
/// ```
/// use zc::decode::DecodePolicy;
///
/// // Start strict, then allow ICC (needed for color management)
/// let policy = DecodePolicy::strict().with_allow_icc(true);
/// assert_eq!(policy.allow_icc, Some(true));
/// assert_eq!(policy.allow_exif, Some(false));
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct DecodePolicy {
    /// Extract ICC color profiles. When `Some(false)`, the decoder
    /// skips ICC parsing and returns no profile in [`ImageInfo`](crate::ImageInfo).
    pub allow_icc: Option<bool>,
    /// Extract EXIF metadata.
    pub allow_exif: Option<bool>,
    /// Extract XMP metadata.
    pub allow_xmp: Option<bool>,
    /// Allow progressive / interlaced images.
    /// When `Some(false)`, the decoder rejects progressive input.
    pub allow_progressive: Option<bool>,
    /// Allow multi-frame / animated images.
    /// When `Some(false)`, only the first frame is decoded.
    pub allow_animation: Option<bool>,
    /// Accept truncated or partially corrupt input.
    /// When `Some(true)`, the decoder returns whatever it decoded so far.
    pub allow_truncated: Option<bool>,
    /// Strict spec compliance.
    /// When `Some(true)`, reject non-conformant inputs that would
    /// otherwise be accepted with workarounds.
    pub strict: Option<bool>,
}

impl DecodePolicy {
    /// No preferences — codec uses its own defaults.
    pub const fn none() -> Self {
        Self {
            allow_icc: None,
            allow_exif: None,
            allow_xmp: None,
            allow_progressive: None,
            allow_animation: None,
            allow_truncated: None,
            strict: None,
        }
    }

    /// Minimal attack surface: no metadata extraction, no progressive,
    /// no animation, strict parsing.
    pub const fn strict() -> Self {
        Self {
            allow_icc: Some(false),
            allow_exif: Some(false),
            allow_xmp: Some(false),
            allow_progressive: Some(false),
            allow_animation: Some(false),
            allow_truncated: Some(false),
            strict: Some(true),
        }
    }

    /// Allow everything.
    pub const fn permissive() -> Self {
        Self {
            allow_icc: Some(true),
            allow_exif: Some(true),
            allow_xmp: Some(true),
            allow_progressive: Some(true),
            allow_animation: Some(true),
            allow_truncated: Some(true),
            strict: Some(false),
        }
    }

    /// Override ICC profile extraction.
    pub const fn with_allow_icc(mut self, v: bool) -> Self {
        self.allow_icc = Some(v);
        self
    }

    /// Override EXIF extraction.
    pub const fn with_allow_exif(mut self, v: bool) -> Self {
        self.allow_exif = Some(v);
        self
    }

    /// Override XMP extraction.
    pub const fn with_allow_xmp(mut self, v: bool) -> Self {
        self.allow_xmp = Some(v);
        self
    }

    /// Override progressive/interlaced support.
    pub const fn with_allow_progressive(mut self, v: bool) -> Self {
        self.allow_progressive = Some(v);
        self
    }

    /// Override animation support.
    pub const fn with_allow_animation(mut self, v: bool) -> Self {
        self.allow_animation = Some(v);
        self
    }

    /// Override truncated input handling.
    pub const fn with_allow_truncated(mut self, v: bool) -> Self {
        self.allow_truncated = Some(v);
        self
    }

    /// Override strict parsing.
    pub const fn with_strict(mut self, v: bool) -> Self {
        self.strict = Some(v);
        self
    }

    /// Resolve a flag: return the explicit value, or fall back to `default`.
    pub const fn resolve_icc(&self, default: bool) -> bool {
        match self.allow_icc {
            Some(v) => v,
            None => default,
        }
    }

    /// Resolve EXIF flag.
    pub const fn resolve_exif(&self, default: bool) -> bool {
        match self.allow_exif {
            Some(v) => v,
            None => default,
        }
    }

    /// Resolve XMP flag.
    pub const fn resolve_xmp(&self, default: bool) -> bool {
        match self.allow_xmp {
            Some(v) => v,
            None => default,
        }
    }

    /// Resolve progressive flag.
    pub const fn resolve_progressive(&self, default: bool) -> bool {
        match self.allow_progressive {
            Some(v) => v,
            None => default,
        }
    }

    /// Resolve animation flag.
    pub const fn resolve_animation(&self, default: bool) -> bool {
        match self.allow_animation {
            Some(v) => v,
            None => default,
        }
    }

    /// Resolve truncated flag.
    pub const fn resolve_truncated(&self, default: bool) -> bool {
        match self.allow_truncated {
            Some(v) => v,
            None => default,
        }
    }

    /// Resolve strict flag.
    pub const fn resolve_strict(&self, default: bool) -> bool {
        match self.strict {
            Some(v) => v,
            None => default,
        }
    }
}

/// Encode metadata policy.
///
/// Controls which metadata an encoder embeds in the output.
/// All fields default to `None`, meaning the codec uses its own default.
/// `Some(true)` explicitly allows embedding; `Some(false)` explicitly strips.
///
/// # Example
///
/// ```
/// use zc::encode::EncodePolicy;
///
/// // Strip all metadata from output
/// let policy = EncodePolicy::strip_all();
///
/// // Or fine-grained: keep ICC, strip EXIF/XMP
/// let policy = EncodePolicy::none()
///     .with_embed_icc(true)
///     .with_embed_exif(false)
///     .with_embed_xmp(false);
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct EncodePolicy {
    /// Embed ICC color profiles in the output.
    pub embed_icc: Option<bool>,
    /// Embed EXIF metadata in the output.
    pub embed_exif: Option<bool>,
    /// Embed XMP metadata in the output.
    pub embed_xmp: Option<bool>,
}

impl EncodePolicy {
    /// No preferences — codec uses its own defaults.
    pub const fn none() -> Self {
        Self {
            embed_icc: None,
            embed_exif: None,
            embed_xmp: None,
        }
    }

    /// Strip all metadata from output.
    pub const fn strip_all() -> Self {
        Self {
            embed_icc: Some(false),
            embed_exif: Some(false),
            embed_xmp: Some(false),
        }
    }

    /// Preserve all metadata in output.
    pub const fn preserve_all() -> Self {
        Self {
            embed_icc: Some(true),
            embed_exif: Some(true),
            embed_xmp: Some(true),
        }
    }

    /// Override ICC embedding.
    pub const fn with_embed_icc(mut self, v: bool) -> Self {
        self.embed_icc = Some(v);
        self
    }

    /// Override EXIF embedding.
    pub const fn with_embed_exif(mut self, v: bool) -> Self {
        self.embed_exif = Some(v);
        self
    }

    /// Override XMP embedding.
    pub const fn with_embed_xmp(mut self, v: bool) -> Self {
        self.embed_xmp = Some(v);
        self
    }

    /// Resolve ICC embedding flag.
    pub const fn resolve_icc(&self, default: bool) -> bool {
        match self.embed_icc {
            Some(v) => v,
            None => default,
        }
    }

    /// Resolve EXIF embedding flag.
    pub const fn resolve_exif(&self, default: bool) -> bool {
        match self.embed_exif {
            Some(v) => v,
            None => default,
        }
    }

    /// Resolve XMP embedding flag.
    pub const fn resolve_xmp(&self, default: bool) -> bool {
        match self.embed_xmp {
            Some(v) => v,
            None => default,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_none_is_all_none() {
        let p = DecodePolicy::none();
        assert_eq!(p.allow_icc, None);
        assert_eq!(p.allow_exif, None);
        assert_eq!(p.allow_xmp, None);
        assert_eq!(p.allow_progressive, None);
        assert_eq!(p.allow_animation, None);
        assert_eq!(p.allow_truncated, None);
        assert_eq!(p.strict, None);
    }

    #[test]
    fn decode_strict_denies_all() {
        let p = DecodePolicy::strict();
        assert_eq!(p.allow_icc, Some(false));
        assert_eq!(p.allow_exif, Some(false));
        assert_eq!(p.allow_animation, Some(false));
        assert_eq!(p.strict, Some(true));
    }

    #[test]
    fn decode_permissive_allows_all() {
        let p = DecodePolicy::permissive();
        assert_eq!(p.allow_icc, Some(true));
        assert_eq!(p.allow_truncated, Some(true));
        assert_eq!(p.strict, Some(false));
    }

    #[test]
    fn decode_builder_overrides() {
        let p = DecodePolicy::strict().with_allow_icc(true);
        assert_eq!(p.allow_icc, Some(true));
        assert_eq!(p.allow_exif, Some(false)); // still strict
    }

    #[test]
    fn decode_resolve_with_default() {
        let p = DecodePolicy::none();
        assert!(p.resolve_icc(true));
        assert!(!p.resolve_icc(false));

        let p = DecodePolicy::strict();
        assert!(!p.resolve_icc(true)); // explicit false overrides default true
    }

    #[test]
    fn encode_none_is_all_none() {
        let p = EncodePolicy::none();
        assert_eq!(p.embed_icc, None);
        assert_eq!(p.embed_exif, None);
        assert_eq!(p.embed_xmp, None);
    }

    #[test]
    fn encode_strip_all() {
        let p = EncodePolicy::strip_all();
        assert_eq!(p.embed_icc, Some(false));
        assert_eq!(p.embed_exif, Some(false));
        assert_eq!(p.embed_xmp, Some(false));
    }

    #[test]
    fn encode_preserve_all() {
        let p = EncodePolicy::preserve_all();
        assert_eq!(p.embed_icc, Some(true));
        assert_eq!(p.embed_exif, Some(true));
        assert_eq!(p.embed_xmp, Some(true));
    }

    #[test]
    fn encode_builder_overrides() {
        let p = EncodePolicy::strip_all().with_embed_icc(true);
        assert_eq!(p.embed_icc, Some(true));
        assert_eq!(p.embed_exif, Some(false)); // still stripped
    }

    #[test]
    fn encode_resolve_with_default() {
        let p = EncodePolicy::none();
        assert!(p.resolve_icc(true));
        assert!(!p.resolve_icc(false));

        let p = EncodePolicy::strip_all();
        assert!(!p.resolve_icc(true));
    }

    #[test]
    fn static_construction() {
        static _DECODE: DecodePolicy = DecodePolicy::strict().with_allow_icc(true);
        static _ENCODE: EncodePolicy = EncodePolicy::strip_all().with_embed_icc(true);
    }

    #[test]
    fn default_is_none() {
        assert_eq!(DecodePolicy::default(), DecodePolicy::none());
        assert_eq!(EncodePolicy::default(), EncodePolicy::none());
    }
}
