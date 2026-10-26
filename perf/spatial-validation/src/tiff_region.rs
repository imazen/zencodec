pub use super::image_items::Rect;
use super::image_items::{ImageGeometry, RegionRequest};
use std::io::{self, Read, Seek, SeekFrom};
use tiff::{
    ColorType,
    decoder::{Decoder, Limits, TiffCodingUnit, TiffHeader},
    tags::{SampleFormat, Tag},
};

pub type Failure = Box<dyn std::error::Error + Send + Sync>;
fn invalid(message: &'static str) -> Failure {
    Box::new(io::Error::new(io::ErrorKind::InvalidInput, message))
}
fn zeros(n: usize) -> Result<Vec<u8>, Failure> {
    let mut v = Vec::new();
    v.try_reserve_exact(n)?;
    v.resize(n, 0);
    Ok(v)
}
pub struct Selection<'a> {
    /// Main IFD ordinal, not animation time or a spatial coding unit.
    pub page: u32,
    /// Explicit SubIFD indices. No assumption that an arbitrary SubIFD is a
    /// reduced level: each selected directory must declare ReducedImage.
    pub levels: &'a [u32],
    pub region: Rect,
}
#[derive(Clone, Copy)]
pub struct Allowances {
    pub pages: u32,
    pub levels: usize,
    pub units: u64,
    pub discoveries: usize,
    pub output: usize,
    pub unit: usize,
    pub tag: usize,
}
impl Default for Allowances {
    fn default() -> Self {
        Self {
            pages: 1024,
            levels: 16,
            units: 4096,
            discoveries: 4096,
            output: 64 * 1024 * 1024,
            unit: 16 * 1024 * 1024,
            tag: 64 * 1024,
        }
    }
}
#[derive(Debug)]
pub struct Region {
    pub geometry: ImageGeometry,
    pub bytes: Vec<u8>,
    pub stride: usize,
    pub width: u32,
    pub height: u32,
    pub color: ColorType,
    pub sample: SampleFormat,
    pub orientation: u32,
    pub extra_samples: Option<Vec<u16>>,
    pub icc: Option<Vec<u8>>,
}

/// Raw stored-coordinate candidate. It preserves native sample bytes and reports
/// the native sample/color tag facts and opaque ICC bytes. Auxiliary dependencies,
/// complete file validation and display conversion are not established here.
pub fn decode<R: Read + Seek>(
    mut input: R,
    selected: Selection<'_>,
    limits: Allowances,
    mut check: impl FnMut() -> Result<(), Failure>,
) -> Result<Region, Failure> {
    check()?;
    if selected.page >= limits.pages || selected.levels.len() > limits.levels {
        return Err(invalid("discovery allowance"));
    }
    let r = selected.region;
    if r.width == 0 || r.height == 0 {
        return Err(invalid("empty region"));
    }
    input.seek(SeekFrom::Start(0))?;
    let mut header = TiffHeader::parse(&mut input)?;
    let mut native_limits = Limits::default();
    native_limits.decoding_buffer_size = limits.unit;
    native_limits.intermediate_buffer_size = limits.unit;
    native_limits.ifd_value_size = limits.tag;
    let mut decoder = header.open(&mut input).with_limits(native_limits.clone());
    decoder.seek_to_image(selected.page as usize)?;
    let mut seen = vec![
        decoder
            .ifd_pointer()
            .ok_or_else(|| invalid("missing page identity"))?,
    ];
    for &level in selected.levels {
        check()?;
        let children = decoder.current_ifd().get_tag(Tag::SubIfd)?.into_ifd_vec()?;
        let pointer = *children
            .get(level as usize)
            .ok_or_else(|| invalid("missing level"))?;
        if seen.contains(&pointer) {
            return Err(invalid("cyclic level identity"));
        }
        seen.push(pointer);
        header.first_ifd = pointer;
        drop(decoder);
        decoder = header.open(&mut input).with_limits(native_limits.clone());
        decoder.next_image()?;
        let flags = decoder.current_ifd().get_tag_u32(Tag::NewSubfileType)?;
        if flags & 1 == 0 || flags & 4 != 0 {
            return Err(invalid("SubIFD is not a reduced image"));
        }
    }
    decode_current(decoder, r.into(), limits, check)
}

/// Candidate adapter seam: the caller has already validated an operation-bound
/// item handle. An arbitrary SubIFD is not required to be a reduced image.
pub(super) fn decode_directory<R: Read + Seek>(
    mut input: R,
    directory: tiff::tags::IfdPointer,
    region: RegionRequest,
    limits: Allowances,
    mut check: impl FnMut() -> Result<(), Failure>,
) -> Result<Region, Failure> {
    check()?;
    input.seek(SeekFrom::Start(0))?;
    let mut header = TiffHeader::parse(&mut input)?;
    header.first_ifd = directory;
    let mut native = Limits::default();
    native.decoding_buffer_size = limits.unit;
    native.intermediate_buffer_size = limits.unit;
    native.ifd_value_size = limits.tag;
    let mut decoder = header.open(input).with_limits(native);
    decoder.next_image()?;
    decode_current(decoder, region, limits, check)
}

fn decode_current<R: Read + Seek>(
    mut decoder: Decoder<R>,
    region: RegionRequest,
    limits: Allowances,
    mut check: impl FnMut() -> Result<(), Failure>,
) -> Result<Region, Failure> {
    let (width, height) = decoder.dimensions()?;
    let r = region.resolve(width, height)?;
    let right = r.x + r.width;
    let bottom = r.y + r.height;
    let color = decoder.colortype()?;
    if !matches!(
        color,
        ColorType::Gray(_) | ColorType::GrayA(_) | ColorType::RGB(_) | ColorType::RGBA(_)
    ) || !matches!(color.bit_depth(), 8 | 16 | 32 | 64)
    {
        return Err(invalid("unqualified native sample layout"));
    }
    let sample_bytes = usize::from(color.bit_depth() / 8);
    let channels = usize::from(color.num_samples());
    let bpp = channels * sample_bytes;
    let stride = (r.width as usize)
        .checked_mul(bpp)
        .ok_or_else(|| invalid("output stride overflow"))?;
    let length = stride
        .checked_mul(r.height as usize)
        .filter(|&n| n <= limits.output)
        .ok_or_else(|| invalid("output allowance"))?;
    let (cw, ch) = decoder.chunk_dimensions();
    if cw == 0 || ch == 0 {
        return Err(invalid("zero coding unit geometry"));
    }
    let columns = width.div_ceil(cw);
    let x0 = r.x / cw;
    let x1 = (right - 1) / cw;
    let y0 = r.y / ch;
    let y1 = (bottom - 1) / ch;
    if u64::from(x1 - x0 + 1) * u64::from(y1 - y0 + 1) > limits.units {
        return Err(invalid("coding unit allowance"));
    }
    // Fallible tag lookup retains malformed-value errors rather than silently
    // assigning orientation 1. Absence alone has the TIFF default.
    let orientation = decoder
        .current_ifd()
        .find_tag(Tag::Orientation)?
        .map(|v| v.into_u32())
        .transpose()?
        .unwrap_or(1);
    if !(1..=8).contains(&orientation) {
        return Err(invalid("orientation outside 1..8"));
    }
    let first = y0
        .checked_mul(columns)
        .and_then(|n| n.checked_add(x0))
        .ok_or_else(|| invalid("unit index overflow"))?;
    let layout = decoder.image_coding_unit_layout(TiffCodingUnit(first))?;
    let sample = layout.sample_format;
    let extra_samples = decoder
        .current_ifd()
        .find_tag(Tag::ExtraSamples)?
        .map(|v| v.into_u16_vec())
        .transpose()?;
    let icc = decoder
        .current_ifd()
        .find_tag(Tag::IccProfile)?
        .map(|v| v.into_u8_vec())
        .transpose()?;
    let mut out = Region {
        geometry: ImageGeometry {
            canvas_width: width,
            canvas_height: height,
            region: r,
        },
        bytes: zeros(length)?,
        stride,
        width: r.width,
        height: r.height,
        color,
        sample,
        orientation,
        extra_samples,
        icc,
    };
    for y in y0..=y1 {
        for x in x0..=x1 {
            check()?;
            let index = y
                .checked_mul(columns)
                .and_then(|n| n.checked_add(x))
                .ok_or_else(|| invalid("unit index overflow"))?;
            let unit = TiffCodingUnit(index);
            let layout = decoder.image_coding_unit_layout(unit)?;
            if layout.complete_len > limits.unit {
                return Err(invalid("coding unit storage allowance"));
            }
            let mut decoded = zeros(layout.complete_len)?;
            decoder.read_coding_unit_bytes(unit, &mut decoded)?;
            let row_stride = layout
                .row_stride
                .ok_or_else(|| invalid("unknown row stride"))?
                .get();
            let plane_stride = layout
                .plane_stride
                .ok_or_else(|| invalid("unknown plane stride"))?
                .get();
            if layout.planes != 1 && layout.planes != channels {
                return Err(invalid("unqualified plane layout"));
            }
            let (left, top) = (r.x.max(x * cw), r.y.max(y * ch));
            let (end_x, end_y) = (
                right.min(width.min((x * cw).saturating_add(cw))),
                bottom.min(height.min((y * ch).saturating_add(ch))),
            );
            for py in top..end_y {
                for px in left..end_x {
                    let destination = (py - r.y) as usize * stride + (px - r.x) as usize * bpp;
                    if layout.planes == 1 {
                        let start =
                            (py - y * ch) as usize * row_stride + (px - x * cw) as usize * bpp;
                        out.bytes[destination..destination + bpp]
                            .copy_from_slice(&decoded[start..start + bpp]);
                    } else {
                        for channel in 0..channels {
                            let start = channel * plane_stride
                                + (py - y * ch) as usize * row_stride
                                + (px - x * cw) as usize * sample_bytes;
                            let to = destination + channel * sample_bytes;
                            out.bytes[to..to + sample_bytes]
                                .copy_from_slice(&decoded[start..start + sample_bytes]);
                        }
                    }
                }
            }
        }
    }
    check()?;
    Ok(out)
}
