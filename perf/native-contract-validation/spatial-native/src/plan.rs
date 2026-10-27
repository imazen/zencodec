//! Region read plan: the coding units a stored region needs, their stored
//! compressed ranges (every plane) and decoded sizes, from tags alone.
use crate::{
    image_items::Rect,
    tiff_region::{Allowances, Failure},
};
use std::io::{self, Read, Seek, SeekFrom};
use tiff::{
    decoder::{Limits, TiffCodingUnit, TiffHeader},
    tags::Tag,
};

fn refuse(message: &'static str) -> Failure {
    Box::new(io::Error::new(io::ErrorKind::InvalidInput, message))
}

/// One coding unit of a plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnitPlan {
    /// Unit index within one plane.
    pub unit: u32,
    /// Stored compressed `(offset, length)` of each plane's chunk.
    pub ranges: Vec<(u64, u64)>,
    /// Decoded bytes of the whole unit, every plane.
    pub decoded: usize,
}

/// Everything a region read touches, known before its compressed reads.
#[derive(Clone, Debug)]
pub struct RegionPlan {
    /// Units intersecting the region, row-major.
    pub units: Vec<UnitPlan>,
    /// Sum of the units' compressed lengths.
    pub compressed: u64,
    /// Largest decoded unit: the workspace of a one-unit-at-a-time read.
    pub largest_unit: usize,
    /// Selected output bytes.
    pub output: usize,
}

/// Plans `region` of main image `page`. Reads the header, the directory chain
/// and its tags, never a coding unit. Refuses when a unit's compressed or
/// decoded size exceeds `allow.unit`, more than `allow.units` units are
/// needed, the output exceeds `allow.output`, or the compressed total exceeds
/// `max_compressed`.
pub fn plan<R: Read + Seek>(
    mut input: R,
    page: u32,
    region: Rect,
    allow: Allowances,
    max_compressed: u64,
) -> Result<RegionPlan, Failure> {
    input.seek(SeekFrom::Start(0))?;
    let header = TiffHeader::parse(&mut input)?;
    let mut limits = Limits::default();
    limits.decoding_buffer_size = allow.unit;
    limits.intermediate_buffer_size = allow.unit;
    limits.ifd_value_size = allow.tag;
    let mut decoder = header.open(&mut input).with_limits(limits);
    decoder.seek_to_image(page as usize)?;
    let (width, height) = decoder.dimensions()?;
    let right = region
        .x
        .checked_add(region.width)
        .filter(|&r| r <= width && region.width > 0)
        .ok_or_else(|| refuse("region outside the image"))?;
    let bottom = region
        .y
        .checked_add(region.height)
        .filter(|&b| b <= height && region.height > 0)
        .ok_or_else(|| refuse("region outside the image"))?;
    let color = decoder.colortype()?;
    let bpp = usize::from(color.bit_depth() / 8) * usize::from(color.num_samples());
    let output = (region.width as usize)
        .checked_mul(region.height as usize)
        .and_then(|n| n.checked_mul(bpp))
        .filter(|&n| n > 0 && n <= allow.output)
        .ok_or_else(|| refuse("output allowance"))?;
    // Metadata the region read consults, so a planned reader holds it too.
    for tag in [Tag::Orientation, Tag::ExtraSamples, Tag::IccProfile] {
        let _ = decoder.current_ifd().find_tag(tag)?;
    }
    let (cw, ch) = decoder.chunk_dimensions();
    if cw == 0 || ch == 0 {
        return Err(refuse("zero coding unit geometry"));
    }
    let columns = width.div_ceil(cw);
    let per_plane = columns as usize * height.div_ceil(ch) as usize;
    let tiled = decoder.current_ifd().find_tag(Tag::TileOffsets)?.is_some();
    let (offsets_tag, counts_tag) = if tiled {
        (Tag::TileOffsets, Tag::TileByteCounts)
    } else {
        (Tag::StripOffsets, Tag::StripByteCounts)
    };
    let offsets = decoder.current_ifd().get_tag(offsets_tag)?.into_u64_vec()?;
    let counts = decoder.current_ifd().get_tag(counts_tag)?.into_u64_vec()?;
    if offsets.len() != counts.len() || per_plane == 0 || offsets.len() % per_plane != 0 {
        return Err(refuse("chunk table does not match the geometry"));
    }
    let planes = offsets.len() / per_plane;
    let (x0, x1) = (region.x / cw, (right - 1) / cw);
    let (y0, y1) = (region.y / ch, (bottom - 1) / ch);
    if u64::from(x1 - x0 + 1) * u64::from(y1 - y0 + 1) > allow.units {
        return Err(refuse("coding unit allowance"));
    }
    let mut units = Vec::new();
    let mut compressed = 0u64;
    let mut largest_unit = 0;
    for y in y0..=y1 {
        for x in x0..=x1 {
            let unit = y * columns + x;
            let decoded = decoder
                .image_coding_unit_layout(TiffCodingUnit(unit))?
                .complete_len;
            if decoded > allow.unit {
                return Err(refuse("coding unit storage allowance"));
            }
            let mut ranges = Vec::with_capacity(planes);
            for plane in 0..planes {
                let i = unit as usize + plane * per_plane;
                let len = counts[i];
                // The parser's own intermediate limit, applied before any read.
                if len > allow.unit as u64 {
                    return Err(refuse("compressed unit allowance"));
                }
                compressed = compressed
                    .checked_add(len)
                    .ok_or_else(|| refuse("compressed size overflow"))?;
                ranges.push((offsets[i], len));
            }
            largest_unit = largest_unit.max(decoded);
            units.push(UnitPlan {
                unit,
                ranges,
                decoded,
            });
        }
    }
    if compressed > max_compressed {
        return Err(refuse("compressed read allowance"));
    }
    Ok(RegionPlan {
        units,
        compressed,
        largest_unit,
        output,
    })
}
