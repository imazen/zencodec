#![no_main]
use libfuzzer_sys::fuzz_target;
use zencodec_media::{
    color::{ChromaLocation, Subsampling, YuvToRgb, YuvView},
    display::OutOfRange,
    frame::{RgbStorage, to_rgb},
    plane::{Plane, Samples},
};
use zenpixels::{ChannelType, Cicp, sample::SampleEncoding};

fuzz_target!(|data: &[u8]| {
    if data.len() < 16 {
        return;
    }
    let w = usize::from(data[0] % 17 + 1);
    let h = usize::from(data[1] % 17 + 1);
    let bits = data[2] % 9 + 8;
    let shift = data[3] % (17 - bits);
    let encoding = SampleEncoding::new(ChannelType::U16, bits, shift).unwrap();
    let sampling = [
        Subsampling::Monochrome,
        Subsampling::Yuv420,
        Subsampling::Yuv422,
        Subsampling::Yuv444,
    ][usize::from(data[4] % 4)];
    let location = [
        ChromaLocation::Unknown,
        ChromaLocation::Center,
        ChromaLocation::Left,
        ChromaLocation::TopLeft,
    ][usize::from(data[5] % 4)];
    let matrix = [0, 1, 5, 6, 9][usize::from(data[6] % 5)];
    let color = Cicp::new(data[7], data[8], matrix, data[9] & 1 != 0);
    let cw = if matches!(sampling, Subsampling::Yuv420 | Subsampling::Yuv422) {
        w.div_ceil(2)
    } else {
        w
    };
    let ch = if sampling == Subsampling::Yuv420 {
        h.div_ceil(2)
    } else {
        h
    };
    let stride = w + usize::from(data[10] % 4);
    let cstride = cw + usize::from(data[11] % 4);
    // Raw words deliberately include dirty high/low padding. The Plane contract
    // must mask it before range reconstruction and must ignore padded columns.
    let words = |length: usize, offset: usize| -> Vec<u16> {
        (0..length)
            .map(|i| {
                u16::from_le_bytes([
                    data[(offset + i * 2) % data.len()],
                    data[(offset + i * 2 + 1) % data.len()],
                ])
            })
            .collect()
    };
    let y = words(stride * h, 12);
    let u = words(cstride * ch, 13);
    let v = words(cstride * ch, 14);
    let plane = |words, width, height, stride| {
        Plane::new(Samples::U16(words), width, height, stride * 2, encoding).unwrap()
    };
    let view = YuvView::new(
        plane(&y, w, h, stride),
        (sampling != Subsampling::Monochrome)
            .then(|| [plane(&u, cw, ch, cstride), plane(&v, cw, ch, cstride)]),
        sampling,
        location,
        color,
    )
    .unwrap();
    let Ok(full) = YuvToRgb::new(view) else {
        return;
    };
    let x = usize::from(data[12]) % w;
    let top = usize::from(data[13]) % h;
    let width = usize::from(data[14]) % (w - x) + 1;
    let height = usize::from(data[15]) % (h - top) + 1;
    let crop_view = view.crop(x, top, width, height).unwrap();
    let cropped = YuvToRgb::new(crop_view).unwrap();
    let mut full_row = vec![[0.0; 3]; w];
    let mut crop_row = vec![[0.0; 3]; width];
    for row in 0..height {
        full.write_row(top + row, &mut full_row).unwrap();
        cropped.write_row(row, &mut crop_row).unwrap();
        assert_eq!(&full_row[x..x + width], crop_row);
        assert!(crop_row.iter().flatten().all(|v| v.is_finite()));
    }
    for storage in [RgbStorage::U8, RgbStorage::U16, RgbStorage::F32] {
        let full = to_rgb(view, storage, OutOfRange::Clamp, None, 8192, None).unwrap();
        let cropped = to_rgb(crop_view, storage, OutOfRange::Clamp, None, 8192, None).unwrap();
        let size = full.descriptor().bytes_per_pixel();
        for row in 0..height {
            assert_eq!(
                cropped.as_slice().row(row as u32),
                &full.as_slice().row((top + row) as u32)[x * size..(x + width) * size]
            );
        }
        assert_eq!(full.descriptor(), cropped.descriptor());
        assert!(to_rgb(view, storage, OutOfRange::Clamp, None, 0, None).is_err());
    }
});
