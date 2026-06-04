//! Owned metadata for encode/decode roundtrip.
//!
//! [`Metadata`] carries ICC, EXIF, XMP, CICP, HDR, and orientation data
//! using `Arc<[u8]>` for byte buffers (cheap cloning via ref-count bump).
//!
//! # Forward compatibility
//!
//! This surface is shaped so it never needs a semver-major break. Every
//! growable record ([`Metadata`], [`MetadataFields`], [`ExifPolicy`]) is
//! `#[non_exhaustive]` and built from a constructor plus `with_*` setters, and
//! every disposition enum ([`MetadataPolicy`], [`IccRetention`],
//! [`Retention`](crate::exif::Retention)) is `#[non_exhaustive]` — so new
//! record fields and new enum variants both land additively, and downstream
//! cannot struct-literal or exhaustively match these types. Query
//! [`Retention`](crate::exif::Retention) via
//! [`keeps`](crate::exif::Retention::keeps) / `discards` rather than matching,
//! so callers stay correct as variants are added.
//!
//! Anticipated additive growth (each a new field or variant, never a break):
//! partial-XMP retention beside the whole-segment `xmp` switch, gain-map and
//! depth-map retention, new [`ExifPolicy`] categories, new [`IccRetention`]
//! modes, and new color-signaling fields on [`Metadata`] /
//! [`SourceColor`](crate::SourceColor).
//!
//! The known cross-codec carrier gaps (imazen/zenpipe#36 —
//! `Metadata::orientation` emission, decode-side EXIF-orientation
//! normalization, CICP wiring for native-carrier formats, and AVIF EXIF-blob
//! preservation) are fixable as behavioral changes in the codec adapters:
//! [`Metadata`] already models every value those fixes produce
//! ([`orientation`](Metadata::orientation), [`cicp`](Metadata::cicp),
//! [`exif`](Metadata::exif)), so none require a type, field, or signature
//! change here.

use alloc::sync::Arc;

use crate::Orientation;
use crate::exif::{ExifPolicy, Retention};
use crate::info::{Cicp, ContentLightLevel, MasteringDisplay};
use zenpixels::{ColorPrimaries, TransferFunction};

/// Owned image metadata for encode/decode roundtrip.
///
/// Byte buffers (ICC, EXIF, XMP) use `Arc<[u8]>` so cloning is a cheap
/// ref-count bump. Construct via [`Metadata::none()`] + builders,
/// or extract from decoded info via `From<&ImageInfo>`.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct Metadata {
    /// ICC color profile.
    pub icc_profile: Option<Arc<[u8]>>,
    /// EXIF metadata.
    pub exif: Option<Arc<[u8]>>,
    /// XMP metadata.
    pub xmp: Option<Arc<[u8]>>,
    /// CICP color description.
    pub cicp: Option<Cicp>,
    /// Content Light Level Info for HDR content.
    pub content_light_level: Option<ContentLightLevel>,
    /// Mastering Display Color Volume for HDR content.
    pub mastering_display: Option<MasteringDisplay>,
    /// EXIF orientation.
    pub orientation: Orientation,
    /// Embed-time retention policy: how [`for_embedding`](Self::for_embedding)
    /// (and [`filtered`](Self::filtered) by default) prunes this metadata before
    /// a codec writes it. Defaults to [`MetadataPolicy::Web`] — privacy-safe:
    /// GPS, timestamps, camera/device identity, the thumbnail, and XMP are
    /// dropped, orientation + rights + color signaling kept. Set
    /// [`PreserveExact`](MetadataPolicy::PreserveExact) via
    /// [`with_policy`](Self::with_policy) to embed the carried bytes verbatim.
    ///
    /// This field carries *intent* only — the carried `exif`/`xmp`/`icc_profile`
    /// bytes are untouched until [`for_embedding`](Self::for_embedding) applies
    /// the policy, so inspection/round-trip via an external EXIF library still
    /// sees the original bytes.
    pub policy: MetadataPolicy,
}

// Metadata contains 3× Option<Arc<[u8]>> (fat pointers), so size varies by
// pointer width. Catch unexpected growth from new fields or alignment changes.
#[cfg(target_pointer_width = "64")]
const _: () = assert!(core::mem::size_of::<Metadata>() == 120);

impl Metadata {
    /// Create empty metadata.
    pub fn none() -> Self {
        Self::default()
    }

    /// Set the ICC color profile.
    ///
    /// Accepts `Vec<u8>`, `&[u8]`, or `Arc<[u8]>`.
    pub fn with_icc(mut self, icc: impl Into<Arc<[u8]>>) -> Self {
        self.icc_profile = Some(icc.into());
        self
    }

    /// Set the EXIF metadata.
    ///
    /// Accepts `Vec<u8>`, `&[u8]`, or `Arc<[u8]>`.
    ///
    /// As a convenience, the Orientation tag (0x0112) is parsed from the
    /// blob and stored in `self.orientation` — but only if `self.orientation`
    /// is currently `Identity` (the default). Callers who set orientation
    /// explicitly via [`with_orientation`](Self::with_orientation) before
    /// `with_exif` keep their explicit value; callers who set it after
    /// also override the parsed one.
    pub fn with_exif(mut self, exif: impl Into<Arc<[u8]>>) -> Self {
        let bytes: Arc<[u8]> = exif.into();
        if self.orientation == Orientation::Identity
            && let Some(o) = parse_exif_orientation(&bytes)
        {
            self.orientation = o;
        }
        self.exif = Some(bytes);
        self
    }

    /// Set the XMP metadata.
    ///
    /// Accepts `Vec<u8>`, `&[u8]`, or `Arc<[u8]>`.
    pub fn with_xmp(mut self, xmp: impl Into<Arc<[u8]>>) -> Self {
        self.xmp = Some(xmp.into());
        self
    }

    /// Set the CICP color description.
    pub fn with_cicp(mut self, cicp: Cicp) -> Self {
        self.cicp = Some(cicp);
        self
    }

    /// Set the Content Light Level Info.
    pub fn with_content_light_level(mut self, clli: ContentLightLevel) -> Self {
        self.content_light_level = Some(clli);
        self
    }

    /// Set the Mastering Display Color Volume.
    pub fn with_mastering_display(mut self, mdcv: MasteringDisplay) -> Self {
        self.mastering_display = Some(mdcv);
        self
    }

    /// Set the EXIF orientation.
    pub fn with_orientation(mut self, orientation: Orientation) -> Self {
        self.orientation = orientation;
        self
    }

    /// Set the embed-time retention [`policy`](Self::policy).
    ///
    /// Defaults to [`MetadataPolicy::Web`] (privacy-safe). Use
    /// [`PreserveExact`](MetadataPolicy::PreserveExact) to embed the carried
    /// metadata verbatim, or any other [`MetadataPolicy`] for finer control.
    #[must_use]
    pub fn with_policy(mut self, policy: MetadataPolicy) -> Self {
        self.policy = policy;
        self
    }

    /// The metadata a codec should actually embed: `self` pruned by its own
    /// [`policy`](Self::policy) — equivalent to
    /// [`self.filtered(&self.policy)`](Self::filtered).
    ///
    /// This is the zencodec filtering hook a codec calls inside its
    /// `EncodeJob::with_metadata` so embedding honors the caller's privacy intent
    /// without the codec implementing any EXIF logic of its own:
    ///
    /// ```ignore
    /// fn with_metadata(mut self, meta: Metadata) -> Self {
    ///     self.metadata = Some(meta.for_embedding()); // honors meta.policy
    ///     self
    /// }
    /// ```
    ///
    /// The returned metadata carries [`MetadataPolicy::PreserveExact`], so it is
    /// already final — embedding it (or calling `for_embedding` again) never
    /// strips twice.
    #[must_use]
    pub fn for_embedding(&self) -> Metadata {
        self.filtered(&self.policy)
    }

    /// Whether any metadata is present.
    pub fn is_empty(&self) -> bool {
        self.icc_profile.is_none()
            && self.exif.is_none()
            && self.xmp.is_none()
            && self.cicp.is_none()
            && self.content_light_level.is_none()
            && self.mastering_display.is_none()
            && self.orientation == Orientation::Identity
    }

    /// Derive the transfer function from CICP metadata.
    ///
    /// Returns the [`TransferFunction`] corresponding to the CICP
    /// `transfer_characteristics` code, or [`Unknown`](TransferFunction::Unknown)
    /// if CICP is absent or the code is not recognized.
    pub fn transfer_function(&self) -> TransferFunction {
        self.cicp
            .and_then(|c| TransferFunction::from_cicp(c.transfer_characteristics))
            .unwrap_or(TransferFunction::Unknown)
    }

    /// Derive the color primaries from CICP metadata.
    ///
    /// Returns [`Bt709`](ColorPrimaries::Bt709) if CICP is absent.
    pub fn color_primaries(&self) -> ColorPrimaries {
        self.cicp
            .map(|c| c.color_primaries_enum())
            .unwrap_or(ColorPrimaries::Bt709)
    }

    /// Apply a retention [`MetadataPolicy`], returning a filtered copy.
    ///
    /// The shared field-level metadata filter for re-encode / recompress
    /// pipelines: keep what a downstream image needs, strip the rest, without
    /// callers hand-parsing EXIF.
    ///
    /// - **ICC** is three-way ([`IccRetention`]): keep as-is, keep only when
    ///   it isn't a redundant sRGB ([`zenpixels::icc::is_common_srgb`]), or drop.
    /// - **EXIF** is pruned by category via [`ExifPolicy`]. The source blob
    ///   passes through unchanged (zero-copy `Arc` clone) when no category is
    ///   dropped and the embedded orientation already matches the field, and is
    ///   rewritten — offsets recomputed — only when pruning.
    /// - **Orientation** is reconciled: the embedded EXIF orientation tag is
    ///   rewritten to match the authoritative [`orientation`](Metadata::orientation)
    ///   field, so a baked-upright image (field `Identity`, blob still rotated)
    ///   cannot be double-rotated by a consumer that re-applies the tag.
    /// - **CICP** and **HDR** light-level/mastering are color *signaling* (they
    ///   change how pixels display); the presets keep them, a
    ///   [`Custom`](MetadataPolicy::Custom) policy can drop them.
    ///
    /// # HDR signaling and gain maps — keep these consistent with the pixels
    ///
    /// CICP (`transfer_characteristics`, `color_primaries`, `matrix_coefficients`)
    /// and the HDR `ContentLightLevel` / `MasteringDisplay` describe **how the
    /// stored pixels are to be interpreted**. They are not free-floating notes:
    /// a decoder uses CICP transfer (e.g. PQ or HLG) to linearize, and uses
    /// CLLI/MDCV to tone-map for the target display. If they disagree with the
    /// actual pixels, the image renders **wrong** (clipped highlights, wrong
    /// gamut, double tone-mapping).
    ///
    /// A **gain map** is a *separate plane* (not a field of [`Metadata`] — it
    /// lives at the encode-request / codec-output layer with its
    /// [`GainMapInfo`](crate::GainMapInfo)). The base image, its HDR signaling,
    /// and the gain map together reconstruct the HDR rendition. That coupling
    /// is the hazard:
    ///
    /// - **Dropping or flattening the gain map (HDR → SDR) without also fixing
    ///   the signaling leaves invalid metadata.** If you tone-map to an SDR
    ///   base and discard the gain map, but leave `transfer_characteristics =
    ///   PQ/HLG` and an MDCV describing a 1000-nit mastering display, a
    ///   conformant decoder will treat your SDR pixels as HDR and tone-map them
    ///   a second time — visibly wrong. When the gain map goes, the HDR
    ///   signaling that described the HDR rendition must go (or be rewritten to
    ///   match the SDR base: `transfer` → sRGB, drop CLLI/MDCV).
    /// - Conversely, **stripping CICP/HDR while keeping a gain map** orphans the
    ///   gain map (the decoder no longer knows the base is HDR-relative), so the
    ///   HDR rendition is lost or misrendered.
    ///
    /// `filtered` **cannot see the gain map** (it isn't in `Metadata`), so it
    /// cannot enforce this — the consistency is the **caller's responsibility**
    /// at the layer that owns the gain map. Practical guidance:
    ///
    /// - Keeping the gain map untouched → keep CICP/HDR (`Web` / `Preserve`).
    /// - Flattening to SDR and dropping the gain map → drop HDR here (a
    ///   [`Custom`](MetadataPolicy::Custom) policy with `hdr: Discard`, and set
    ///   the encoder's CICP to the SDR transfer) so the signaling matches the
    ///   pixels you actually wrote.
    ///
    /// `cicp` and `hdr` are deliberately *separate* retention flags so this
    /// SDR-flatten case is expressible (drop HDR light-level/mastering while
    /// keeping CICP primaries).
    #[must_use]
    pub fn filtered(&self, policy: &MetadataPolicy) -> Metadata {
        let f = policy.fields();
        let mut out = Metadata::none();

        // ICC — three-way; only KeepNonSrgb drops a redundant sRGB profile.
        out.icc_profile = match f.icc {
            IccRetention::Drop => None,
            // Target-blind retention keeps the profile; the CICP-conditional
            // drop is resolved against a concrete target in
            // `color::resolve_color_emit`, which `filtered` does not see.
            IccRetention::Keep
            | IccRetention::DropIfCicpRepresentable
            | IccRetention::DropIfCicpSafeSoleCarrier => self.icc_profile.clone(),
            IccRetention::KeepNonSrgb => self
                .icc_profile
                .as_ref()
                .filter(|icc| !zenpixels::icc::is_common_srgb(icc))
                .cloned(),
        };

        // Orientation field (codecs may apply it without re-reading the EXIF).
        out.orientation = if f.exif.orientation.keeps() {
            self.orientation
        } else {
            Orientation::Identity
        };

        // Color signaling.
        if f.cicp.keeps() {
            out.cicp = self.cicp;
        }
        if f.hdr.keeps() {
            out.content_light_level = self.content_light_level;
            out.mastering_display = self.mastering_display;
        }

        // XMP (whole-segment).
        if f.xmp.keeps() {
            out.xmp = self.xmp.clone();
        }

        // EXIF — pruned by category; `Arc` clone when nothing is dropped.
        out.exif = self
            .exif
            .as_ref()
            .and_then(|src| match crate::exif::retain(src, &f.exif)? {
                alloc::borrow::Cow::Borrowed(_) => Some(src.clone()),
                alloc::borrow::Cow::Owned(v) => Some(Arc::from(v)),
            });

        // Reconcile the embedded EXIF orientation tag with the authoritative
        // `out.orientation` field. A decoder that bakes orientation upright sets
        // the field to Identity while the source blob still carries the original
        // tag (e.g. Rotate90); left alone, a consumer that re-applies the EXIF tag
        // would rotate twice. Rewriting the tag to match closes that. Only fires
        // on a mismatch, so the matched/common case keeps the zero-copy `Arc`
        // clone above; absent or tag-less blobs are left untouched.
        let want = out.orientation;
        let reconciled = out
            .exif
            .as_deref()
            .filter(|e| parse_exif_orientation(e) != Some(want))
            .and_then(|e| crate::helpers::set_exif_orientation(e, want));
        if let Some(v) = reconciled {
            out.exif = Some(Arc::from(v));
        }
        // The result is already pruned to `policy`; mark it final so a later
        // `for_embedding` / re-filter is a no-op rather than stripping again.
        out.policy = MetadataPolicy::PreserveExact;
        out
    }
}

/// How to treat the ICC profile when filtering [`Metadata`].
///
/// `#[non_exhaustive]`: ICC handling can gain dispositions (e.g. a future
/// convert-to-sRGB or keep-if-display-referred mode) without a breaking
/// change. Match with a `_` arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum IccRetention {
    /// Always drop the profile.
    Drop,
    /// Keep the profile unless it is a redundant sRGB profile — the common
    /// choice (sRGB is the assumed default, so embedding it is pure weight).
    KeepNonSrgb,
    /// Keep the profile as-is, even a redundant sRGB one (byte-faithful).
    Keep,
    /// Drop the profile when it maps to a CICP expressible as code points
    /// (sRGB / Display-P3 / BT.2020 / BT.2100…) — i.e. CICP fully describes the
    /// color. **Target-aware**: only takes effect in
    /// [`color::resolve_color_emit`](crate::color::resolve_color_emit), where the
    /// target's CICP carrier is known. In the target-blind [`Metadata::filtered`]
    /// path it conservatively keeps the profile.
    DropIfCicpRepresentable,
    /// Drop the profile only when the target format's CICP is safe as the sole
    /// color carrier ([`EncodeCapabilities::cicp_safe_sole_carrier`](crate::encode::EncodeCapabilities::cicp_safe_sole_carrier)
    /// — JXL today) and CICP represents the color. Like
    /// [`DropIfCicpRepresentable`](Self::DropIfCicpRepresentable), this is
    /// target-aware and keeps the profile in [`Metadata::filtered`].
    DropIfCicpSafeSoleCarrier,
}

/// Per-field metadata retention for [`MetadataPolicy::Custom`].
///
/// EXIF is encapsulated in [`ExifPolicy`] (pruned by category); the remaining
/// fields use [`Retention`] (explicit `Keep`/`Discard`). This type is
/// `#[non_exhaustive]` (new fields can be added without a breaking change), so
/// downstream crates build from [`KEEP_ALL`](Self::KEEP_ALL) /
/// [`DISCARD_ALL`](Self::DISCARD_ALL) via the `with_*` builders rather than
/// struct-update syntax. Drop only GPS, keep all else:
///
/// ```
/// use zencodec::{MetadataFields, exif::{ExifPolicy, Retention}};
/// let fields = MetadataFields::KEEP_ALL
///     .with_exif(ExifPolicy::KEEP_ALL.with_gps(Retention::Discard));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct MetadataFields {
    /// ICC color profile.
    pub icc: IccRetention,
    /// EXIF, pruned by category.
    pub exif: ExifPolicy,
    /// XMP, whole-segment. The XMP packet (RDF/XML) can carry GPS
    /// (`exif:GPS*`), edit history (`photoshop:History`, `xmpMM:History`),
    /// and C2PA provenance (`xmpMM` manifests), so the presets that strip
    /// privacy/bloat ([`Web`](MetadataPolicy::Web)) discard it wholesale while
    /// keeping EXIF rights.
    ///
    /// Partial XMP (e.g. keep `dc:rights`/`dc:creator`, drop GPS + history +
    /// C2PA) is a planned future addition — it needs an RDF/XML parser, so it
    /// is deferred rather than half-done. It will arrive as a *new*
    /// `MetadataFields` field (this struct is `#[non_exhaustive]`, so adding
    /// one is non-breaking); `xmp` will remain the whole-segment master switch.
    pub xmp: Retention,
    /// CICP color signaling.
    pub cicp: Retention,
    /// HDR `ContentLightLevel` + `MasteringDisplay`.
    pub hdr: Retention,
}

impl MetadataFields {
    /// Keep every field (ICC kept as-is, including a redundant sRGB).
    pub const KEEP_ALL: Self = Self {
        icc: IccRetention::Keep,
        exif: ExifPolicy::KEEP_ALL,
        xmp: Retention::Keep,
        cicp: Retention::Keep,
        hdr: Retention::Keep,
    };
    /// Discard every field.
    pub const DISCARD_ALL: Self = Self {
        icc: IccRetention::Drop,
        exif: ExifPolicy::DISCARD_ALL,
        xmp: Retention::Discard,
        cicp: Retention::Discard,
        hdr: Retention::Discard,
    };

    /// Set ICC retention. (Builder — this type is `#[non_exhaustive]`.)
    #[must_use]
    pub const fn with_icc(mut self, r: IccRetention) -> Self {
        self.icc = r;
        self
    }
    /// Set the EXIF retention policy.
    #[must_use]
    pub const fn with_exif(mut self, p: ExifPolicy) -> Self {
        self.exif = p;
        self
    }
    /// Set XMP retention.
    #[must_use]
    pub const fn with_xmp(mut self, r: Retention) -> Self {
        self.xmp = r;
        self
    }
    /// Set CICP retention.
    #[must_use]
    pub const fn with_cicp(mut self, r: Retention) -> Self {
        self.cicp = r;
        self
    }
    /// Set HDR (light-level/mastering) retention.
    #[must_use]
    pub const fn with_hdr(mut self, r: Retention) -> Self {
        self.hdr = r;
        self
    }
}

/// Field-level metadata retention policy applied by [`Metadata::filtered`].
///
/// `Copy` (all variants, including `Custom(MetadataFields)`, are `Copy`) so it
/// can be bundled by value into [`EncodePolicy`](crate::encode::EncodePolicy).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum MetadataPolicy {
    /// Keep everything the source carried, byte-faithfully — including a
    /// redundant sRGB ICC profile.
    PreserveExact,
    /// Keep everything, but drop a redundant sRGB ICC profile.
    Preserve,
    /// **Default.** The web-publish set: keep the ICC profile (unless a
    /// redundant sRGB), EXIF orientation + rights (copyright/artist), and
    /// CICP / HDR color signaling. Drop the rest of EXIF (GPS, timestamps,
    /// camera/device identity, thumbnail) and all XMP.
    #[default]
    Web,
    /// Keep only what places pixels on screen: the ICC profile (unless a
    /// redundant sRGB), CICP / HDR color signaling, and EXIF orientation.
    /// Drops attribution, XMP, and all other EXIF.
    ColorAndRotation,
    /// Explicit per-field control via [`MetadataFields`].
    Custom(MetadataFields),
}

impl MetadataPolicy {
    /// Resolve the policy to its concrete per-field retention set.
    #[must_use]
    pub fn fields(&self) -> MetadataFields {
        match self {
            Self::PreserveExact => MetadataFields::KEEP_ALL,
            Self::Preserve => MetadataFields {
                icc: IccRetention::KeepNonSrgb,
                ..MetadataFields::KEEP_ALL
            },
            Self::Web => MetadataFields {
                icc: IccRetention::KeepNonSrgb,
                exif: ExifPolicy::ATTRIBUTED_ORIENTATION,
                xmp: Retention::Discard,
                cicp: Retention::Keep,
                hdr: Retention::Keep,
            },
            Self::ColorAndRotation => MetadataFields {
                icc: IccRetention::KeepNonSrgb,
                exif: ExifPolicy::ORIENTATION_ONLY,
                xmp: Retention::Discard,
                cicp: Retention::Keep,
                hdr: Retention::Keep,
            },
            Self::Custom(f) => *f,
        }
    }
}

impl From<&crate::ImageInfo> for Metadata {
    fn from(info: &crate::ImageInfo) -> Self {
        Self {
            icc_profile: info.source_color.icc_profile.clone(),
            exif: info.embedded_metadata.exif.clone(),
            xmp: info.embedded_metadata.xmp.clone(),
            cicp: info.source_color.cicp,
            content_light_level: info.source_color.content_light_level,
            mastering_display: info.source_color.mastering_display,
            orientation: info.orientation,
            // Decoded metadata defaults to the privacy-safe Web policy: the raw
            // bytes are carried for inspection, but `for_embedding` will strip
            // GPS/camera/timestamps/thumbnail/XMP unless the caller overrides
            // via `with_policy` (e.g. `PreserveExact` for a verbatim transcode).
            policy: MetadataPolicy::default(),
        }
    }
}

/// Parse the EXIF Orientation tag (0x0112) from a TIFF/EXIF blob.
///
/// Delegates to the canonical implementation in
/// [`helpers::parse_exif_orientation`](crate::helpers::parse_exif_orientation),
/// which performs full bounds-checking, supports both `SHORT` and `LONG`
/// TIFF types, validates the TIFF magic, and caps IFD entry count to
/// prevent DoS from malformed data.
///
/// Handles both little-endian (`II*\0`) and big-endian (`MM\0*`) byte
/// orders. Returns `None` if the blob is malformed or no Orientation
/// tag exists.
fn parse_exif_orientation(blob: &[u8]) -> Option<Orientation> {
    crate::helpers::parse_exif_orientation(blob)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ImageFormat;

    #[test]
    fn metadata_roundtrip() {
        let info = crate::ImageInfo::new(100, 200, ImageFormat::Jpeg)
            .with_icc_profile(alloc::vec![1, 2, 3])
            .with_exif(alloc::vec![4, 5])
            .with_cicp(Cicp::SRGB)
            .with_content_light_level(ContentLightLevel {
                max_content_light_level: 1000,
                max_frame_average_light_level: 400,
            });
        let meta = info.metadata();
        assert_eq!(meta.icc_profile.as_deref(), Some([1, 2, 3].as_slice()));
        assert_eq!(meta.exif.as_deref(), Some([4, 5].as_slice()));
        assert!(meta.xmp.is_none());
        assert_eq!(meta.cicp, Some(Cicp::SRGB));
        assert_eq!(
            meta.content_light_level.unwrap().max_content_light_level,
            1000
        );
        assert!(meta.mastering_display.is_none());
        assert!(!meta.is_empty());
    }

    #[test]
    fn metadata_empty() {
        let meta = Metadata::none();
        assert!(meta.is_empty());
    }

    #[test]
    fn metadata_with_cicp_not_empty() {
        let meta = Metadata::none().with_cicp(Cicp::SRGB);
        assert!(!meta.is_empty());
    }

    #[test]
    fn metadata_with_hdr_not_empty() {
        let meta = Metadata::none().with_content_light_level(ContentLightLevel {
            max_content_light_level: 1000,
            max_frame_average_light_level: 400,
        });
        assert!(!meta.is_empty());
    }

    #[test]
    fn metadata_orientation_roundtrip() {
        let info = crate::ImageInfo::new(100, 200, ImageFormat::Jpeg)
            .with_orientation(Orientation::Rotate90);
        let meta = info.metadata();
        assert_eq!(meta.orientation, Orientation::Rotate90);
    }

    #[test]
    fn metadata_orientation_default_is_normal() {
        let meta = Metadata::none();
        assert_eq!(meta.orientation, Orientation::Identity);
    }

    #[test]
    fn metadata_with_orientation_builder() {
        let meta = Metadata::none().with_orientation(Orientation::Rotate270);
        assert_eq!(meta.orientation, Orientation::Rotate270);
    }

    #[test]
    fn metadata_orientation_not_empty() {
        let meta = Metadata::none().with_orientation(Orientation::Rotate90);
        assert!(!meta.is_empty());
    }

    #[test]
    fn metadata_identity_orientation_is_empty() {
        let meta = Metadata::none().with_orientation(Orientation::Identity);
        assert!(meta.is_empty());
    }

    #[test]
    fn metadata_transfer_function() {
        let meta = Metadata::none().with_cicp(Cicp::SRGB);
        assert_eq!(meta.transfer_function(), TransferFunction::Srgb);

        let meta = Metadata::none();
        assert_eq!(meta.transfer_function(), TransferFunction::Unknown);
    }

    #[test]
    fn metadata_builder() {
        let meta = Metadata::none()
            .with_icc(alloc::vec![1, 2, 3])
            .with_exif(alloc::vec![4, 5])
            .with_cicp(Cicp::SRGB)
            .with_orientation(Orientation::Rotate90);
        assert!(!meta.is_empty());
        assert_eq!(meta.icc_profile.as_deref(), Some([1, 2, 3].as_slice()));
        assert_eq!(meta.exif.as_deref(), Some([4, 5].as_slice()));
        assert!(meta.xmp.is_none());
        assert_eq!(meta.cicp, Some(Cicp::SRGB));
        assert_eq!(meta.orientation, Orientation::Rotate90);
    }

    #[test]
    fn metadata_from_image_info() {
        let info = crate::ImageInfo::new(100, 200, ImageFormat::Jpeg)
            .with_icc_profile(alloc::vec![10, 20, 30])
            .with_exif(alloc::vec![4, 5])
            .with_cicp(Cicp::SRGB)
            .with_orientation(Orientation::Rotate270);
        let meta = Metadata::from(&info);
        assert_eq!(meta.icc_profile.as_deref(), Some([10, 20, 30].as_slice()));
        assert_eq!(meta.exif.as_deref(), Some([4, 5].as_slice()));
        assert_eq!(meta.cicp, Some(Cicp::SRGB));
        assert_eq!(meta.orientation, Orientation::Rotate270);
    }

    fn build_minimal_exif_with_orientation(value: u16, big_endian: bool) -> alloc::vec::Vec<u8> {
        let mut v = alloc::vec::Vec::new();
        if big_endian {
            v.extend_from_slice(b"MM\x00\x2a");
            v.extend_from_slice(&8u32.to_be_bytes());
            v.extend_from_slice(&1u16.to_be_bytes());
            v.extend_from_slice(&0x0112u16.to_be_bytes());
            v.extend_from_slice(&3u16.to_be_bytes());
            v.extend_from_slice(&1u32.to_be_bytes());
            // SHORT value is padded right within 4-byte value field; for BE
            // the value sits in the FIRST 2 bytes.
            v.extend_from_slice(&value.to_be_bytes());
            v.extend_from_slice(&[0u8, 0]);
            v.extend_from_slice(&0u32.to_be_bytes());
        } else {
            v.extend_from_slice(b"II\x2a\x00");
            v.extend_from_slice(&8u32.to_le_bytes());
            v.extend_from_slice(&1u16.to_le_bytes());
            v.extend_from_slice(&0x0112u16.to_le_bytes());
            v.extend_from_slice(&3u16.to_le_bytes());
            v.extend_from_slice(&1u32.to_le_bytes());
            v.extend_from_slice(&(value as u32).to_le_bytes());
            v.extend_from_slice(&0u32.to_le_bytes());
        }
        v
    }

    #[test]
    fn parse_exif_orientation_le_returns_correct_variant() {
        let blob = build_minimal_exif_with_orientation(6, false);
        assert_eq!(parse_exif_orientation(&blob), Some(Orientation::Rotate90));
    }

    #[test]
    fn parse_exif_orientation_be_returns_correct_variant() {
        let blob = build_minimal_exif_with_orientation(6, true);
        assert_eq!(parse_exif_orientation(&blob), Some(Orientation::Rotate90));
    }

    #[test]
    fn parse_exif_orientation_garbage_returns_none() {
        assert_eq!(parse_exif_orientation(b"garbage"), None);
        assert_eq!(parse_exif_orientation(&[]), None);
        assert_eq!(parse_exif_orientation(&[0u8; 7]), None);
    }

    #[test]
    fn with_exif_auto_parses_orientation_from_blob() {
        let blob = build_minimal_exif_with_orientation(8, false);
        let meta = Metadata::none().with_exif(blob);
        assert_eq!(meta.orientation, Orientation::Rotate270);
    }

    /// Build TIFF with the orientation tag stored as TIFF_LONG (type 4)
    /// instead of SHORT (type 3). The previous loose parser in this file
    /// only read u16 at +8 regardless of type, so for big-endian LONG it
    /// would read the high zero bytes and miss the value. The delegated
    /// helper handles both types correctly.
    fn build_exif_with_long_orientation(value: u32, big_endian: bool) -> alloc::vec::Vec<u8> {
        let mut v = alloc::vec::Vec::new();
        if big_endian {
            v.extend_from_slice(b"MM\x00\x2a");
            v.extend_from_slice(&8u32.to_be_bytes());
            v.extend_from_slice(&1u16.to_be_bytes());
            v.extend_from_slice(&0x0112u16.to_be_bytes());
            v.extend_from_slice(&4u16.to_be_bytes()); // type = LONG
            v.extend_from_slice(&1u32.to_be_bytes());
            v.extend_from_slice(&value.to_be_bytes());
        } else {
            v.extend_from_slice(b"II\x2a\x00");
            v.extend_from_slice(&8u32.to_le_bytes());
            v.extend_from_slice(&1u16.to_le_bytes());
            v.extend_from_slice(&0x0112u16.to_le_bytes());
            v.extend_from_slice(&4u16.to_le_bytes()); // type = LONG
            v.extend_from_slice(&1u32.to_le_bytes());
            v.extend_from_slice(&value.to_le_bytes());
        }
        v
    }

    #[test]
    fn parse_exif_orientation_accepts_long_type_be() {
        let blob = build_exif_with_long_orientation(6, true);
        assert_eq!(parse_exif_orientation(&blob), Some(Orientation::Rotate90));
    }

    #[test]
    fn parse_exif_orientation_accepts_long_type_le() {
        let blob = build_exif_with_long_orientation(8, false);
        assert_eq!(parse_exif_orientation(&blob), Some(Orientation::Rotate270));
    }

    #[test]
    fn with_exif_does_not_override_explicit_orientation() {
        let blob = build_minimal_exif_with_orientation(6, false);
        let meta = Metadata::none()
            .with_orientation(Orientation::FlipH)
            .with_exif(blob);
        // Explicit FlipH must win over the EXIF blob's Rotate90.
        assert_eq!(meta.orientation, Orientation::FlipH);
    }

    // ── MetadataPolicy / filtered ──────────────────────────────────────────

    use crate::exif::Exif;

    /// LE TIFF source: Make (0x010F, camera) + Orientation (0x0112) + Copyright
    /// (0x8298, out-of-line), tag-sorted. `prefix` adds `Exif\0\0` framing.
    fn src_exif(orientation: u16, copyright: &str, prefix: bool) -> alloc::vec::Vec<u8> {
        use alloc::vec::Vec;
        let mut cw = copyright.as_bytes().to_vec();
        cw.push(0); // > 4 bytes → out-of-line
        let n: u16 = 3;
        let ifd_size = 2 + 12 * n as usize + 4;
        let ext_off = 8 + ifd_size;

        let mut t = Vec::new();
        t.extend_from_slice(b"II");
        t.extend_from_slice(&42u16.to_le_bytes());
        t.extend_from_slice(&8u32.to_le_bytes());
        t.extend_from_slice(&n.to_le_bytes());
        // Make 0x010F ASCII "Cam\0" (4 bytes, inline) — camera category.
        t.extend_from_slice(&0x010Fu16.to_le_bytes());
        t.extend_from_slice(&2u16.to_le_bytes());
        t.extend_from_slice(&4u32.to_le_bytes());
        t.extend_from_slice(b"Cam\0");
        // Orientation 0x0112 SHORT (inline).
        t.extend_from_slice(&0x0112u16.to_le_bytes());
        t.extend_from_slice(&3u16.to_le_bytes());
        t.extend_from_slice(&1u32.to_le_bytes());
        t.extend_from_slice(&u32::from(orientation).to_le_bytes());
        // Copyright 0x8298 ASCII (out-of-line).
        t.extend_from_slice(&0x8298u16.to_le_bytes());
        t.extend_from_slice(&2u16.to_le_bytes());
        t.extend_from_slice(&(cw.len() as u32).to_le_bytes());
        t.extend_from_slice(&(ext_off as u32).to_le_bytes());
        t.extend_from_slice(&0u32.to_le_bytes()); // next-IFD offset
        t.extend_from_slice(&cw);

        if prefix {
            let mut out = Vec::with_capacity(6 + t.len());
            out.extend_from_slice(b"Exif\0\0");
            out.extend_from_slice(&t);
            out
        } else {
            t
        }
    }

    /// True if the (little-endian) tag appears in the blob's entry stream.
    fn has_tag(blob: &[u8], tag: u16) -> bool {
        blob.windows(2).any(|w| w == tag.to_le_bytes())
    }

    #[test]
    fn policy_default_is_web() {
        assert_eq!(MetadataPolicy::default(), MetadataPolicy::Web);
    }

    #[test]
    fn policy_fields_resolution() {
        assert_eq!(
            MetadataPolicy::PreserveExact.fields(),
            MetadataFields::KEEP_ALL
        );
        assert_eq!(
            MetadataPolicy::PreserveExact.fields().icc,
            IccRetention::Keep
        );
        assert_eq!(
            MetadataPolicy::Preserve.fields().icc,
            IccRetention::KeepNonSrgb
        );
        assert_eq!(MetadataPolicy::Preserve.fields().exif, ExifPolicy::KEEP_ALL);

        let web = MetadataPolicy::Web.fields();
        assert_eq!(web.icc, IccRetention::KeepNonSrgb);
        assert_eq!(web.exif, ExifPolicy::ATTRIBUTED_ORIENTATION);
        assert_eq!(web.xmp, Retention::Discard);
        assert_eq!(web.cicp, Retention::Keep);
        assert_eq!(web.hdr, Retention::Keep);

        let car = MetadataPolicy::ColorAndRotation.fields();
        assert_eq!(car.exif, ExifPolicy::ORIENTATION_ONLY);
        assert_eq!(car.cicp, Retention::Keep);

        let custom = MetadataFields {
            xmp: Retention::Keep,
            ..MetadataFields::DISCARD_ALL
        };
        assert_eq!(MetadataPolicy::Custom(custom).fields(), custom);
    }

    #[test]
    fn icc_three_way_retention() {
        let icc = alloc::vec![0xABu8; 256]; // arbitrary → not recognized as sRGB
        let meta = Metadata::none().with_icc(icc.clone());
        // KeepNonSrgb keeps a non-sRGB profile (Web/Preserve).
        assert_eq!(
            meta.filtered(&MetadataPolicy::Web).icc_profile.as_deref(),
            Some(icc.as_slice())
        );
        // Keep keeps it too (PreserveExact).
        assert_eq!(
            meta.filtered(&MetadataPolicy::PreserveExact)
                .icc_profile
                .as_deref(),
            Some(icc.as_slice())
        );
        // Drop removes it.
        let drop = MetadataFields {
            icc: IccRetention::Drop,
            ..MetadataFields::KEEP_ALL
        };
        assert!(
            meta.filtered(&MetadataPolicy::Custom(drop))
                .icc_profile
                .is_none()
        );
    }

    #[test]
    fn web_keeps_orientation_rights_drops_camera_and_xmp() {
        let src = src_exif(6, "(c) 2026 Lilith", false);
        let meta = Metadata::none()
            .with_exif(src.clone())
            .with_xmp(alloc::vec![1, 2, 3])
            .with_cicp(Cicp::SRGB)
            .with_content_light_level(ContentLightLevel {
                max_content_light_level: 1000,
                max_frame_average_light_level: 400,
            });
        assert_eq!(meta.orientation, Orientation::Rotate90);

        let out = meta.filtered(&MetadataPolicy::Web);
        let e = out.exif.as_deref().expect("rewritten EXIF");
        let ex = Exif::parse(e).expect("parses");
        assert_eq!(ex.orientation(), Some(Orientation::Rotate90));
        assert_eq!(ex.copyright().unwrap(), "(c) 2026 Lilith");
        // Camera (Make 0x010F) dropped; output is smaller than the source.
        assert!(!has_tag(e, 0x010F));
        assert!(e.len() < src.len());
        assert_eq!(out.orientation, Orientation::Rotate90);
        assert!(out.xmp.is_none());
        assert_eq!(out.cicp, Some(Cicp::SRGB));
        assert!(out.content_light_level.is_some());
    }

    #[test]
    fn preserve_exact_passes_exif_through_byte_identical() {
        let src = src_exif(6, "(c) Owner", false);
        let meta = Metadata::none()
            .with_exif(src.clone())
            .with_xmp(alloc::vec![9, 9])
            .with_icc(alloc::vec![0xABu8; 200]);
        let out = meta.filtered(&MetadataPolicy::PreserveExact);
        assert_eq!(out.exif.as_deref(), Some(src.as_slice()));
        assert!(has_tag(out.exif.as_deref().unwrap(), 0x010F)); // camera kept
        assert_eq!(out.xmp.as_deref(), Some([9, 9].as_slice()));
        assert!(out.icc_profile.is_some());
    }

    #[test]
    fn color_and_rotation_keeps_orientation_drops_rights() {
        let src = src_exif(8, "(c) Owner", false);
        let meta = Metadata::none().with_exif(src).with_cicp(Cicp::SRGB);
        let out = meta.filtered(&MetadataPolicy::ColorAndRotation);
        let e = out.exif.as_deref().expect("EXIF");
        let ex = Exif::parse(e).expect("parses");
        assert_eq!(ex.orientation(), Some(Orientation::Rotate270));
        assert!(ex.copyright().is_none()); // rights dropped
        assert!(!has_tag(e, 0x010F)); // camera dropped
        assert_eq!(out.cicp, Some(Cicp::SRGB));
    }

    // ── Embed-time policy carried on Metadata (for_embedding) ────────────────

    #[test]
    fn default_policy_is_web() {
        // Privacy-safe default: a freshly built Metadata wants Web filtering.
        assert_eq!(Metadata::none().policy, MetadataPolicy::Web);
        assert_eq!(Metadata::default().policy, MetadataPolicy::Web);
    }

    #[test]
    fn with_policy_sets_policy() {
        let m = Metadata::none().with_policy(MetadataPolicy::PreserveExact);
        assert_eq!(m.policy, MetadataPolicy::PreserveExact);
    }

    #[test]
    fn for_embedding_applies_carried_web_policy_by_default() {
        // The codec-facing hook: with the default Web policy, for_embedding
        // strips camera identity while keeping orientation + rights.
        let src = src_exif(6, "(c) Me", false);
        let meta = Metadata::none().with_exif(src);
        assert_eq!(meta.policy, MetadataPolicy::Web);
        let embed = meta.for_embedding();
        let e = embed.exif.as_deref().expect("EXIF");
        let ex = Exif::parse(e).expect("parses");
        assert_eq!(ex.orientation(), Some(Orientation::Rotate90));
        assert_eq!(ex.copyright().unwrap(), "(c) Me");
        assert!(
            !has_tag(e, 0x010F),
            "camera (Make) must be stripped by default"
        );
    }

    #[test]
    fn for_embedding_preserve_exact_is_verbatim() {
        let src = src_exif(6, "(c) Me", false);
        let meta = Metadata::none()
            .with_exif(src.clone())
            .with_policy(MetadataPolicy::PreserveExact);
        let embed = meta.for_embedding();
        assert_eq!(embed.exif.as_deref(), Some(src.as_slice()), "verbatim");
        assert!(
            has_tag(embed.exif.as_deref().unwrap(), 0x010F),
            "camera kept"
        );
    }

    #[test]
    fn for_embedding_output_is_marked_final_no_double_strip() {
        let src = src_exif(6, "(c) Me", false);
        let once = Metadata::none().with_exif(src).for_embedding(); // Web-filtered
        assert_eq!(once.policy, MetadataPolicy::PreserveExact);
        // Re-embedding the already-filtered metadata is a no-op, not a re-strip.
        let twice = once.for_embedding();
        assert_eq!(twice.exif, once.exif);
        let ex = Exif::parse(twice.exif.as_deref().unwrap()).unwrap();
        assert_eq!(ex.copyright().unwrap(), "(c) Me");
    }

    #[test]
    fn filtered_reconciles_baked_orientation_tag() {
        // Simulate a decoder that baked orientation upright: the structured field
        // is Identity, but the source EXIF blob still carries Rotate90 (6).
        let blob = src_exif(6, "(c) Owner", false);
        let meta = Metadata::none()
            .with_exif(blob) // parses 6 → field = Rotate90
            .with_orientation(Orientation::Identity); // baked: field reset to Identity
        assert_eq!(meta.orientation, Orientation::Identity);
        // The unfiltered blob still says Rotate90 — the divergence.
        assert_eq!(
            parse_exif_orientation(meta.exif.as_deref().unwrap()),
            Some(Orientation::Rotate90)
        );

        // filtered() rewrites the embedded tag to match the authoritative field,
        // so the emitted metadata is self-consistent (no double-rotation).
        let out = meta.filtered(&MetadataPolicy::PreserveExact);
        assert_eq!(out.orientation, Orientation::Identity);
        assert_eq!(
            parse_exif_orientation(out.exif.as_deref().unwrap()),
            Some(Orientation::Identity),
            "baked-upright blob must be rewritten to Identity, not left at Rotate90"
        );
    }

    #[test]
    fn custom_drop_only_camera_keeps_rest() {
        let src = src_exif(6, "(c) Owner", false);
        let fields = MetadataFields {
            exif: ExifPolicy {
                camera: Retention::Discard,
                ..ExifPolicy::KEEP_ALL
            },
            ..MetadataFields::KEEP_ALL
        };
        let out = Metadata::none()
            .with_exif(src)
            .filtered(&MetadataPolicy::Custom(fields));
        let e = out.exif.as_deref().expect("EXIF");
        let ex = Exif::parse(e).expect("parses");
        assert_eq!(ex.orientation(), Some(Orientation::Rotate90));
        assert_eq!(ex.copyright().unwrap(), "(c) Owner");
        assert!(!has_tag(e, 0x010F)); // only camera dropped
    }

    #[test]
    fn dropping_orientation_resets_field_to_identity() {
        let meta = Metadata::none().with_orientation(Orientation::Rotate90);
        let fields = MetadataFields {
            exif: ExifPolicy {
                orientation: Retention::Discard,
                ..ExifPolicy::KEEP_ALL
            },
            ..MetadataFields::KEEP_ALL
        };
        let out = meta.filtered(&MetadataPolicy::Custom(fields));
        assert_eq!(out.orientation, Orientation::Identity);
    }

    #[test]
    fn exif_prefix_preserved_through_rewrite() {
        let src = src_exif(6, "(c) Owner", true); // Exif\0\0 prefix
        let out = Metadata::none()
            .with_exif(src)
            .filtered(&MetadataPolicy::Web);
        let e = out.exif.as_deref().expect("EXIF");
        assert_eq!(&e[..6], b"Exif\0\0");
        let ex = Exif::parse(e).expect("parses");
        assert_eq!(ex.orientation(), Some(Orientation::Rotate90));
        assert_eq!(ex.copyright().unwrap(), "(c) Owner");
    }

    #[test]
    fn filtered_empty_metadata_is_empty() {
        for p in [
            MetadataPolicy::PreserveExact,
            MetadataPolicy::Preserve,
            MetadataPolicy::Web,
            MetadataPolicy::ColorAndRotation,
        ] {
            assert!(Metadata::none().filtered(&p).is_empty());
        }
    }
}
