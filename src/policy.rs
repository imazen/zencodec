//! Per-job security policy flags for decode and encode operations.
//!
//! These control what a codec is allowed to do on a given job.
//! All fields default to `None`, meaning the codec uses its own default.
//! `Some(true)` explicitly allows; `Some(false)` explicitly denies.
//!
//! # Choosing a starting point
//!
//! For **untrusted input** (network bytes, user uploads, third-party data),
//! prefer [`DecodePolicy::strict()`] as the starting point and selectively
//! re-enable features the application actually needs. This is the
//! recommended default for any service that processes bytes it did not
//! produce itself. Pair this with
//! [`ResourceLimits::for_untrusted_input`](crate::ResourceLimits::for_untrusted_input)
//! for resource caps.
//!
//! For **trusted input** (your own pipeline, internal tools), use
//! [`DecodePolicy::none()`] (all defaults) or [`DecodePolicy::permissive()`]
//! to keep all features available.
//!
//! # Named levels
//!
//! - [`DecodePolicy::strict()`] — **recommended for untrusted input.**
//!   Minimal attack surface: no ICC/EXIF/XMP extraction, no progressive,
//!   no animation, strict spec parsing, no truncated input.
//! - [`DecodePolicy::none()`] / [`EncodePolicy::none()`] — all defaults
//!   (each codec picks its own behavior).
//! - [`DecodePolicy::permissive()`] — allow everything (use only for
//!   trusted input).
//!
//! Individual flags can be overridden after constructing a named level
//! — e.g. `DecodePolicy::strict().with_allow_icc(true)` for strict-but-with-color.

/// Decode security policy.
///
/// Controls what features a decoder is permitted to use when processing
/// untrusted input. Codecs check these flags and skip or reject
/// accordingly; unrecognized flags are ignored.
///
/// # Recommended: start strict for untrusted input
///
/// When decoding bytes from the network, end users, or any third-party
/// source, use [`DecodePolicy::strict()`] as the starting point and
/// selectively enable the features the application actually needs.
/// `DecodePolicy::default()` returns [`DecodePolicy::none()`] (all
/// `None` — each codec's own default applies) for backwards compatibility,
/// but this is **not** the safest choice for untrusted input.
///
/// # Example
///
/// ```
/// use zencodec::decode::DecodePolicy;
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

// All Option<bool>, no pointers — same size on all platforms.
const _: () = assert!(core::mem::size_of::<DecodePolicy>() == 7);

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

/// Output-emission policy for an encode or transcode: which color carrier to
/// emit, which metadata to retain, and a coarse per-channel embed gate. One
/// object, three concerns that apply at different stages.
///
/// - [`color`](Self::color) — color-carrier emission (ICC bytes vs CICP code
///   points). The codec reads it during encode via
///   [`resolve_color`](Self::resolve_color) and feeds it to
///   [`resolve_color_emit`](crate::resolve_color_emit). `None` defers to the
///   codec's default.
/// - [`metadata`](Self::metadata) — field-level retention (which EXIF tags, a
///   redundant sRGB ICC, XMP, CICP/HDR signaling). Applied by the pipeline or
///   caller via [`Metadata::filtered`](crate::Metadata::filtered) *before* the
///   record reaches the codec, so it is always honored; codecs do not read this
///   field. `None` leaves the record unfiltered.
/// - [`embed_icc`](Self::embed_icc) / [`embed_exif`](Self::embed_exif) /
///   [`embed_xmp`](Self::embed_xmp) — a coarse, best-effort per-channel embed
///   gate handed to the codec via
///   [`EncodeJob::with_policy`](crate::encode::EncodeJob::with_policy).
///   Tri-state (`None` = codec default, `Some(true/false)` = embed/strip),
///   whole-channel only. Best-effort: the `with_policy` default is a no-op, so a
///   codec that does not implement it silently ignores this gate. For reliable
///   retention use `metadata`, not these.
///
/// # Example
///
/// ```
/// use zencodec::encode::EncodePolicy;
/// use zencodec::{ColorEmitPolicy, MetadataPolicy};
///
/// // Smallest output: prefer compact color carriers, keep only color + rotation.
/// let policy = EncodePolicy::none()
///     .with_color(ColorEmitPolicy::Compact)
///     .with_metadata_policy(MetadataPolicy::ColorAndRotation);
///
/// // Coarse legacy gate: ask the codec to strip every metadata channel.
/// let policy = EncodePolicy::strip_all();
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct EncodePolicy {
    /// Color-carrier emission policy (ICC bytes vs CICP code points). `None`
    /// defers to the codec's default. The codec reads it during encode via
    /// [`resolve_color`](EncodePolicy::resolve_color).
    pub color: Option<crate::ColorEmitPolicy>,
    /// Field-level metadata retention. `None` leaves the record unfiltered.
    /// Applied by the pipeline/caller via
    /// [`Metadata::filtered`](crate::Metadata::filtered) before encode; codecs
    /// do not read this field.
    pub metadata: Option<crate::MetadataPolicy>,
    /// Embed ICC color profiles in the output.
    pub embed_icc: Option<bool>,
    /// Embed EXIF metadata in the output.
    pub embed_exif: Option<bool>,
    /// Embed XMP metadata in the output.
    pub embed_xmp: Option<bool>,
}

// No longer a 3-byte gate: EncodePolicy now bundles the color + metadata
// policies. Keep a loose upper bound to catch accidental bloat (e.g. a field
// that pulls in a Vec/Arc).
const _: () = assert!(core::mem::size_of::<EncodePolicy>() <= 32);

impl EncodePolicy {
    /// No preferences — codec uses its own defaults.
    pub const fn none() -> Self {
        Self {
            color: None,
            metadata: None,
            embed_icc: None,
            embed_exif: None,
            embed_xmp: None,
        }
    }

    /// Strip all metadata from output.
    pub const fn strip_all() -> Self {
        Self {
            color: None,
            // Carry a real discard policy through the reliable metadata channel
            // (`Metadata::filtered` / `resolve_metadata`), not only the advisory
            // `embed_*` flags — the latter silently no-op on codecs that don't
            // implement `with_policy`, so a strip via flags alone could leak.
            metadata: Some(crate::MetadataPolicy::Custom(
                crate::MetadataFields::DISCARD_ALL,
            )),
            embed_icc: Some(false),
            embed_exif: Some(false),
            embed_xmp: Some(false),
        }
    }

    /// Preserve all metadata in output.
    pub const fn preserve_all() -> Self {
        Self {
            color: None,
            metadata: Some(crate::MetadataPolicy::PreserveExact),
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

    /// Set the color-carrier emission policy.
    pub const fn with_color(mut self, policy: crate::ColorEmitPolicy) -> Self {
        self.color = Some(policy);
        self
    }

    /// Set the field-level metadata retention policy.
    pub const fn with_metadata_policy(mut self, policy: crate::MetadataPolicy) -> Self {
        self.metadata = Some(policy);
        self
    }

    /// Resolve the color-carrier emission policy, falling back to `default` —
    /// codecs pass their own default here (e.g.
    /// [`ColorEmitPolicy::Balanced`](crate::ColorEmitPolicy::Balanced)), so a
    /// caller that set nothing keeps the codec's behavior.
    pub const fn resolve_color(&self, default: crate::ColorEmitPolicy) -> crate::ColorEmitPolicy {
        match self.color {
            Some(p) => p,
            None => default,
        }
    }

    /// Resolve the metadata retention policy, falling back to `default`.
    pub const fn resolve_metadata(&self, default: crate::MetadataPolicy) -> crate::MetadataPolicy {
        match self.metadata {
            Some(p) => p,
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
        // Reliable channel: strip_all carries a real discard policy, so a
        // pipeline applying `resolve_metadata` actually strips even when the
        // advisory embed_* flags are a no-op on the codec.
        assert_eq!(
            p.resolve_metadata(crate::MetadataPolicy::Web),
            crate::MetadataPolicy::Custom(crate::MetadataFields::DISCARD_ALL)
        );
    }

    #[test]
    fn encode_preserve_all() {
        let p = EncodePolicy::preserve_all();
        assert_eq!(p.embed_icc, Some(true));
        assert_eq!(p.embed_exif, Some(true));
        assert_eq!(p.embed_xmp, Some(true));
        assert_eq!(
            p.resolve_metadata(crate::MetadataPolicy::Web),
            crate::MetadataPolicy::PreserveExact
        );
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
