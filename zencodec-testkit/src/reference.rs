//! A faithful in-memory reference codec.
//!
//! It round-trips both pixels *and* metadata (ICC / EXIF / XMP / CICP /
//! orientation), so the conformance checks in this crate can be validated
//! against a known-correct implementation. It also doubles as a worked example
//! of implementing the `zencodec` traits end to end.
//!
//! Wire format (little-endian), one metadata header shared across frames:
//!
//! ```text
//! "ZCR1"            magic (4 bytes)
//! width   : u32
//! height  : u32
//! frames  : u32     frame count (>= 1)
//! bpp     : u8      3 = RGB8, 4 = RGBA8, 6 = RGB16, 8 = RGBA16
//! orient  : u8      EXIF orientation 1..=8
//! flags   : u8      bit0 icc, bit1 exif, bit2 xmp, bit3 cicp, bit4 clli, bit5 mdcv, bit6 gain map
//! [icc ]  : u32 len + bytes
//! [exif]  : u32 len + bytes
//! [xmp ]  : u32 len + bytes
//! [cicp]  : cp:u8 tc:u8 mc:u8 range:u8
//! [clli]  : max_cll:u16 max_fall:u16
//! [mdcv]  : 10 × f32 (rx ry gx gy bx by wx wy max_lum min_lum)
//! [gmap]  : w:u32 h:u32 channels:u8 (1 or 3) alt_cicp:u8 [cp tc mc range]
//!           iso_len:u32 + ISO 21496-1 bytes (bare `JxlJhgm` payload)
//!           pixels (w*h*channels, 8-bit)
//! per frame: duration:u32 + pixels (width*height*bpp)
//! ```
//!
//! The decoded descriptor is stamped from the CICP transfer/primaries when
//! present (a PQ file decodes to a PQ-labelled buffer, never an sRGB-labelled
//! one) — the "descriptor describes the current pixels" rule.
//!
//! The gain map is the worked example of the encode-side contract
//! ([`EncodeJob::with_gain_map_pixels`] / [`with_gain_map_encoded`](EncodeJob::with_gain_map_encoded)):
//! the job validates and stores it, the wire carries the pixels and the ISO
//! 21496-1 metadata, and the decoder surfaces it as a [`DecodedGainMap`] only
//! when asked ([`GainMapRender::Components`], or `ReconstructHdr` — which the
//! reference cannot honour, so it falls back to surfacing the components, as
//! the `reconstructs_hdr = false` contract says).

use std::borrow::Cow;
use std::sync::Arc;

use enough::StopReason;
use whereat::At;
use zencodec::decode::{
    AnimationFrameDecoder, BatchMeta, Decode, DecodeCapabilities, DecodeJob, DecodeOutput,
    DecodeRowSink, DecoderConfig, Event, IncrementalDecode, Input, InputRequest, MetadataUpdate,
    OutputInfo, Progress, SinkError, SourceColor, StepError, StreamingDecode,
};
use zencodec::encode::{
    AnimationFrameEncoder, AnimationOutputTiming, EncodeCapabilities, EncodeJob, EncodeOutput,
    EncodePolicy, Encoder, EncoderConfig, OutputBuffer, OutputTiming,
};
use zencodec::gainmap::{
    DecodedGainMap, GainMapInfo, GainMapPresence, GainMapRender, GainMapSource, Iso21496Format,
    parse_iso21496_fmt, serialize_iso21496_fmt,
};
use zencodec::source::{AccessPattern, DynReadAt};
use zencodec::{
    AnimationFrame, CategorizedError, Cicp, CodecError, CodecIoKind, ContentLightLevel,
    ErrorCategory, ImageFormat, ImageFormatDefinition, ImageInfo, ImageSequence, MasteringDisplay,
    Metadata, Orientation, ResourceLimits, SharedPulse, UnsupportedOperation,
};
use zenpixels::{
    ColorContext, ColorModel, PixelBuffer, PixelDescriptor, PixelSlice, TransferFunction,
};

// ===========================================================================
// Error
// ===========================================================================

/// Error type for the reference codec.
#[derive(Debug)]
pub enum RefError {
    /// An operation the reference codec does not implement.
    Unsupported(UnsupportedOperation),
    /// Malformed wire data.
    Invalid(String),
    /// Cooperative cancellation fired. The [`CategorizedError`] impl maps it
    /// (and a timeout) to the right [`ErrorCategory`] without naming `RefError`.
    Cancelled(StopReason),
    /// A resource limit was exceeded.
    Limit(zencodec::LimitExceeded),
    /// A decode sink reported an error.
    Sink(SinkError),
}

impl std::fmt::Display for RefError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsupported(e) => write!(f, "reference: unsupported: {e}"),
            Self::Invalid(s) => write!(f, "reference: invalid: {s}"),
            Self::Cancelled(r) => write!(f, "reference: cancelled: {r}"),
            Self::Limit(e) => write!(f, "reference: limit: {e}"),
            Self::Sink(e) => write!(f, "reference: sink: {e}"),
        }
    }
}

impl std::error::Error for RefError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Unsupported(e) => Some(e),
            Self::Limit(e) => Some(e),
            Self::Sink(e) => Some(e.as_ref()),
            // StopReason is not an Error, so cancellation isn't a source link.
            Self::Cancelled(_) | Self::Invalid(_) => None,
        }
    }
}

impl From<UnsupportedOperation> for RefError {
    fn from(e: UnsupportedOperation) -> Self {
        Self::Unsupported(e)
    }
}
impl From<StopReason> for RefError {
    fn from(r: StopReason) -> Self {
        Self::Cancelled(r)
    }
}

/// Opt into the codec-agnostic error taxonomy: map each variant to a category,
/// delegating to the zencodec cause types where they apply.
impl CategorizedError for RefError {
    fn codec_name(&self) -> Option<&'static str> {
        Some(MINIMAL_CODEC_NAME)
    }
    fn category(&self) -> ErrorCategory {
        match self {
            Self::Unsupported(e) => e.category(),
            Self::Invalid(_) => ErrorCategory::Image(zencodec::ImageError::Malformed),
            Self::Cancelled(r) => r.category(),
            Self::Limit(e) => e.category(),
            Self::Sink(_) => ErrorCategory::Io(CodecIoKind::opaque()),
        }
    }
}
impl From<zencodec::LimitExceeded> for RefError {
    fn from(e: zencodec::LimitExceeded) -> Self {
        Self::Limit(e)
    }
}

/// The one-impl bridge a codec adds to return the shared envelope as
/// `At<CodecError>` (the internal `minimal` codec does this; the
/// [`reference`](crate::reference) keeps the simpler `type Error = RefError`).
///
/// `.start_at()` begins the location trace; `CodecError::of` then takes that
/// `At<RefError>` and maps it to `At<CodecError>`, keeping the trace on the
/// outside and reading the category *and* codec name
/// ([`RefError::codec_name`](CategorizedError::codec_name)) from the value. With
/// this in place, `?` on any `Result<_, RefError>` auto-wraps into
/// `At<CodecError>`, so existing fallible internals need no rewrite.
impl From<RefError> for At<CodecError> {
    #[track_caller]
    fn from(e: RefError) -> Self {
        use whereat::ErrorAtExt;
        CodecError::of(e.start_at())
    }
}

/// Codec name the envelope reports (via [`RefError`]'s `codec_name`). The only
/// consumer of the bridge is the internal `minimal` codec (the
/// `reference` returns `RefError` directly), so it names that codec.
pub(crate) const MINIMAL_CODEC_NAME: &str = "zencodec-testkit/minimal";

// ===========================================================================
// Wire format
// ===========================================================================

const MAGIC: &[u8; 4] = b"ZCR1";

pub(crate) fn descriptor_for_bpp(bpp: u8) -> Result<PixelDescriptor, RefError> {
    match bpp {
        3 => Ok(PixelDescriptor::RGB8_SRGB),
        4 => Ok(PixelDescriptor::RGBA8_SRGB),
        6 => Ok(PixelDescriptor::RGB16_SRGB),
        8 => Ok(PixelDescriptor::RGBA16_SRGB),
        b => Err(RefError::Invalid(format!("unsupported bpp {b}"))),
    }
}

/// The output descriptor for a parsed header: the layout from `bpp`, with the
/// transfer function and primaries stamped from the file's CICP when it has
/// one. This is what makes a PQ/HLG file decode to a PQ/HLG-labelled buffer.
pub(crate) fn descriptor_for_header(h: &Header) -> Result<PixelDescriptor, RefError> {
    let base = descriptor_for_bpp(h.bpp)?;
    Ok(match h.meta.cicp {
        Some(c) => {
            let with_transfer = match TransferFunction::from_cicp(c.transfer_characteristics) {
                Some(t) => base.with_transfer(t),
                None => base,
            };
            with_transfer.with_primaries(c.color_primaries_enum())
        }
        None => base,
    })
}

/// A gain map as the wire carries it: the ISO 21496-1 description plus the
/// 8-bit pixel bytes (`width * height * channels`, tightly packed).
#[derive(Clone, Debug)]
pub(crate) struct GainMapWire {
    pub(crate) info: GainMapInfo,
    pub(crate) pixels: Vec<u8>,
}

impl GainMapWire {
    /// The reference stores 8-bit gain maps with 1 (luminance) or 3
    /// (per-channel) samples per pixel. Geometry and channel count come from
    /// the pixels — the authoritative source on the decoded-pixel path — and
    /// are stamped back into the metadata so the two can never disagree.
    fn from_decoded(gm: DecodedGainMap) -> Result<Self, RefError> {
        let ps = gm.pixels.as_slice();
        let channels = gm.channels();
        let bytes_per_sample = ps.descriptor().bytes_per_pixel() / channels.max(1) as usize;
        if !matches!(channels, 1 | 3) || bytes_per_sample != 1 {
            return Err(RefError::Invalid(format!(
                "gain map must be 8-bit with 1 or 3 channels, got {:?}",
                ps.descriptor()
            )));
        }
        let mut pixels =
            Vec::with_capacity(ps.width() as usize * ps.rows() as usize * channels as usize);
        let rb = ps.width() as usize * channels as usize;
        for y in 0..ps.rows() {
            pixels.extend_from_slice(&ps.row(y)[..rb]);
        }
        let mut info = gm.metadata;
        info.width = ps.width();
        info.height = ps.rows();
        info.channels = channels;
        info.bit_depth = 8;
        Ok(Self { info, pixels })
    }

    fn descriptor(&self) -> PixelDescriptor {
        // Gain, not colour: the transfer is deliberately `Unknown`.
        if self.info.channels == 1 {
            PixelDescriptor::GRAY8
        } else {
            PixelDescriptor::RGB8
        }
    }

    fn to_decoded(&self) -> Result<DecodedGainMap, RefError> {
        let buf = PixelBuffer::from_vec(
            self.pixels.clone(),
            self.info.width,
            self.info.height,
            self.descriptor(),
        )
        .map_err(|e| RefError::Invalid(format!("gain map buffer: {e}")))?;
        Ok(DecodedGainMap::new(buf, self.info.clone()))
    }
}

/// Parsed header plus the byte offset where frame data starts.
pub(crate) struct Header {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) frame_count: u32,
    pub(crate) bpp: u8,
    pub(crate) meta: Metadata,
    pub(crate) gain_map: Option<GainMapWire>,
    pub(crate) frames_offset: usize,
}

fn push_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn read_u32(data: &[u8], at: usize) -> Result<u32, RefError> {
    data.get(at..at + 4)
        .map(|s| u32::from_le_bytes(s.try_into().unwrap()))
        .ok_or_else(|| RefError::Invalid("truncated u32".into()))
}

fn push_cicp(out: &mut Vec<u8>, c: &Cicp) {
    out.push(c.color_primaries);
    out.push(c.transfer_characteristics);
    out.push(c.matrix_coefficients);
    out.push(c.full_range as u8);
}

fn write_header(
    out: &mut Vec<u8>,
    w: u32,
    h: u32,
    frames: u32,
    bpp: u8,
    meta: &Metadata,
    gain_map: Option<&GainMapWire>,
) {
    out.extend_from_slice(MAGIC);
    push_u32(out, w);
    push_u32(out, h);
    push_u32(out, frames);
    out.push(bpp);
    out.push(meta.orientation.to_exif());

    let mut flags = 0u8;
    if meta.icc_profile.is_some() {
        flags |= 1;
    }
    if meta.exif.is_some() {
        flags |= 2;
    }
    if meta.xmp.is_some() {
        flags |= 4;
    }
    if meta.cicp.is_some() {
        flags |= 8;
    }
    if meta.content_light_level.is_some() {
        flags |= 16;
    }
    if meta.mastering_display.is_some() {
        flags |= 32;
    }
    if gain_map.is_some() {
        flags |= 64;
    }
    out.push(flags);

    if let Some(icc) = &meta.icc_profile {
        push_u32(out, icc.len() as u32);
        out.extend_from_slice(icc);
    }
    if let Some(exif) = &meta.exif {
        push_u32(out, exif.len() as u32);
        out.extend_from_slice(exif);
    }
    if let Some(xmp) = &meta.xmp {
        push_u32(out, xmp.len() as u32);
        out.extend_from_slice(xmp);
    }
    if let Some(c) = &meta.cicp {
        push_cicp(out, c);
    }
    if let Some(cll) = &meta.content_light_level {
        out.extend_from_slice(&cll.max_content_light_level.to_le_bytes());
        out.extend_from_slice(&cll.max_frame_average_light_level.to_le_bytes());
    }
    if let Some(md) = &meta.mastering_display {
        for xy in md.primaries_xy {
            out.extend_from_slice(&xy[0].to_le_bytes());
            out.extend_from_slice(&xy[1].to_le_bytes());
        }
        out.extend_from_slice(&md.white_point_xy[0].to_le_bytes());
        out.extend_from_slice(&md.white_point_xy[1].to_le_bytes());
        out.extend_from_slice(&md.max_luminance.to_le_bytes());
        out.extend_from_slice(&md.min_luminance.to_le_bytes());
    }
    if let Some(gm) = gain_map {
        push_u32(out, gm.info.width);
        push_u32(out, gm.info.height);
        out.push(gm.info.channels);
        match &gm.info.alternate_cicp {
            Some(c) => {
                out.push(1);
                push_cicp(out, c);
            }
            None => out.push(0),
        }
        let iso = serialize_iso21496_fmt(&gm.info.params, Iso21496Format::JxlJhgm);
        push_u32(out, iso.len() as u32);
        out.extend_from_slice(&iso);
        out.extend_from_slice(&gm.pixels);
    }
}

fn read_cicp(data: &[u8], at: usize) -> Result<Cicp, RefError> {
    let c = data
        .get(at..at + 4)
        .ok_or_else(|| RefError::Invalid("truncated cicp".into()))?;
    Ok(Cicp::new(c[0], c[1], c[2], c[3] != 0))
}

fn read_f32(data: &[u8], at: usize) -> Result<f32, RefError> {
    data.get(at..at + 4)
        .map(|s| f32::from_le_bytes(s.try_into().unwrap()))
        .ok_or_else(|| RefError::Invalid("truncated f32".into()))
}

pub(crate) fn parse_header(data: &[u8]) -> Result<Header, RefError> {
    // Fixed header is 19 bytes: magic(4) + w(4) + h(4) + frames(4) + bpp(1) +
    // orient(1) + flags(1); data[18] (flags) is read below.
    if data.len() < 19 || &data[..4] != MAGIC {
        return Err(RefError::Invalid("bad magic / short header".into()));
    }
    let width = read_u32(data, 4)?;
    let height = read_u32(data, 8)?;
    let frame_count = read_u32(data, 12)?;
    let bpp = data[16];
    let orient = data[17];
    let flags = data[18];
    descriptor_for_bpp(bpp)?; // validate
    if width == 0 || height == 0 || frame_count == 0 {
        return Err(RefError::Invalid("zero dimension / frame count".into()));
    }

    let mut meta = Metadata::none()
        .with_orientation(Orientation::from_exif(orient).unwrap_or(Orientation::Identity));
    let mut at = 19usize;

    let read_blob = |at: &mut usize| -> Result<Vec<u8>, RefError> {
        let len = read_u32(data, *at)? as usize;
        *at += 4;
        let bytes = data
            .get(*at..*at + len)
            .ok_or_else(|| RefError::Invalid("truncated blob".into()))?
            .to_vec();
        *at += len;
        Ok(bytes)
    };

    if flags & 1 != 0 {
        meta = meta.with_icc(read_blob(&mut at)?);
    }
    if flags & 2 != 0 {
        meta = meta.with_exif(read_blob(&mut at)?);
    }
    if flags & 4 != 0 {
        meta = meta.with_xmp(read_blob(&mut at)?);
    }
    if flags & 8 != 0 {
        meta = meta.with_cicp(read_cicp(data, at)?);
        at += 4;
    }
    if flags & 16 != 0 {
        let c = data
            .get(at..at + 4)
            .ok_or_else(|| RefError::Invalid("truncated clli".into()))?;
        meta = meta.with_content_light_level(ContentLightLevel::new(
            u16::from_le_bytes([c[0], c[1]]),
            u16::from_le_bytes([c[2], c[3]]),
        ));
        at += 4;
    }
    if flags & 32 != 0 {
        let mut f = [0f32; 10];
        for (i, v) in f.iter_mut().enumerate() {
            *v = read_f32(data, at + i * 4)?;
        }
        at += 40;
        meta = meta.with_mastering_display(MasteringDisplay::new(
            [[f[0], f[1]], [f[2], f[3]], [f[4], f[5]]],
            [f[6], f[7]],
            f[8],
            f[9],
        ));
    }
    let gain_map = if flags & 64 != 0 {
        let gw = read_u32(data, at)?;
        let gh = read_u32(data, at + 4)?;
        let channels = *data
            .get(at + 8)
            .ok_or_else(|| RefError::Invalid("truncated gain map".into()))?;
        let has_alt = *data
            .get(at + 9)
            .ok_or_else(|| RefError::Invalid("truncated gain map".into()))?;
        at += 10;
        let alternate_cicp = if has_alt != 0 {
            let c = read_cicp(data, at)?;
            at += 4;
            Some(c)
        } else {
            None
        };
        let iso = read_blob(&mut at)?;
        let params = parse_iso21496_fmt(&iso, Iso21496Format::JxlJhgm)
            .map_err(|e| RefError::Invalid(format!("gain map metadata: {e}")))?;
        if !matches!(channels, 1 | 3) || gw == 0 || gh == 0 {
            return Err(RefError::Invalid("bad gain map geometry".into()));
        }
        let len = gw as usize * gh as usize * channels as usize;
        let pixels = data
            .get(at..at + len)
            .ok_or_else(|| RefError::Invalid("truncated gain map pixels".into()))?
            .to_vec();
        at += len;
        let mut info = GainMapInfo::new(params, gw, gh, channels);
        if let Some(c) = alternate_cicp {
            info = info.with_alternate_cicp(c);
        }
        Some(GainMapWire { info, pixels })
    } else {
        None
    };

    Ok(Header {
        width,
        height,
        frame_count,
        bpp,
        meta,
        gain_map,
        frames_offset: at,
    })
}

/// Byte offset of frame `i`'s pixel data (skipping its leading duration u32).
pub(crate) fn frame_pixels_offset(h: &Header, i: u32) -> usize {
    let frame_bytes = 4 + h.width as usize * h.height as usize * h.bpp as usize;
    h.frames_offset + i as usize * frame_bytes + 4
}

pub(crate) fn frame_pixel_len(h: &Header) -> usize {
    h.width as usize * h.height as usize * h.bpp as usize
}

/// Structural inventory of a `ZCR1` stream: the worked example of
/// [`DecodeJob::inventory`].
///
/// Walks the wire format with checked arithmetic and no pixel allocation, and
/// never fails on malformed input: whatever cannot be read becomes one
/// [`Malformed`](zencodec::inventory::Disposition::Malformed) part, and bytes
/// after the last frame are [`Trailing`](zencodec::inventory::Disposition::Trailing)
/// because the decoder never reads them. The only error is the part cap.
pub(crate) fn zcr_inventory(
    data: &[u8],
    format: ImageFormat,
) -> Result<zencodec::inventory::Inventory, RefError> {
    use zencodec::inventory::{
        Disposition, Inventory, InventoryError, MetadataKind, Part, PartKind, PartTag,
    };
    let len = data.len() as u64;
    let mut inv = Inventory::new(format, len);
    let cap = |e: InventoryError| RefError::Invalid(format!("inventory: {e}"));
    let malformed_from = |inv: &mut Inventory, at: u64, why: &str| -> Result<(), RefError> {
        if at < len {
            inv.push(
                None,
                Part::new(
                    PartKind::Gap,
                    PartTag::None,
                    at..len,
                    Disposition::Malformed,
                )
                .with_detail(why.to_owned()),
            )
            .map_err(cap)?;
        }
        Ok(())
    };

    let header_ok = data.len() >= 19
        && &data[..4] == MAGIC
        && descriptor_for_bpp(data[16]).is_ok()
        && read_u32(data, 4).is_ok_and(|w| w > 0)
        && read_u32(data, 8).is_ok_and(|h| h > 0)
        && read_u32(data, 12).is_ok_and(|f| f > 0);
    if !header_ok {
        malformed_from(&mut inv, 0, "not a ZCR1 header")?;
        return Ok(inv);
    }
    let (w, h, frames, bpp, flags) = (
        u64::from(read_u32(data, 4)?),
        u64::from(read_u32(data, 8)?),
        u64::from(read_u32(data, 12)?),
        u64::from(data[16]),
        data[18],
    );
    inv.push(
        None,
        Part::new(
            PartKind::Header,
            PartTag::None,
            0..19,
            Disposition::Structure,
        )
        .with_label("ZCR1")
        .with_detail("magic, geometry, frame count, layout, orientation, flags"),
    )
    .map_err(cap)?;

    let mut at: u64 = 19;
    // A length-prefixed blob: `u32` length, then the bytes.
    let blob_end = |at: u64| -> Option<u64> {
        let n = read_u32(data, usize::try_from(at).ok()?).ok()?;
        let end = at.checked_add(4)?.checked_add(u64::from(n))?;
        (end <= len).then_some(end)
    };
    let fixed_end = |at: u64, n: u64| -> Option<u64> {
        let end = at.checked_add(n)?;
        (end <= len).then_some(end)
    };
    let sections: [(u8, &str, MetadataKind); 6] = [
        (1, "icc", MetadataKind::Icc),
        (2, "exif", MetadataKind::Exif),
        (4, "xmp", MetadataKind::Xmp),
        (8, "cicp", MetadataKind::Cicp),
        (16, "clli", MetadataKind::HdrStatic),
        (32, "mdcv", MetadataKind::HdrStatic),
    ];
    for (bit, name, kind) in sections {
        if flags & bit == 0 {
            continue;
        }
        let end = match bit {
            1 | 2 | 4 => blob_end(at),
            8 | 16 => fixed_end(at, 4),
            _ => fixed_end(at, 40),
        };
        let Some(end) = end else {
            malformed_from(&mut inv, at, &format!("truncated {name} section"))?;
            return Ok(inv);
        };
        inv.push(
            None,
            Part::new(
                PartKind::Chunk,
                PartTag::Name(name.into()),
                at..end,
                Disposition::Metadata(kind),
            ),
        )
        .map_err(cap)?;
        at = end;
    }
    if flags & 64 != 0 {
        // w:u32 h:u32 channels:u8 has_alt:u8 [cicp:4] iso_len:u32 iso pixels
        let gm = (|| -> Option<u64> {
            let at_us = usize::try_from(at).ok()?;
            let gw = u64::from(read_u32(data, at_us).ok()?);
            let gh = u64::from(read_u32(data, at_us.checked_add(4)?).ok()?);
            let channels = u64::from(*data.get(at_us.checked_add(8)?)?);
            let has_alt = *data.get(at_us.checked_add(9)?)? != 0;
            let iso_at = at + 10 + if has_alt { 4 } else { 0 };
            let iso_end = blob_end(iso_at)?;
            fixed_end(iso_end, gw.checked_mul(gh)?.checked_mul(channels)?)
        })();
        let Some(end) = gm else {
            malformed_from(&mut inv, at, "truncated gain-map section")?;
            return Ok(inv);
        };
        inv.push(
            None,
            Part::new(
                PartKind::Chunk,
                PartTag::Name("gain-map".into()),
                at..end,
                Disposition::Metadata(MetadataKind::GainMap),
            ),
        )
        .map_err(cap)?;
        at = end;
    }

    let pixels = w.checked_mul(h).and_then(|n| n.checked_mul(bpp));
    for i in 0..frames {
        let (Some(dur_end), Some(px_end)) = (
            fixed_end(at, 4),
            pixels.and_then(|n| fixed_end(at.saturating_add(4), n)),
        ) else {
            malformed_from(&mut inv, at, &format!("truncated frame {i}"))?;
            return Ok(inv);
        };
        inv.push(
            None,
            Part::new(
                PartKind::Chunk,
                PartTag::Code(i as u32),
                at..dur_end,
                Disposition::Metadata(MetadataKind::Animation),
            )
            .with_label("duration"),
        )
        .map_err(cap)?;
        inv.push(
            None,
            Part::new(
                PartKind::Chunk,
                PartTag::Code(i as u32),
                dur_end..px_end,
                Disposition::ImageData,
            )
            .with_label("pixels"),
        )
        .map_err(cap)?;
        at = px_end;
    }
    inv.fill_gaps(None, Disposition::Trailing).map_err(cap)?;
    Ok(inv)
}

pub(crate) fn build_info(h: &Header) -> ImageInfo {
    let mut info =
        ImageInfo::new(h.width, h.height, ImageFormat::Pnm).with_orientation(h.meta.orientation);
    if let Some(icc) = &h.meta.icc_profile {
        info = info.with_icc_profile(icc.to_vec());
    }
    if let Some(exif) = &h.meta.exif {
        info = info.with_exif(exif.to_vec());
    }
    if let Some(xmp) = &h.meta.xmp {
        info = info.with_xmp(xmp.to_vec());
    }
    if let Some(c) = h.meta.cicp {
        info = info.with_cicp(c);
    }
    if let Some(cll) = h.meta.content_light_level {
        info = info.with_content_light_level(cll);
    }
    if let Some(md) = h.meta.mastering_display {
        info = info.with_mastering_display(md);
    }
    // The header is the whole container, so presence is definitive either way.
    info = match &h.gain_map {
        Some(gm) => {
            let mut supplements = info.supplements.clone();
            supplements.gain_map = true;
            info.with_supplements(supplements)
                .with_gain_map(GainMapPresence::Available(Box::new(gm.info.clone())))
        }
        None => info.with_gain_map(GainMapPresence::Absent),
    };
    if h.frame_count > 1 {
        info = info.with_sequence(ImageSequence::Animation {
            frame_count: Some(h.frame_count),
            loop_count: None,
            random_access: false,
        });
    }
    info
}

/// The buffer-level colour description a decoder attaches to its output
/// pixels — the worked example of the convention in `docs/IMPLEMENTING.md`
/// ("Colour on the decoded buffer"):
///
/// 1. Start from the authoritative field only
///    ([`SourceColor::to_color_context`] — drop-dupe: ICC *or* CICP, never a
///    duplicated pair the CMS would have to arbitrate).
/// 2. **Class-gate the ICC.** A profile rides a buffer only when its device
///    class (header bytes 16..20, read via `zenpixels::icc::profile_color_space`)
///    matches the buffer's colour model — `RGB ` on Rgb/Rgba/Bgr/Bgra, `GRAY` on
///    Gray/GrayAlpha, `CMYK` on Cmyk. Crosswise pairing is invalid signaling.
/// 3. On a mismatch, **derive the profile's CICP** (embedded `cICP` tag, then
///    well-known-profile identification) and carry that alone; fall back to the
///    signaled CICP. A decoder that can choose its output layout should prefer
///    a layout the profile describes over reaching this branch (the reference
///    can't — it only emits RGB/RGBA — so it documents the fallback instead).
/// 4. An empty description is not attached at all — `None` means "nothing
///    known", not "described as nothing".
///
/// Shared by every emission path (one-shot buffer, streaming strips, animation
/// frames) because scratch/strip buffers are where a context silently dies;
/// `check_color_context_consistency` diffs the paths to prove it didn't.
pub(crate) fn class_gated_context(
    source: &SourceColor,
    model: ColorModel,
) -> Option<Arc<ColorContext>> {
    let mut ctx = source.to_color_context();
    if let Some(icc) = ctx
        .icc
        .take_if(|icc| zenpixels::icc::profile_color_space(icc) != Some(model))
    {
        ctx.cicp = zenpixels::icc::extract_cicp(&icc)
            .or_else(|| zenpixels::icc::identify_common(&icc).and_then(|id| id.to_cicp()))
            .or(ctx.cicp)
            .or(source.cicp);
    }
    if ctx.icc.is_none() && ctx.cicp.is_none() {
        return None;
    }
    Some(Arc::new(ctx))
}

pub(crate) fn encode_single(
    pixels: PixelSlice<'_>,
    meta: &Metadata,
    gain_map: Option<&GainMapWire>,
) -> Vec<u8> {
    let bpp = pixels.descriptor().bytes_per_pixel() as u8;
    let mut out = Vec::new();
    write_header(
        &mut out,
        pixels.width(),
        pixels.rows(),
        1,
        bpp,
        meta,
        gain_map,
    );
    push_u32(&mut out, 0); // duration
    for y in 0..pixels.rows() {
        out.extend_from_slice(pixels.row(y));
    }
    out
}

// ===========================================================================
// Capabilities
// ===========================================================================

// The reference is a lossless-only codec. Every capability declared here is
// honored — that's the point: `check_capability_honesty` must pass against it.
// (No `lossy`, `stop`, or quality-range: the reference stores raw pixels, encodes
// instantly so there's nothing to cancel, and has no quality knob. Declaring them
// would be the exact dishonesty the check exists to catch.)
static ENCODE_CAPS: EncodeCapabilities = EncodeCapabilities::new()
    .with_animation_timing(true)
    .with_lossless(true)
    .with_native_alpha(true)
    .with_native_16bit(true)
    .with_hdr(true) // PQ/HLG CICP + CLLI/MDCV round-trip (check_native_hdr_roundtrip)
    .with_gain_map(true) // with_gain_map_pixels / _encoded (check_gain_map_roundtrip)
    .with_animation(true)
    .with_push_rows(true)
    .with_streaming_output(true) // begin writes the header; rows stream out
    .with_encode_from(false) // reference declines the pull path (see encoder)
    .with_icc(true)
    .with_exif(true)
    .with_xmp(true)
    .with_cicp(true);

static DECODE_CAPS: DecodeCapabilities = DecodeCapabilities::new()
    .with_icc(true)
    .with_exif(true)
    .with_xmp(true)
    .with_cicp(true)
    .with_cheap_probe(true)
    .with_animation(true)
    .with_streaming(true)
    .with_native_alpha(true)
    .with_native_16bit(true)
    .with_hdr(true)
    // Extracts / surfaces the gain map (`GainMapRender::Components`) but does
    // NOT reconstruct HDR — zencodec carries no HDR math, so `reconstructs_hdr`
    // stays false and a `ReconstructHdr` request falls back to `Components`.
    .with_gain_map(true)
    .with_inventory(true)
    .with_incremental(true)
    .with_seekable(true);

/// Every layout the reference stores raw: 8- and 16-bit RGB/RGBA, with the
/// 16-bit HDR-labelled variants listed so a PQ/HLG caller negotiates natively.
static DESCRIPTORS: [PixelDescriptor; 6] = [
    PixelDescriptor::RGB8_SRGB,
    PixelDescriptor::RGBA8_SRGB,
    PixelDescriptor::RGB16_SRGB,
    PixelDescriptor::RGBA16_SRGB,
    PixelDescriptor::RGB16_BT2100_PQ,
    PixelDescriptor::RGB16_BT2100_HLG,
];

// ===========================================================================
// Encode: Config -> Job -> Encoder / AnimationFrameEncoder
// ===========================================================================

/// Reference encoder configuration. Accepts RGB8 and RGBA8.
#[derive(Clone, Debug, Default)]
pub struct ReferenceEncoderConfig {}

impl ReferenceEncoderConfig {
    /// Construct a fresh config.
    pub fn new() -> Self {
        Self::default()
    }
}

impl EncoderConfig for ReferenceEncoderConfig {
    type Error = RefError;
    type Job = RefEncodeJob;

    fn format() -> ImageFormat {
        ImageFormat::Pnm
    }
    fn supported_descriptors() -> &'static [PixelDescriptor] {
        &DESCRIPTORS
    }
    fn capabilities() -> &'static EncodeCapabilities {
        &ENCODE_CAPS
    }
    // The reference stores raw pixels, so it is lossless whatever was asked
    // for: `is_lossless()` is `Some(true)` unconditionally, and a `Lossy`
    // fidelity request resolves — honestly — to `Fidelity::Lossless` through
    // the trait's default `resolved_target_fidelity`. This is the exemplar of
    // the "promote and report" rule for a codec with no lossy mode
    // (`check_fidelity_honesty`); a no-op `with_lossless` that left
    // `is_lossless()` at `None` would deny the declared `lossless` capability,
    // and recording `Some(false)` would claim a lossy mode it does not have.
    fn with_lossless(self, _lossless: bool) -> Self {
        self
    }
    fn is_lossless(&self) -> Option<bool> {
        Some(true)
    }
    fn job(self) -> RefEncodeJob {
        RefEncodeJob {
            metadata: Metadata::none(),
            policy: EncodePolicy::none(),
            loop_count: None,
            gain_map: None,
        }
    }
}

/// Reference encode job.
pub struct RefEncodeJob {
    metadata: Metadata,
    policy: EncodePolicy,
    loop_count: Option<u32>,
    gain_map: Option<GainMapWire>,
}

fn apply_embed_policy(mut metadata: Metadata, policy: EncodePolicy) -> Metadata {
    if policy.embed_icc == Some(false) {
        metadata.icc_profile = None;
    }
    if policy.embed_exif == Some(false) {
        metadata.exif = None;
    }
    if policy.embed_xmp == Some(false) {
        metadata.xmp = None;
    }
    metadata
}

impl EncodeJob for RefEncodeJob {
    type Error = RefError;
    type Enc = RefEnc;
    type AnimationFrameEnc = RefAnimEnc;

    fn with_pulse(self, _stop: SharedPulse) -> Self {
        self
    }
    fn with_limits(self, _limits: ResourceLimits) -> Self {
        self
    }
    fn with_policy(mut self, policy: EncodePolicy) -> Self {
        self.policy = policy;
        self
    }
    fn try_set_policy(&mut self, policy: EncodePolicy) -> Result<(), UnsupportedOperation> {
        let mut unsupported = policy;
        unsupported.embed_icc = None;
        unsupported.embed_exif = None;
        unsupported.embed_xmp = None;
        if unsupported != EncodePolicy::none() {
            return Err(UnsupportedOperation::EncodePolicy);
        }
        self.policy = policy;
        Ok(())
    }
    fn with_metadata(mut self, meta: Metadata) -> Self {
        self.metadata = meta;
        self
    }
    fn with_loop_count(mut self, count: Option<u32>) -> Self {
        self.loop_count = count;
        self
    }

    /// The pixel form: validate (8-bit, 1 or 3 channels) and store; a bad gain
    /// map is the codec's own error, never a silent drop.
    fn with_gain_map_pixels(mut self, gain_map: DecodedGainMap) -> Result<Self, RefError> {
        self.gain_map = Some(GainMapWire::from_decoded(gain_map)?);
        Ok(self)
    }

    /// The byte-carried form: only the reference's own format is accepted (a
    /// single-frame RGB8 file of its own wire format — the reference has no
    /// grayscale layout of its own, so a 1-channel map must come as pixels).
    /// Any other `format` is the documented mismatch error: the caller decodes
    /// to pixels and uses `with_gain_map_pixels`.
    fn with_gain_map_encoded(mut self, gain_map: GainMapSource) -> Result<Self, RefError> {
        if gain_map.format != ImageFormat::Pnm {
            return Err(RefError::Invalid(format!(
                "encoded gain map is {:?}, not the reference's own format — decode it and use \
                 with_gain_map_pixels",
                gain_map.format
            )));
        }
        let h = parse_header(&gain_map.data)?;
        if h.frame_count != 1 || h.bpp != 3 {
            return Err(RefError::Invalid(
                "encoded gain map must be a single-frame RGB8 reference image".into(),
            ));
        }
        let start = frame_pixels_offset(&h, 0);
        let len = frame_pixel_len(&h);
        let pixels = gain_map
            .data
            .get(start..start + len)
            .ok_or_else(|| RefError::Invalid("truncated gain map pixels".into()))?
            .to_vec();
        let mut info = gain_map.metadata;
        info.width = h.width;
        info.height = h.height;
        info.channels = 3;
        info.bit_depth = 8;
        self.gain_map = Some(GainMapWire { info, pixels });
        Ok(self)
    }

    fn encoder(self) -> Result<RefEnc, RefError> {
        let metadata = apply_embed_policy(self.metadata, self.policy);
        Ok(RefEnc {
            metadata,
            gain_map: self.gain_map,
            accumulated: Vec::new(),
            out: OutputBuffer::new(),
            width: None,
            rows: 0,
            desc: None,
            declared_height: None,
        })
    }

    fn animation_frame_encoder(self) -> Result<RefAnimEnc, RefError> {
        let metadata = apply_embed_policy(self.metadata, self.policy);
        Ok(RefAnimEnc {
            metadata,
            gain_map: self.gain_map,
            frames: Vec::new(),
            timing: zencodec::animation::TimingQuantizer::new(1000, u32::MAX.into()).unwrap(),
            streaming: None,
        })
    }
}

/// Reference single-image encoder. Supports one-shot `encode` and incremental
/// `push_rows` + `finish`; declines the pull path (`encode_from`).
///
/// After [`begin`](Encoder::begin) the header is written at once and each
/// pushed row is appended to the output, which
/// [`pending_output`](Encoder::pending_output) exposes: the streaming
/// encoder pattern. Without `begin`, rows accumulate and the file is built
/// at `finish`.
pub struct RefEnc {
    metadata: Metadata,
    gain_map: Option<GainMapWire>,
    /// Without `begin`: raw rows.
    accumulated: Vec<u8>,
    /// With `begin`: the encoded file not yet handed out.
    out: OutputBuffer,
    width: Option<u32>,
    rows: u32,
    desc: Option<PixelDescriptor>,
    declared_height: Option<u32>,
}

impl Encoder for RefEnc {
    type Error = RefError;

    fn reject(op: UnsupportedOperation) -> RefError {
        RefError::Unsupported(op)
    }

    fn preferred_strip_height(&self) -> u32 {
        4
    }

    fn encode(self, pixels: PixelSlice<'_>) -> Result<EncodeOutput, RefError> {
        // The usage paths are exclusive: no one-shot encode after `begin` or
        // `push_rows`.
        if self.width.is_some() {
            return Err(RefError::Invalid("encode after begin or push_rows".into()));
        }
        Ok(EncodeOutput::new(
            encode_single(pixels, &self.metadata, self.gain_map.as_ref()),
            ImageFormat::Pnm,
        ))
    }

    fn begin(
        &mut self,
        width: u32,
        height: u32,
        desc: PixelDescriptor,
    ) -> Result<OutputTiming, RefError> {
        if self.declared_height.is_some() {
            return Err(RefError::Invalid("begin called twice".into()));
        }
        if self.width.is_some() {
            return Err(RefError::Invalid("begin after push_rows".into()));
        }
        self.width = Some(width);
        self.desc = Some(desc);
        self.declared_height = Some(height);
        let file = self.out.vec_mut();
        write_header(
            file,
            width,
            height,
            1,
            desc.bytes_per_pixel() as u8,
            &self.metadata,
            self.gain_map.as_ref(),
        );
        push_u32(file, 0); // duration
        // Each row is appended as it arrives.
        Ok(OutputTiming::streaming(0))
    }

    fn push_rows(&mut self, rows: PixelSlice<'_>) -> Result<(), RefError> {
        if self.width.is_none() {
            self.width = Some(rows.width());
            self.desc = Some(rows.descriptor());
        }
        if Some(rows.width()) != self.width || Some(rows.descriptor()) != self.desc {
            return Err(RefError::Invalid(
                "push_rows: width or format changed".into(),
            ));
        }
        if let Some(h) = self.declared_height
            && self.rows.saturating_add(rows.rows()) > h
        {
            return Err(RefError::Invalid(format!(
                "push_rows: more than the {h} rows declared at begin"
            )));
        }
        let dest = match self.declared_height {
            Some(_) => self.out.vec_mut(),
            None => &mut self.accumulated,
        };
        for y in 0..rows.rows() {
            dest.extend_from_slice(rows.row(y));
            self.rows += 1;
        }
        Ok(())
    }

    fn pending_output(&self) -> &[u8] {
        self.out.pending()
    }

    fn consume_output(&mut self, n: usize) {
        self.out.consume(n);
    }

    fn take_output(&mut self, into: &mut Vec<u8>) {
        self.out.take_into(into);
    }

    fn finish(self) -> Result<EncodeOutput, RefError> {
        if let Some(h) = self.declared_height {
            if self.rows != h {
                return Err(RefError::Invalid(format!(
                    "finish: {} rows pushed, {h} declared at begin",
                    self.rows
                )));
            }
            return Ok(EncodeOutput::new(
                self.out.into_remaining(),
                ImageFormat::Pnm,
            ));
        }
        let w = self
            .width
            .ok_or_else(|| RefError::Invalid("finish before push_rows".into()))?;
        let desc = self.desc.unwrap();
        let buf = PixelBuffer::from_vec(self.accumulated, w, self.rows, desc)
            .map_err(|e| RefError::Invalid(format!("buffer: {e}")))?;
        Ok(EncodeOutput::new(
            encode_single(buf.as_slice(), &self.metadata, self.gain_map.as_ref()),
            ImageFormat::Pnm,
        ))
    }
}

/// Reference animation encoder.
///
/// The file starts with the frame count, so like APNG it streams only when
/// [`begin`](AnimationFrameEncoder::begin) declares one: then the header is
/// written at once and each frame as it is pushed. Otherwise frames
/// accumulate and the file is built at `finish`.
pub struct RefAnimEnc {
    timing: zencodec::animation::TimingQuantizer,
    metadata: Metadata,
    gain_map: Option<GainMapWire>,
    frames: Vec<(Vec<u8>, u32, u32, u32, PixelDescriptor)>, // bytes, w, h, dur, desc
    streaming: Option<RefAnimStream>,
}

/// A streaming animation: what `begin` declared, and the file so far.
struct RefAnimStream {
    width: u32,
    height: u32,
    desc: PixelDescriptor,
    frame_count: u32,
    pushed: u32,
    out: OutputBuffer,
}

impl AnimationFrameEncoder for RefAnimEnc {
    type Error = RefError;

    fn reject(op: UnsupportedOperation) -> RefError {
        RefError::Unsupported(op)
    }

    fn push_frame(
        &mut self,
        pixels: PixelSlice<'_>,
        duration_ms: u32,
        stop: Option<&dyn zencodec::Pulse>,
    ) -> Result<(), RefError> {
        if let Some(s) = stop {
            s.check()?;
        }
        let (w, h, desc) = (pixels.width(), pixels.rows(), pixels.descriptor());
        if let Some(st) = &mut self.streaming {
            if (w, h, desc) != (st.width, st.height, st.desc) {
                return Err(RefError::Invalid(
                    "push_frame: frame differs from the canvas declared at begin".into(),
                ));
            }
            if st.pushed == st.frame_count {
                return Err(RefError::Invalid(format!(
                    "push_frame: more than the {} frames declared at begin",
                    st.frame_count
                )));
            }
            let file = st.out.vec_mut();
            push_u32(file, duration_ms);
            for y in 0..h {
                file.extend_from_slice(pixels.row(y));
            }
            st.pushed += 1;
            return Ok(());
        }
        let mut bytes = Vec::with_capacity(w as usize * h as usize * desc.bytes_per_pixel());
        for y in 0..h {
            bytes.extend_from_slice(pixels.row(y));
        }
        self.frames.push((bytes, w, h, duration_ms, desc));
        Ok(())
    }

    fn begin(
        &mut self,
        width: u32,
        height: u32,
        desc: PixelDescriptor,
        frame_count: Option<u32>,
    ) -> Result<AnimationOutputTiming, RefError> {
        if self.streaming.is_some() {
            return Err(RefError::Invalid("begin called twice".into()));
        }
        if !self.frames.is_empty() {
            return Err(RefError::Invalid("begin after push_frame".into()));
        }
        // The header carries the frame count: without one, nothing can be
        // written before `finish`.
        let Some(frame_count) = frame_count else {
            return Ok(AnimationOutputTiming::AT_FINISH);
        };
        if frame_count == 0 {
            return Err(RefError::Invalid("no frames".into()));
        }
        let mut out = OutputBuffer::new();
        write_header(
            out.vec_mut(),
            width,
            height,
            frame_count,
            desc.bytes_per_pixel() as u8,
            &self.metadata,
            self.gain_map.as_ref(),
        );
        self.streaming = Some(RefAnimStream {
            width,
            height,
            desc,
            frame_count,
            pushed: 0,
            out,
        });
        Ok(AnimationOutputTiming::streaming(0))
    }

    fn pending_output(&self) -> &[u8] {
        self.streaming.as_ref().map_or(&[], |st| st.out.pending())
    }

    fn consume_output(&mut self, n: usize) {
        if let Some(st) = &mut self.streaming {
            st.out.consume(n);
        }
    }

    fn take_output(&mut self, into: &mut Vec<u8>) {
        if let Some(st) = &mut self.streaming {
            st.out.take_into(into);
        }
    }

    fn push_frame_exact(
        &mut self,
        pixels: PixelSlice<'_>,
        duration: zencodec::animation::FrameDuration,
        stop: Option<&dyn zencodec::Pulse>,
    ) -> Result<(), RefError> {
        let ticks = duration
            .ticks_at(1000)
            .map_err(|e| RefError::Invalid(e.to_string()))?;
        let ticks =
            u32::try_from(ticks).map_err(|_| RefError::Invalid("duration overflow".into()))?;
        self.push_frame(pixels, ticks, stop)
    }

    fn push_frame_timed(
        &mut self,
        pixels: PixelSlice<'_>,
        duration: zencodec::animation::FrameDuration,
        stop: Option<&dyn zencodec::Pulse>,
    ) -> Result<zencodec::animation::FrameDuration, RefError> {
        let mut timing = self.timing;
        let encoded = timing
            .quantize(duration)
            .map_err(|e| RefError::Invalid(e.to_string()))?;
        self.push_frame_exact(pixels, encoded, stop)?;
        self.timing = timing;
        Ok(encoded)
    }

    fn finish(self, stop: Option<&dyn zencodec::Pulse>) -> Result<EncodeOutput, RefError> {
        if let Some(s) = stop {
            s.check()?;
        }
        if let Some(st) = self.streaming {
            if st.pushed != st.frame_count {
                return Err(RefError::Invalid(format!(
                    "finish: {} frames pushed, {} declared at begin",
                    st.pushed, st.frame_count
                )));
            }
            return Ok(EncodeOutput::new(st.out.into_remaining(), ImageFormat::Pnm));
        }
        let (first_bytes, w, h, _, desc) = self
            .frames
            .first()
            .ok_or_else(|| RefError::Invalid("no frames".into()))?;
        let bpp = desc.bytes_per_pixel() as u8;
        let mut out = Vec::new();
        write_header(
            &mut out,
            *w,
            *h,
            self.frames.len() as u32,
            bpp,
            &self.metadata,
            self.gain_map.as_ref(),
        );
        push_u32(&mut out, self.frames[0].3);
        out.extend_from_slice(first_bytes);
        for (bytes, _, _, dur, _) in &self.frames[1..] {
            push_u32(&mut out, *dur);
            out.extend_from_slice(bytes);
        }
        Ok(EncodeOutput::new(out, ImageFormat::Pnm))
    }
}

// ===========================================================================
// Decode: Config -> Job -> Decode / StreamingDecode / AnimationFrameDecoder
// ===========================================================================

/// Reference decoder configuration.
#[derive(Clone, Debug, Default)]
pub struct ReferenceDecoderConfig;

impl DecoderConfig for ReferenceDecoderConfig {
    type Error = RefError;
    type Job<'a> = RefDecodeJob;

    fn formats() -> &'static [ImageFormat] {
        &[ImageFormat::Pnm]
    }
    fn supported_descriptors() -> &'static [PixelDescriptor] {
        &DESCRIPTORS
    }
    fn capabilities() -> &'static DecodeCapabilities {
        &DECODE_CAPS
    }
    fn job<'a>(self) -> Self::Job<'a> {
        RefDecodeJob {
            limits: ResourceLimits::none(),
            render: GainMapRender::BaseOnly,
            format: ImageFormat::Pnm,
        }
    }
}

fn detect_zcr(data: &[u8]) -> bool {
    data.starts_with(MAGIC)
}

/// An [`ImageFormatDefinition`] whose `detect` matches the reference codec's
/// `ZCR1` wire magic — the piece [`ReferenceDecoderConfig`] lacks.
///
/// The reference decoder registers under [`ImageFormat::Pnm`] (standing in for
/// "a real codec"), but the bytes it produces are its own `ZCR1` container, not
/// PNM — so [`CodecSet::detect`](zencodec::CodecSet::detect) can't route encoded
/// reference bytes back to it. Register [`ReferenceZcrDecoderConfig`] under this
/// format and detection-based decode (`decode` / `probe` / `estimate_decode_of`)
/// works end to end, without every example hand-rolling its own custom format.
pub static ZCR_FORMAT: ImageFormatDefinition = ImageFormatDefinition::new(
    "zcr",
    None,
    "ZCR (reference codec wire format)",
    "zcr",
    &["zcr"],
    "image/x-zcr",
    &["image/x-zcr"],
    true,  // alpha
    true,  // animation
    true,  // lossless
    false, // lossy
    MAGIC.len(),
    detect_zcr,
);

static ZCR_FORMATS: &[ImageFormat] = &[ImageFormat::Custom(&ZCR_FORMAT)];

/// The reference decoder registered under the self-detecting [`ZCR_FORMAT`]
/// instead of [`ImageFormat::Pnm`].
///
/// Drop-in for [`ReferenceDecoderConfig`] on a [`CodecSet`](zencodec::CodecSet)
/// when you want `set.decode(bytes)` / `set.probe(bytes)` to detect the
/// reference codec's own bytes. Decode behavior is identical — only the
/// registration format differs.
#[derive(Clone, Debug, Default)]
pub struct ReferenceZcrDecoderConfig;

impl DecoderConfig for ReferenceZcrDecoderConfig {
    type Error = RefError;
    type Job<'a> = RefDecodeJob;

    fn formats() -> &'static [ImageFormat] {
        ZCR_FORMATS
    }
    fn supported_descriptors() -> &'static [PixelDescriptor] {
        ReferenceDecoderConfig::supported_descriptors()
    }
    fn capabilities() -> &'static DecodeCapabilities {
        ReferenceDecoderConfig::capabilities()
    }
    fn job<'a>(self) -> Self::Job<'a> {
        RefDecodeJob {
            limits: ResourceLimits::none(),
            render: GainMapRender::BaseOnly,
            format: ZCR_FORMATS[0],
        }
    }
}

/// Reference decode job.
pub struct RefDecodeJob {
    limits: ResourceLimits,
    render: GainMapRender,
    /// The format the job's config registered under; inventories report it.
    format: ImageFormat,
}

impl<'a> DecodeJob<'a> for RefDecodeJob {
    type Error = RefError;
    type Dec = RefDec<'a>;
    type StreamDec = RefStreamDec<'a>;
    type AnimationFrameDec = RefAnimDec;

    fn with_pulse(self, _stop: SharedPulse) -> Self {
        self
    }
    fn with_limits(mut self, limits: ResourceLimits) -> Self {
        self.limits = limits;
        self
    }
    /// The 2-way toggle is exactly `Components` / `BaseOnly`.
    fn with_extract_gain_map(mut self, extract: bool) -> Self {
        self.render = if extract {
            GainMapRender::Components
        } else {
            GainMapRender::BaseOnly
        };
        self
    }
    fn with_gain_map_render(mut self, render: GainMapRender) -> Self {
        self.render = render;
        self
    }

    fn probe(&self, data: &[u8]) -> Result<ImageInfo, RefError> {
        Ok(build_info(&parse_header(data)?))
    }

    fn inventory(&self, data: &[u8]) -> Result<zencodec::inventory::Inventory, At<CodecError>> {
        self.limits
            .check_input_size(data.len() as u64)
            .map_err(RefError::from)?;
        zcr_inventory(data, self.format).map_err(Into::into)
    }

    fn output_info(&self, data: &[u8]) -> Result<OutputInfo, RefError> {
        let h = parse_header(data)?;
        Ok(OutputInfo::full_decode(
            h.width,
            h.height,
            descriptor_for_header(&h)?,
        ))
    }

    fn decoder(
        self,
        data: Cow<'a, [u8]>,
        _preferred: &[PixelDescriptor],
    ) -> Result<RefDec<'a>, RefError> {
        parse_header(&data)?; // validate eagerly
        Ok(RefDec {
            data,
            render: self.render,
        })
    }

    fn push_decoder(
        self,
        data: Cow<'a, [u8]>,
        sink: &mut dyn DecodeRowSink,
        preferred: &[PixelDescriptor],
    ) -> Result<OutputInfo, RefError> {
        zencodec::helpers::copy_decode_to_sink(self, data, sink, preferred, RefError::Sink)
    }

    fn streaming_decoder(
        self,
        data: Cow<'a, [u8]>,
        _preferred: &[PixelDescriptor],
    ) -> Result<RefStreamDec<'a>, RefError> {
        let h = parse_header(&data)?;
        let info = build_info(&h);
        let desc = descriptor_for_header(&h)?;
        let ctx = class_gated_context(&info.source_color, desc.color_model());
        Ok(RefStreamDec {
            data,
            info,
            width: h.width,
            height: h.height,
            bpp: h.bpp,
            desc,
            pixels_offset: frame_pixels_offset(&h, 0),
            next_row: 0,
            ctx,
        })
    }

    fn dyn_incremental_decoder(
        self,
        _preferred: &[PixelDescriptor],
    ) -> Result<Box<zencodec::decode::DynIncrementalDecoder>, zencodec::decode::BoxedError> {
        Ok(zencodec::decode::erase_incremental(RefIncDec::new(
            self.limits,
        )))
    }

    /// Pull-native path: the header is read by growing prefix (the wire
    /// format is self-delimiting, so a complete header parses and a short
    /// one does not), then strips are fetched two at a time through
    /// `read_many_at` so a batching source is exercised.
    fn push_decoder_from_source(
        self,
        source: &mut dyn DynReadAt,
        sink: &mut dyn DecodeRowSink,
        _preferred: &[PixelDescriptor],
    ) -> Result<OutputInfo, zencodec::decode::BoxedError> {
        // Short reads find the end; `len()` is never needed, so a source
        // over a stream is not consumed to learn its length.
        let mut want = 64usize;
        let h = loop {
            let mut prefix = vec![0u8; want];
            let n = source.read_at(0, &mut prefix)?;
            prefix.truncate(n);
            self.limits
                .check_input_size(n as u64)
                .map_err(RefError::from)?;
            match parse_header(&prefix) {
                Ok(h) => break h,
                Err(e) if n < want => return Err(Box::new(e)),
                Err(_) => want *= 4,
            }
        };
        let info = build_info(&h);
        let desc = descriptor_for_header(&h)?;
        let _ = info;
        let row_bytes = h.width as usize * h.bpp as usize;
        let base = frame_pixels_offset(&h, 0) as u64;
        sink.begin(h.width, h.height, desc)
            .map_err(RefError::Sink)?;
        let mut y = 0u32;
        while y < h.height {
            // Two strips of up to 4 rows per source round.
            let mut strips: Vec<(u32, u32, Vec<u8>)> = Vec::new();
            let mut yy = y;
            while yy < h.height && strips.len() < 2 {
                let rows = (h.height - yy).min(4);
                strips.push((yy, rows, vec![0u8; rows as usize * row_bytes]));
                yy += rows;
            }
            {
                let mut reqs: Vec<(u64, &mut [u8])> = strips
                    .iter_mut()
                    .map(|(sy, _, buf)| (base + *sy as u64 * row_bytes as u64, &mut buf[..]))
                    .collect();
                source.read_many_at(&mut reqs)?;
            }
            for (sy, rows, buf) in &strips {
                let mut dst = sink
                    .provide_next_buffer(*sy, *rows, h.width, desc)
                    .map_err(RefError::Sink)?;
                for r in 0..*rows {
                    let start = r as usize * row_bytes;
                    dst.row_mut(r)
                        .copy_from_slice(&buf[start..start + row_bytes]);
                }
            }
            y = yy;
        }
        sink.finish().map_err(RefError::Sink)?;
        Ok(OutputInfo::full_decode(h.width, h.height, desc))
    }

    fn animation_frame_decoder(
        self,
        data: Cow<'a, [u8]>,
        _preferred: &[PixelDescriptor],
    ) -> Result<RefAnimDec, RefError> {
        let h = parse_header(&data)?;
        let info = build_info(&h);
        let desc = descriptor_for_header(&h)?;
        let ctx = class_gated_context(&info.source_color, desc.color_model());
        Ok(RefAnimDec {
            data: data.into_owned(),
            info,
            width: h.width,
            height: h.height,
            frame_count: h.frame_count,
            bpp: h.bpp,
            desc,
            frames_offset: h.frames_offset,
            next_frame: 0,
            ctx,
        })
    }
}

/// Reference one-shot decoder.
#[derive(Debug)]
pub struct RefDec<'a> {
    data: Cow<'a, [u8]>,
    render: GainMapRender,
}

impl Decode for RefDec<'_> {
    type Error = RefError;

    fn decode(self) -> Result<DecodeOutput, RefError> {
        let h = parse_header(&self.data)?;
        let desc = descriptor_for_header(&h)?;
        let start = frame_pixels_offset(&h, 0);
        let len = frame_pixel_len(&h);
        let pixels = self
            .data
            .get(start..start + len)
            .ok_or_else(|| RefError::Invalid("truncated pixels".into()))?;
        let mut buf = PixelBuffer::from_vec(pixels.to_vec(), h.width, h.height, desc)
            .map_err(|e| RefError::Invalid(format!("buffer: {e}")))?;
        let info = build_info(&h);
        // Attach after the descriptor is final, before anyone negotiates on it.
        if let Some(ctx) = class_gated_context(&info.source_color, desc.color_model()) {
            buf = buf.with_color_context(ctx);
        }
        let mut out = DecodeOutput::new(buf, info);
        // Opt-in only: `BaseOnly` (the default) never carries the gain map.
        // `ReconstructHdr` is not honoured (`reconstructs_hdr` is false — no HDR
        // math here), so per the contract it surfaces the components instead of
        // relabelling the SDR base as HDR.
        let surface = matches!(
            self.render,
            GainMapRender::Components | GainMapRender::ReconstructHdr { .. }
        );
        if surface && let Some(gm) = &h.gain_map {
            out = out.with_extras(gm.to_decoded()?);
        }
        Ok(out)
    }
}

/// Reference streaming decoder. Yields strips of up to 4 rows.
pub struct RefStreamDec<'a> {
    data: Cow<'a, [u8]>,
    info: ImageInfo,
    width: u32,
    height: u32,
    bpp: u8,
    desc: PixelDescriptor,
    pixels_offset: usize,
    next_row: u32,
    /// Computed once; re-applied to every emitted strip (strips are fresh
    /// slices, so the context has to be attached at emission).
    ctx: Option<Arc<ColorContext>>,
}

impl StreamingDecode for RefStreamDec<'_> {
    type Error = RefError;

    fn next_batch(&mut self) -> Result<Option<(u32, PixelSlice<'_>)>, RefError> {
        if self.next_row >= self.height {
            return Ok(None);
        }
        let strip = (self.height - self.next_row).min(4);
        let row_bytes = self.width as usize * self.bpp as usize;
        let off = self.pixels_offset + self.next_row as usize * row_bytes;
        let span = strip as usize * row_bytes;
        let bytes = self
            .data
            .get(off..off + span)
            .ok_or_else(|| RefError::Invalid("truncated strip".into()))?;
        let mut ps = PixelSlice::new(bytes, self.width, strip, row_bytes, self.desc)
            .map_err(|e| RefError::Invalid(format!("slice: {e}")))?;
        if let Some(ctx) = &self.ctx {
            ps = ps.with_color_context(ctx.clone());
        }
        let y = self.next_row;
        self.next_row += strip;
        Ok(Some((y, ps)))
    }

    fn info(&self) -> &ImageInfo {
        &self.info
    }
}

/// Reference incremental decoder: the hand-written template for the
/// [`IncrementalDecode`] contract.
///
/// It keeps no input buffer. It asks for a growing prefix until the
/// self-delimiting header parses, then for each strip of up to four rows
/// with `min_len` set to the strip's size, and decodes straight from the
/// driver's bytes into the sink. Termination and cancellation rules come
/// from [`Fused`](zencodec::decode::Fused) (applied by
/// [`erase_incremental`](zencodec::decode::erase_incremental)), so the state
/// machine below handles only decoding. Only the first frame is decoded,
/// like the one-shot path.
pub struct RefIncDec {
    limits: ResourceLimits,
    header: Option<(Header, PixelDescriptor, ImageInfo)>,
    /// Bytes of prefix asked for while the header does not parse yet.
    header_want: usize,
    next_row: u32,
    request: [InputRequest; 1],
}

impl RefIncDec {
    fn new(limits: ResourceLimits) -> Self {
        Self {
            limits,
            header: None,
            header_want: 64,
            next_row: 0,
            request: [InputRequest::at_least(0, 64)],
        }
    }

    fn need(&mut self, offset: u64, min_len: usize) -> Progress {
        self.request = [InputRequest::at_least(offset, min_len)];
        Progress::need(0)
    }
}

impl IncrementalDecode for RefIncDec {
    type Error = RefError;

    fn step(
        &mut self,
        input: Input<'_>,
        sink: &mut dyn DecodeRowSink,
        _stop: &dyn zencodec::Pulse,
    ) -> Result<Progress, StepError<RefError>> {
        if let Some(total) = input.total_len {
            self.limits
                .check_input_size(total)
                .map_err(|e| StepError::Codec(RefError::from(e)))?;
        }
        let Some((h, desc, _)) = &self.header else {
            if input.offset != 0 {
                return Ok(self.need(0, self.header_want));
            }
            if input.bytes.len() >= MAGIC.len() && !input.bytes.starts_with(MAGIC) {
                return Err(StepError::Codec(RefError::Invalid("bad magic".into())));
            }
            return match parse_header(input.bytes) {
                Ok(h) => {
                    let desc = descriptor_for_header(&h).map_err(StepError::Codec)?;
                    let info = build_info(&h);
                    sink.begin(h.width, h.height, desc)
                        .map_err(StepError::Sink)?;
                    // Take the header (and the frame's duration field):
                    // decoding continues at the first strip.
                    let consumed = (frame_pixels_offset(&h, 0)).min(input.bytes.len());
                    self.header = Some((h, desc, info));
                    Ok(Progress::new(
                        consumed,
                        Event::Metadata(MetadataUpdate::HeaderReady),
                    ))
                }
                Err(e) if input.reaches_end() => Err(StepError::Codec(e)),
                Err(_) => {
                    if input.bytes.len() >= self.header_want {
                        self.header_want *= 4;
                    }
                    Ok(self.need(0, self.header_want))
                }
            };
        };
        let (width, height, bpp, desc) = (h.width, h.height, h.bpp, *desc);
        let row_bytes = width as usize * bpp as usize;
        let pixels_start = frame_pixels_offset(h, 0) as u64;
        if self.next_row >= height {
            sink.finish().map_err(StepError::Sink)?;
            return Ok(Progress::new(
                0,
                Event::Done(OutputInfo::full_decode(width, height, desc)),
            ));
        }
        let rows = (height - self.next_row).min(4);
        let strip_start = pixels_start + self.next_row as u64 * row_bytes as u64;
        let strip_len = rows as usize * row_bytes;
        if input.offset != strip_start || input.bytes.len() < strip_len {
            if input.offset == strip_start && input.reaches_end() {
                return Err(StepError::Codec(RefError::Invalid(
                    "truncated strip".into(),
                )));
            }
            return Ok(self.need(strip_start, strip_len));
        }
        {
            let mut dst = sink
                .provide_next_buffer(self.next_row, rows, width, desc)
                .map_err(StepError::Sink)?;
            for r in 0..rows {
                let s = r as usize * row_bytes;
                dst.row_mut(r)
                    .copy_from_slice(&input.bytes[s..s + row_bytes]);
            }
        }
        let meta = BatchMeta::final_rows(self.next_row);
        self.next_row += rows;
        Ok(Progress::new(strip_len, Event::Rows { meta, rows }))
    }

    fn requested(&self) -> &[InputRequest] {
        &self.request
    }

    fn info(&self) -> Option<&ImageInfo> {
        self.header.as_ref().map(|(_, _, info)| info)
    }

    fn output_descriptor(&self) -> Option<PixelDescriptor> {
        self.header.as_ref().map(|(_, desc, _)| *desc)
    }

    fn access_pattern(&self) -> AccessPattern {
        AccessPattern::Sequential
    }
}

/// Reference animation decoder.
pub struct RefAnimDec {
    data: Vec<u8>,
    info: ImageInfo,
    width: u32,
    height: u32,
    frame_count: u32,
    bpp: u8,
    desc: PixelDescriptor,
    frames_offset: usize,
    next_frame: u32,
    /// Re-applied to every yielded frame slice (same reason as the streaming
    /// decoder's).
    ctx: Option<Arc<ColorContext>>,
}

impl AnimationFrameDecoder for RefAnimDec {
    type Error = RefError;

    fn wrap_sink_error(err: SinkError) -> RefError {
        RefError::Sink(err)
    }

    fn info(&self) -> &ImageInfo {
        &self.info
    }
    fn frame_count(&self) -> Option<u32> {
        Some(self.frame_count)
    }
    fn loop_count(&self) -> Option<u32> {
        Some(0)
    }

    fn render_next_frame(
        &mut self,
        stop: Option<&dyn zencodec::Pulse>,
    ) -> Result<Option<AnimationFrame<'_>>, RefError> {
        if let Some(s) = stop {
            s.check()?;
        }
        if self.next_frame >= self.frame_count {
            return Ok(None);
        }
        let row_bytes = self.width as usize * self.bpp as usize;
        let frame_bytes = 4 + self.height as usize * row_bytes;
        let base = self.frames_offset + self.next_frame as usize * frame_bytes;
        let dur = read_u32(&self.data, base)?;
        let start = base + 4;
        let span = self.height as usize * row_bytes;
        let bytes = self
            .data
            .get(start..start + span)
            .ok_or_else(|| RefError::Invalid("truncated frame".into()))?;
        let mut ps = PixelSlice::new(bytes, self.width, self.height, row_bytes, self.desc)
            .map_err(|e| RefError::Invalid(format!("frame slice: {e}")))?;
        if let Some(ctx) = &self.ctx {
            ps = ps.with_color_context(ctx.clone());
        }
        let frame = AnimationFrame::new(ps, dur, self.next_frame);
        self.next_frame += 1;
        Ok(Some(frame))
    }

    fn render_next_frame_to_sink(
        &mut self,
        stop: Option<&dyn zencodec::Pulse>,
        sink: &mut dyn DecodeRowSink,
    ) -> Result<Option<OutputInfo>, RefError> {
        zencodec::helpers::copy_frame_to_sink(self, stop, sink)
    }
}
