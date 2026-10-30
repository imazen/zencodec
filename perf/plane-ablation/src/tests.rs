use super::*;

#[test]
fn native_words_borrow_without_copy_and_codes_share_encoding() {
    let words = [64u16, 65472, 0xbeef, 32768, 128, 0xdead];
    let bytes = bytemuck::cast_slice(&words);
    let p = Plane::new(bytes, ChannelType::U16, Order::Native, 2, 2, 6, 2).unwrap();
    assert_eq!(p.bytes.len(), 10); // excludes unused trailing word
    assert_eq!(
        p.row(0).unwrap().typed::<u16>().unwrap().as_ptr(),
        words.as_ptr()
    );
    assert_eq!(
        p.row(1).unwrap().typed::<u16>().unwrap().as_ptr(),
        words[3..].as_ptr()
    );
    let enc = SampleEncoding::new(ChannelType::U16, 10, 6).unwrap();
    let canonical = zenpixels::sample::CodePlane::from_u16(&words, 2, 2, 6, enc).unwrap();
    let codes = p.codes(enc).unwrap();
    for y in 0..2 {
        for x in 0..2 {
            assert_eq!(codes.code(x, y).unwrap(), canonical.code(x, y).unwrap());
        }
    }
}

#[test]
fn semiplanar_p010_component_stride_preserves_words_and_padding() {
    let uv = [64u16, 128, 65472, 32768, 0xabcd, 192, 256, 320, 384];
    let bytes: &[u8] = bytemuck::cast_slice(&uv);
    let enc = SampleEncoding::new(ChannelType::U16, 10, 6).unwrap();
    for (offset, expected) in [(0, [[1, 1023], [3, 5]]), (2, [[2, 512], [4, 6]])] {
        let p = Plane::new(
            &bytes[offset..],
            ChannelType::U16,
            Order::Native,
            2,
            2,
            10,
            4,
        )
        .unwrap();
        assert_eq!(p.bytes.as_ptr(), bytes[offset..].as_ptr());
        assert_eq!(p.row(0).unwrap().typed::<u16>(), Err(Error::Layout));
        let c = p.codes(enc).unwrap();
        for (y, row) in expected.iter().enumerate() {
            for (x, word) in row.iter().enumerate() {
                assert_eq!(c.code(x, y), Ok(*word));
            }
        }
    }
}

#[test]
fn float_nan_signed_zero_and_wide_scalar_bits_survive() {
    let values = [
        f32::from_bits(0x7fc12345),
        -0.0,
        f32::INFINITY,
        f32::from_bits(1),
    ];
    let p = Plane::new(
        bytemuck::cast_slice(&values),
        ChannelType::F32,
        Order::Native,
        2,
        2,
        8,
        4,
    )
    .unwrap();
    for y in 0..2 {
        let row = p.row(y).unwrap().typed::<f32>().unwrap();
        assert_eq!(row.as_ptr(), values[y * 2..].as_ptr());
        for x in 0..2 {
            assert_eq!(row[x].to_bits(), values[y * 2 + x].to_bits());
        }
    }
    assert_eq!(p.row(0).unwrap().typed::<u32>(), Err(Error::Storage));
    let signed = [i64::MIN, i64::MAX];
    let p = Plane::new(
        bytemuck::cast_slice(&signed),
        ChannelType::I64,
        Order::Native,
        2,
        1,
        16,
        8,
    )
    .unwrap();
    assert_eq!(p.row(0).unwrap().typed::<i64>().unwrap(), signed);
    assert_eq!(p.row(0).unwrap().typed::<u64>(), Err(Error::Storage));
}

#[test]
fn mixed_depth_and_float_planes_need_no_frame_wide_encoding() {
    let y = [1023u16];
    let alpha = [255u8];
    let depth = [1.5f32];
    let planes = [
        Plane::new(
            bytemuck::cast_slice(&y),
            ChannelType::U16,
            Order::Native,
            1,
            1,
            2,
            2,
        )
        .unwrap(),
        Plane::new(&alpha, ChannelType::U8, Order::Native, 1, 1, 1, 1).unwrap(),
        Plane::new(
            bytemuck::cast_slice(&depth),
            ChannelType::F32,
            Order::Native,
            1,
            1,
            4,
            4,
        )
        .unwrap(),
    ];
    assert_eq!(
        planes[0]
            .codes(SampleEncoding::new(ChannelType::U16, 10, 0).unwrap())
            .unwrap()
            .code(0, 0),
        Ok(1023)
    );
    assert_eq!(planes[1].row(0).unwrap().typed::<u8>().unwrap(), alpha);
    assert_eq!(planes[2].row(0).unwrap().typed::<f32>().unwrap(), depth);
}

#[test]
fn unaligned_storage_remains_readable_without_forging_typed_borrows() {
    let aligned = [0x0102030405060708u64; 2];
    let bytes: &[u8] = bytemuck::cast_slice(&aligned);
    let p = Plane::new(&bytes[1..], ChannelType::U16, Order::Native, 2, 2, 5, 2).unwrap();
    assert_eq!(p.row(0).unwrap().typed::<u16>(), Err(Error::Alignment));
    let c = p
        .codes(SampleEncoding::new(ChannelType::U16, 16, 0).unwrap())
        .unwrap();
    for y in 0..2 {
        for x in 0..2 {
            let start = 1 + y * 5 + x * 2;
            assert_eq!(
                c.code(x, y),
                Ok(u16::from_ne_bytes([bytes[start], bytes[start + 1]]))
            );
        }
    }
}

#[test]
fn explicit_wire_endian_is_not_silently_reinterpreted_as_native() {
    for (order, bytes) in [(Order::Big, [0x12, 0x34]), (Order::Little, [0x34, 0x12])] {
        let p = Plane::new(&bytes, ChannelType::U16, order, 1, 1, 2, 2).unwrap();
        assert_eq!(
            p.codes(SampleEncoding::new(ChannelType::U16, 16, 0).unwrap())
                .unwrap()
                .code(0, 0),
            Ok(0x1234)
        );
        if !order.native() {
            assert_eq!(p.row(0).unwrap().typed::<u16>(), Err(Error::Endian));
        }
    }
}

#[test]
fn empty_and_overflow_geometry_never_address_unused_storage() {
    let p = Plane::new(
        &[],
        ChannelType::F64,
        Order::Native,
        0,
        usize::MAX,
        usize::MAX,
        8,
    )
    .unwrap();
    assert!(
        p.row(usize::MAX - 1)
            .unwrap()
            .typed::<f64>()
            .unwrap()
            .is_empty()
    );
    assert_eq!(p.row(0).unwrap().sample(0), Err(Error::Bounds));
    assert!(matches!(
        Plane::new(
            &[],
            ChannelType::U16,
            Order::Native,
            usize::MAX,
            2,
            usize::MAX,
            2
        ),
        Err(Error::Geometry)
    ));
    assert!(matches!(
        Plane::new(&[0; 8], ChannelType::U16, Order::Native, 2, 2, 2, 2),
        Err(Error::Layout)
    ));
    assert!(matches!(
        Plane::new(&[0; 5], ChannelType::U16, Order::Native, 2, 2, 4, 2),
        Err(Error::Storage)
    ));
    assert!(matches!(
        Plane::new(&[], ChannelType::U16, Order::Native, 0, 0, 0, 0),
        Err(Error::Layout)
    ));
}
