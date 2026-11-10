//! Minimal real-native mapping witnesses, separate from numerical comparison cohorts.
#![forbid(unsafe_code)]
#[cfg(test)]
mod tests {
    use std::{io::Cursor, sync::Arc};
    use zencodec::frame::{
        ColorClaim, Frame, FrameGeometry, FrameInterpretation, FrameTiming, MapError, MappedPlanes,
        Picture, Presentation,
    };
    use zencodec_media::frame::{OwnedPlane, OwnedPlanes};
    use zencodec_media_testkit::check_frame_interchange;
    use zenpixels::{
        ChannelType, Cicp, ColorContext,
        plane::{AnyPlane, PlanarLayout, Plane, PlaneEncoding, PlaneRef, PlaneRole, PlaneSpec},
        sample::{ChromaSiting, ChromaSubsampling, SampleEncoding},
    };

    // Adapted from zenavif media mapping: native guards stay alive until loan drop.
    struct Native(rav1d_safe::Frame);
    enum Loan<'a> {
        Eight(Vec<rav1d_safe::PlaneView8<'a>>),
        Deep(Vec<rav1d_safe::PlaneView16<'a>>, SampleEncoding),
    }
    impl MappedPlanes for Loan<'_> {
        fn plane_count(&self) -> usize {
            match self {
                Self::Eight(v) => v.len(),
                Self::Deep(v, _) => v.len(),
            }
        }
        fn plane(&self, i: usize) -> Option<AnyPlane<'_>> {
            match self {
                Self::Eight(v) => {
                    let p = v.get(i)?;
                    Some(AnyPlane::U8(
                        PlaneRef::new(p.as_slice(), p.width(), p.height(), p.stride()).ok()?,
                    ))
                }
                Self::Deep(v, _) => {
                    let p = v.get(i)?;
                    Some(AnyPlane::U16(
                        PlaneRef::new(p.as_slice(), p.width(), p.height(), p.stride()).ok()?,
                    ))
                }
            }
        }
        fn encoding(&self, i: usize) -> Option<PlaneEncoding> {
            if i >= self.plane_count() {
                return None;
            }
            Some(PlaneEncoding::Code(match self {
                Self::Eight(_) => SampleEncoding::new(ChannelType::U8, 8, 0).unwrap(),
                Self::Deep(_, e) => *e,
            }))
        }
    }
    impl Picture for Native {
        fn map(&self) -> Result<Box<dyn MappedPlanes + '_>, MapError> {
            Ok(match self.0.planes() {
                rav1d_safe::Planes::Depth8(p) => Box::new(Loan::Eight(
                    [Some(p.y()), p.u(), p.v()].into_iter().flatten().collect(),
                )),
                rav1d_safe::Planes::Depth16(p) => Box::new(Loan::Deep(
                    [Some(p.y()), p.u(), p.v()].into_iter().flatten().collect(),
                    SampleEncoding::new(ChannelType::U16, self.0.bit_depth(), 0).unwrap(),
                )),
            })
        }
    }
    fn interpretation(width: u32, height: u32, bits: u8) -> FrameInterpretation {
        let e = SampleEncoding::new(
            if bits == 8 {
                ChannelType::U8
            } else {
                ChannelType::U16
            },
            bits,
            0,
        )
        .unwrap();
        FrameInterpretation::new(
            FrameGeometry::full(width, height),
            PlanarLayout::new(
                vec![PlaneSpec::new(PlaneRole::Luma, PlaneEncoding::Code(e))],
                ChromaSubsampling::Mono,
                ChromaSiting::Unknown,
            )
            .unwrap(),
        )
    }
    fn active(frame: &Frame) -> Vec<u8> {
        frame
            .with_planar(None, |view| {
                let mut data = Vec::new();
                match view.planes()[0] {
                    AnyPlane::U8(p) => p.rows().for_each(|r| data.extend_from_slice(r)),
                    AnyPlane::U16(p) => p.rows().for_each(|r| {
                        r.iter()
                            .for_each(|v| data.extend_from_slice(&v.to_le_bytes()))
                    }),
                    _ => panic!("unexpected native sample type"),
                }
                data
            })
            .unwrap()
    }
    #[test]
    fn native_av1_owner_survives_decoder_and_matches_independent_reader() {
        let root = std::path::PathBuf::from(
            std::env::var_os("FRAME_NATIVE_INPUTS")
                .expect("run prepare.py and set FRAME_NATIVE_INPUTS"),
        );
        for (name, bits) in [
            ("mono_5x3_8b_full.avif", 8),
            ("mono_gradient_10b_full.avif", 10),
        ] {
            let input = std::fs::read(root.join(name)).unwrap();
            let parser = zenavif_parse::AvifParser::from_bytes(&input).unwrap();
            let obu = parser.primary_data().unwrap();
            let mut decoder = rav1d_safe::Decoder::new().unwrap();
            let mut pictures = Vec::new();
            if let Some(p) = decoder.decode(&obu).unwrap() {
                pictures.push(p);
            }
            pictures.extend(decoder.flush().unwrap());
            assert_eq!(pictures.len(), 1);
            let native = pictures.pop().unwrap();
            assert_eq!(native.bit_depth(), bits);
            let c = native.raw_color_info();
            let cicp = Cicp::new(
                c.primaries,
                c.transfer_characteristics,
                c.matrix_coefficients,
                c.full_range,
            );
            let signal = interpretation(native.width(), native.height(), bits)
                .with_color(ColorClaim::new(Some(cicp), None, Some(cicp)))
                .with_color_context(Arc::new(ColorContext::from_cicp(cicp)));
            let picture = Arc::new(Native(native));
            assert_eq!(Picture::map(picture.as_ref()).unwrap().plane_count(), 1); // Typed native map.
            let frame = Frame::new(
                picture,
                Arc::new(signal),
                FrameTiming::default(),
                Presentation::default(),
            );
            drop(decoder);
            check_frame_interchange(&frame).unwrap(); // Erased owner, image map, canonical loan.
            assert_eq!(
                active(&frame),
                std::fs::read(root.join(format!("{name}.raw"))).unwrap(),
                "{name}: native vs independent decoder"
            );
        }
    }
    #[test]
    fn native_tiff_pages_keep_deep_and_odd_extents_through_one_interchange() {
        let mut file = Cursor::new(Vec::new());
        let expected8: Vec<u8> = (0..15).collect();
        let expected16: Vec<u16> = (0..21).map(|v| v * 3071).collect();
        {
            let mut writer = tiff::encoder::TiffEncoder::new(&mut file).unwrap();
            writer
                .write_image::<tiff::encoder::colortype::Gray8>(5, 3, &expected8)
                .unwrap();
            writer
                .write_image::<tiff::encoder::colortype::Gray16>(7, 3, &expected16)
                .unwrap();
        }
        file.set_position(0);
        let mut reader = tiff::decoder::Decoder::open(file).unwrap();
        reader.next_image().unwrap();
        for (index, (w, h, bits)) in [(5, 3, 8), (7, 3, 16)].into_iter().enumerate() {
            assert_eq!(reader.dimensions().unwrap(), (w, h));
            let plane = match reader.read_image().unwrap() {
                tiff::decoder::DecodingSampleBuffer::U8(v) => {
                    assert_eq!(v, expected8);
                    OwnedPlane::U8(Plane::new(v, w as usize, h as usize, w as usize).unwrap())
                }
                tiff::decoder::DecodingSampleBuffer::U16(v) => {
                    assert_eq!(v, expected16);
                    OwnedPlane::U16(Plane::new(v, w as usize, h as usize, w as usize).unwrap())
                }
                _ => panic!("unexpected TIFF output"),
            };
            let signal = interpretation(w, h, bits);
            let e = signal.uniform_code_encoding().unwrap();
            let frame = Frame::new(
                Arc::new(OwnedPlanes::new(vec![plane], e).unwrap()),
                Arc::new(signal),
                FrameTiming::default(),
                Presentation::default(),
            );
            check_frame_interchange(&frame).unwrap();
            if index == 0 {
                assert!(reader.more_images());
                reader.next_image().unwrap();
            }
        }
        assert!(!reader.more_images());
    }
}
