//! Large tiles: the region candidate fetches each coding unit as it decodes,
//! so a refusal at a later unit follows the earlier units' compressed reads.
//! A plan from tags admits every unit before any compressed read and fetches
//! the admitted ranges in one transport call; the unchanged region candidate
//! then decodes from the planned bytes with no further transport call.
use native_contract_spatial::{
    image_items::Rect,
    plan::plan,
    tiff_region::{self, Allowances, Selection},
    transport::{Counts, Kept, Planned, Recording, fetch},
};
use std::{cell::RefCell, collections::BTreeMap, io::Cursor, rc::Rc};
use tiff::{encoder::TiffEncoder, tags::Tag};

const SIDE: u32 = 512;
const TILE: u32 = 256;
const TILE_BYTES: usize = (TILE * TILE * 3) as usize;

fn raster() -> Vec<u8> {
    (0..SIDE * SIDE * 3)
        .map(|i| ((i * 61 + i / 19) % 251) as u8)
        .collect()
}

fn crop(r: &[u8], roi: Rect) -> Vec<u8> {
    (roi.y..roi.y + roi.height)
        .flat_map(|y| {
            let at = ((y * SIDE + roi.x) * 3) as usize;
            r[at..at + roi.width as usize * 3].iter().copied()
        })
        .collect()
}

/// 512x512 RGB8 in four uncompressed 256x256 tiles, written by the native
/// directory encoder; returns the file and each tile's stored range. With
/// `inflate_last`, the last tile declares one byte more than it stores.
fn fixture(r: &[u8], inflate_last: bool) -> (Vec<u8>, Vec<(u64, u64)>) {
    let mut cursor = Cursor::new(Vec::new());
    let mut tiles = Vec::new();
    {
        let mut encoder = TiffEncoder::new(&mut cursor).unwrap();
        let mut dir = encoder.image_directory().unwrap();
        let (mut offsets, mut sizes) = (Vec::new(), Vec::new());
        for ty in 0..SIDE / TILE {
            for tx in 0..SIDE / TILE {
                let mut px = Vec::with_capacity(TILE_BYTES);
                for y in 0..TILE {
                    let at = (((ty * TILE + y) * SIDE + tx * TILE) * 3) as usize;
                    px.extend_from_slice(&r[at..at + (TILE * 3) as usize]);
                }
                let offset = dir.write_data(px.as_slice()).unwrap();
                tiles.push((offset, px.len() as u64));
                offsets.push(u32::try_from(offset).unwrap());
                sizes.push(px.len() as u32);
            }
        }
        if inflate_last {
            *sizes.last_mut().unwrap() += 1;
        }
        dir.write_tag(Tag::ImageWidth, SIDE).unwrap();
        dir.write_tag(Tag::ImageLength, SIDE).unwrap();
        dir.write_tag(Tag::BitsPerSample, &[8u16; 3][..]).unwrap();
        dir.write_tag(Tag::SamplesPerPixel, 3u16).unwrap();
        dir.write_tag(Tag::PhotometricInterpretation, 2u16).unwrap();
        dir.write_tag(Tag::Compression, 1u16).unwrap();
        dir.write_tag(Tag::PlanarConfiguration, 1u16).unwrap();
        dir.write_tag(Tag::TileWidth, TILE).unwrap();
        dir.write_tag(Tag::TileLength, TILE).unwrap();
        dir.write_tag(Tag::TileOffsets, offsets.as_slice()).unwrap();
        dir.write_tag(Tag::TileByteCounts, sizes.as_slice())
            .unwrap();
        dir.write_tag(Tag::Orientation, 1u16).unwrap();
        dir.finish().unwrap();
    }
    (cursor.into_inner(), tiles)
}

/// Bytes delivered inside each tile's stored range.
fn tile_bytes(counts: &Counts, tiles: &[(u64, u64)]) -> Vec<u64> {
    tiles
        .iter()
        .map(|&(start, len)| {
            counts
                .reads
                .iter()
                .map(|&(at, n)| {
                    let (lo, hi) = (at.max(start), (at + n).min(start + len));
                    hi.saturating_sub(lo)
                })
                .sum()
        })
        .collect()
}

fn allow() -> Allowances {
    Allowances {
        unit: TILE_BYTES,
        ..Allowances::default()
    }
}

fn state() -> (Rc<RefCell<Counts>>, Kept) {
    (
        Rc::new(RefCell::new(Counts::default())),
        Rc::new(RefCell::new(BTreeMap::new())),
    )
}

const FULL: Rect = Rect {
    x: 0,
    y: 0,
    width: SIDE,
    height: SIDE,
};

#[test]
fn current_region_read_fetches_earlier_tiles_before_refusing_an_oversized_one() {
    let r = raster();
    let (file, tiles) = fixture(&r, true);
    let (counts, kept) = state();
    let selection = Selection {
        page: 0,
        levels: &[],
        region: FULL,
    };
    let err = tiff_region::decode(
        Recording::new(&file, counts.clone(), kept),
        selection,
        allow(),
        || Ok(()),
    )
    .unwrap_err();
    let per_tile = tile_bytes(&counts.borrow(), &tiles);
    eprintln!("current refusal ({err}) after tile bytes {per_tile:?}");
    let t = TILE_BYTES as u64;
    assert_eq!(
        per_tile,
        [t, t, t, 0],
        "three tiles read before the refusal"
    );
}

#[test]
fn planned_region_refuses_the_oversized_tile_before_any_tile_read() {
    let r = raster();
    let (file, tiles) = fixture(&r, true);
    let (counts, kept) = state();
    let err = plan(
        Recording::new(&file, counts.clone(), kept),
        0,
        FULL,
        allow(),
        u64::MAX,
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("compressed unit allowance"),
        "{err}"
    );
    assert_eq!(tile_bytes(&counts.borrow(), &tiles), [0, 0, 0, 0]);
    // A total cap below the honest file's four tiles also refuses unread.
    let (file, tiles) = fixture(&r, false);
    let (counts, kept) = state();
    let err = plan(
        Recording::new(&file, counts.clone(), kept),
        0,
        FULL,
        allow(),
        4 * TILE_BYTES as u64 - 1,
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("compressed read allowance"),
        "{err}"
    );
    assert_eq!(tile_bytes(&counts.borrow(), &tiles), [0, 0, 0, 0]);
}

#[test]
fn planned_region_fetches_admitted_tiles_in_one_call_and_decodes_exactly() {
    let r = raster();
    let (file, tiles) = fixture(&r, false);
    // Crosses all four tiles.
    let roi = Rect {
        x: 200,
        y: 230,
        width: 117,
        height: 61,
    };
    let selection = || Selection {
        page: 0,
        levels: &[],
        region: roi,
    };

    // The current candidate: one transport call per parser read.
    let (direct, kept) = state();
    let region = tiff_region::decode(
        Recording::new(&file, direct.clone(), kept),
        selection(),
        allow(),
        || Ok(()),
    )
    .unwrap();
    assert_eq!(region.bytes, crop(&r, roi));
    let direct_tile_calls = direct
        .borrow()
        .reads
        .iter()
        .filter(|&&(at, n)| tiles.iter().any(|&(s, l)| at < s + l && s < at + n))
        .count();

    // Planned: metadata reads, one batched fetch, then no transport at all.
    let (counts, kept) = state();
    let planned = plan(
        Recording::new(&file, counts.clone(), kept.clone()),
        0,
        roi,
        allow(),
        u64::MAX,
    )
    .unwrap();
    assert_eq!(planned.units.len(), 4);
    assert_eq!(planned.compressed, 4 * TILE_BYTES as u64);
    assert_eq!(planned.largest_unit, TILE_BYTES);
    assert_eq!(planned.output, 117 * 61 * 3);
    let metadata_calls = counts.borrow().calls;
    assert_eq!(tile_bytes(&counts.borrow(), &tiles), [0, 0, 0, 0]);
    let ranges: Vec<_> = planned
        .units
        .iter()
        .flat_map(|u| u.ranges.iter().copied())
        .collect();
    fetch(&file, &ranges, &counts, &kept).unwrap();
    let region = tiff_region::decode(
        Planned::new(kept, file.len() as u64),
        selection(),
        allow(),
        || Ok(()),
    )
    .unwrap();
    assert_eq!(
        region.bytes,
        crop(&r, roi),
        "same samples from planned bytes"
    );
    assert_eq!(
        counts.borrow().calls,
        metadata_calls + 1,
        "decoding made no transport call"
    );
    eprintln!(
        "direct: {} transport calls ({direct_tile_calls} in tile data), {} bytes; \
         planned: {metadata_calls} metadata calls + 1 batch, {} bytes",
        direct.borrow().calls,
        direct.borrow().bytes,
        counts.borrow().bytes
    );
}
