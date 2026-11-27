//! Contract tests for `zencodec_render::media` (`VideoCapturePlan`, `VideoRenderPlan`)
//! and `Renderer::with_color_services`.

#![cfg(feature = "media")]

use std::sync::Arc;

use zencodec::frame::FrameTiming;
use zencodec::storage::StorageBudget;
use zencodec::time::{TimeBase, Timestamp};
use zencodec_media::adapt::{CaptureFrame, RenderFrame};
use zencodec_media::session::CodecConfig;
use zencodec_media::track::{CodecId, TrackInfo, VideoTrackInfo};
use zencodec_render::media::{VideoCapturePlan, VideoRenderPlan};
use zencodec_render::{ColorServices, Renderer};
use zenpixels::{
    BitPacking, ChromaSiting, Cicp, PixelBuf, PixelFormat, PlaneFormat, PlaneRole, SampleType,
};
use zenpixels_convert::{PlanarCaptureOptions, PlanarReconstructionOptions};

fn make_solid_rgb(w: u32, h: u32, rgb: [u8; 3]) -> PixelBuf {
    let mut data = Vec::with_capacity((w * h * 3) as usize);
    for _ in 0..(w * h) {
        data.extend_from_slice(&rgb);
    }
    PixelBuf::from_typed(data, w, h, PixelFormat::RGB8_SRGB).unwrap()
}

#[test]
fn video_capture_and_render_roundtrip_p010_and_420_with_budget_accounting() {
    let (w, h) = (8u32, 6u32);
    let tb = TimeBase::new(1, 30).unwrap();
    let cfg = CodecConfig::new(CodecId::Av1, tb).with_video(VideoTrackInfo::new(w, h));
    let track = TrackInfo::video(0, CodecId::Av1, tb, VideoTrackInfo::new(w, h))
        .build()
        .unwrap();

    // 10-bit P010 4:2:0 narrow BT.709 with Left siting.
    let p010_planes = [
        PlaneFormat::new(PlaneRole::Y, SampleType::U16).with_packing(BitPacking::P010),
        PlaneFormat::new(PlaneRole::CbCr, SampleType::U16)
            .with_packing(BitPacking::P010)
            .with_subsampling(2, 2),
    ];
    let budget = StorageBudget::new(64 * 1024, 16);
    let mut capture = VideoCapturePlan::new(
        p010_planes,
        ChromaSiting::Left,
        Cicp::new(1, 13, 1, false),
        budget.clone(),
    );
    capture
        .validate_input(&PixelFormat::RGB8_SRGB, &cfg)
        .unwrap();

    let src = make_solid_rgb(w, h, [180, 90, 60]);
    let expected_scratch = capture
        .required_scratch_bytes(&PixelFormat::RGB8_SRGB, &cfg)
        .unwrap();
    assert!(expected_scratch > 0);
    let frame = capture
        .capture(&src.as_view(), &cfg, FrameTiming::at(Timestamp::new(5, tb)))
        .unwrap();
    assert_eq!(capture.retained_scratch_bytes(), expected_scratch);
    assert!(
        budget.reserved_bytes() > expected_scratch,
        "both captured planes and retained scratch must be charged against StorageBudget"
    );
    assert_eq!(frame.interpretation().layout.siting(), ChromaSiting::Left);
    assert_eq!(frame.timing().pts, Some(Timestamp::new(5, tb)));
    // Capture must never invent a bitstream color claim before encode; container claim
    // mirrors `config.video.color` (None here) and resolved carries the true target signal.
    assert_eq!(frame.interpretation().color.bitstream, None);
    assert_eq!(frame.interpretation().color.container, None);
    assert_eq!(
        frame.interpretation().color.resolved,
        Some(Cicp::new(1, 13, 1, false))
    );

    let mut renderer = VideoRenderPlan::new(PixelFormat::RGBA8_SRGB);
    assert_eq!(
        renderer.canvas_descriptor(&track).unwrap(),
        PixelFormat::RGBA8_SRGB
    );
    let mut canvas = PixelBuf::new(w, h, PixelFormat::RGBA8_SRGB).unwrap();
    renderer.render(&frame, &mut canvas.as_view_mut()).unwrap();
    for y in 0..h {
        let row = canvas.as_view().row(y);
        for px in row.as_chunks::<4>().0 {
            assert!(
                px[0].abs_diff(180) <= 1
                    && px[1].abs_diff(90) <= 1
                    && px[2].abs_diff(60) <= 1
                    && px[3] == 255,
                "P010 capture+render drifted: {px:?}"
            );
        }
    }

    drop(frame);
    assert_eq!(
        budget.reserved_bytes(),
        expected_scratch,
        "dropping captured Frame must release plane bytes while leaving accounted scratch"
    );
    capture.clear_scratch();
    assert_eq!(
        budget.reserved_bytes(),
        0,
        "clearing capture scratch must release all remaining StorageBudget"
    );
}

#[test]
fn video_render_refuses_missing_color_claim_and_unknown_siting() {
    let (w, h) = (4u32, 4u32);
    let tb = TimeBase::new(1, 30).unwrap();
    let cfg = CodecConfig::new(CodecId::Vp9, tb).with_video(VideoTrackInfo::new(w, h));
    let budget = StorageBudget::new(64 * 1024, 16);

    // BoxAverage capture with Unknown siting leaves Unknown siting on frame.
    let yuv420 = [
        PlaneFormat::new(PlaneRole::Y, SampleType::U8),
        PlaneFormat::new(PlaneRole::Cb, SampleType::U8).with_subsampling(2, 2),
        PlaneFormat::new(PlaneRole::Cr, SampleType::U8).with_subsampling(2, 2),
    ];
    let mut cap = VideoCapturePlan::new(
        yuv420,
        ChromaSiting::Unknown,
        Cicp::new(1, 13, 1, false),
        budget,
    )
    .with_options(PlanarCaptureOptions::box_average());
    let src = make_solid_rgb(w, h, [100, 120, 140]);
    let frame = cap
        .capture(&src.as_view(), &cfg, FrameTiming::at(Timestamp::new(0, tb)))
        .unwrap();

    // Default bilinear VideoRenderPlan refuses Unknown siting on 4:2:0!
    let mut strict_renderer = VideoRenderPlan::new(PixelFormat::RGB8_SRGB);
    let mut canvas = PixelBuf::new(w, h, PixelFormat::RGB8_SRGB).unwrap();
    assert!(
        strict_renderer
            .render(&frame, &mut canvas.as_view_mut())
            .is_err()
    );

    // Explicit fallback succeeds!
    let mut fb_renderer = VideoRenderPlan::new(PixelFormat::RGB8_SRGB).with_reconstruction(
        PlanarReconstructionOptions::bilinear().with_unknown_siting_fallback(ChromaSiting::Center),
    );
    fb_renderer
        .render(&frame, &mut canvas.as_view_mut())
        .unwrap();

    // Frame with no resolved color claim MUST be refused (never guesses sRGB).
    let mut interp_no_color = (**frame.interpretation()).clone();
    interp_no_color.color = zencodec::frame::ColorClaim::default();
    let untagged = zencodec::frame::Frame::new(
        Arc::clone(frame.picture()),
        Arc::new(interp_no_color),
        frame.timing(),
        frame.presentation(),
    );
    assert!(
        fb_renderer
            .render(&untagged, &mut canvas.as_view_mut())
            .is_err()
    );
}

#[test]
fn renderer_accepts_color_services_strict_no_icc() {
    let storage = StorageBudget::new(1024, 8);
    let r = Renderer::new(storage).with_color_services(ColorServices::strict_no_icc());
    assert!(r.cms.is_none());
    assert!(r.color_services.is_frozen());
}

#[test]
fn cropped_420_center_chroma_halo_matches_full_frame_render_bit_exact() {
    use zencodec::frame::{
        ColorClaim, Frame, FrameGeometry, FrameInterpretation, Presentation, Rect,
    };
    use zencodec::plane::{
        PlanarLayout, PlaneBuf, PlaneEncoding, PlaneRole as MediaPlaneRole, PlaneSpec,
    };
    use zencodec::sample::{ChromaSubsampling, SampleEncoding};
    use zencodec_media::frame::{OwnedPlane, OwnedPlanes};

    // Nonuniform 10x10 4:2:0 frame (5x5 chroma) with steep gradients in BOTH Y, Cb, and Cr
    // so discarding the 1-sample chroma halo at crop origin (2, 2) or odd (1, 1) visibly alters
    // the edge pixels under ChromaSiting::Center bilinear interpolation!
    let (cw, ch) = (10u32, 10u32);
    let (uw, uh) = (5usize, 5usize);
    let mut y_data = Vec::with_capacity((cw * ch) as usize);
    for y in 0..ch {
        for x in 0..cw {
            y_data.push((32 + x * 17 + y * 13) as u8);
        }
    }
    let mut cb_data = Vec::with_capacity(uw * uh);
    let mut cr_data = Vec::with_capacity(uw * uh);
    for cy in 0..uh {
        for cx in 0..uw {
            cb_data.push((24 + cx * 45 + cy * 11) as u8);
            cr_data.push((230 - cx * 37 - cy * 19) as u8);
        }
    }

    let enc = SampleEncoding::new(SampleType::U8, 8, 0).unwrap();
    let planes: Arc<dyn zencodec_media::frame::Picture> = Arc::new(
        OwnedPlanes::new(
            vec![
                OwnedPlane::U8(
                    PlaneBuf::new(y_data, cw as usize, ch as usize, cw as usize).unwrap(),
                ),
                OwnedPlane::U8(PlaneBuf::new(cb_data, uw, uh, uw).unwrap()),
                OwnedPlane::U8(PlaneBuf::new(cr_data, uw, uh, uw).unwrap()),
            ],
            enc,
        )
        .unwrap(),
    );
    let layout = PlanarLayout::new(
        vec![
            PlaneSpec::new(MediaPlaneRole::Luma, PlaneEncoding::Code(enc)),
            PlaneSpec::new(MediaPlaneRole::Cb, PlaneEncoding::Code(enc)),
            PlaneSpec::new(MediaPlaneRole::Cr, PlaneEncoding::Code(enc)),
        ],
        ChromaSubsampling::Yuv420,
        ChromaSiting::Center,
    )
    .unwrap();
    let bt709 = Cicp::new(1, 13, 1, false);
    let tb = TimeBase::new(1, 30).unwrap();

    let full_interp = Arc::new(
        FrameInterpretation::new(FrameGeometry::full(cw, ch), layout.clone())
            .with_color(ColorClaim::new(None, None, Some(bt709))),
    );
    let full_frame = Frame::new(
        Arc::clone(&planes),
        full_interp,
        FrameTiming::at(Timestamp::new(0, tb)),
        Presentation::default(),
    );

    let p3_rgb8 = PixelFormat::new(
        zenpixels::SampleLayout::RGB8,
        zenpixels::ColorSpace::DISPLAY_P3,
    );

    // Test BOTH the direct fast path (RGB8_SRGB) AND the F32 color-managed path (DisplayP3 RGB8)!
    for dst_fmt in [PixelFormat::RGB8_SRGB, p3_rgb8] {
        let mut full_canvas = PixelBuf::new(cw, ch, dst_fmt.clone()).unwrap();
        let mut renderer = VideoRenderPlan::new(dst_fmt.clone());
        renderer
            .render(&full_frame, &mut full_canvas.as_view_mut())
            .unwrap();

        for crop in [
            Rect {
                x: 2,
                y: 2,
                width: 4,
                height: 4,
            },
            Rect {
                x: 1,
                y: 1,
                width: 5,
                height: 5,
            },
            Rect {
                x: 4,
                y: 3,
                width: 6,
                height: 5,
            },
        ] {
            let mut geom = FrameGeometry::full(cw, ch);
            geom.crop = crop;
            let cropped_interp = Arc::new(
                FrameInterpretation::new(geom, layout.clone()).with_color(ColorClaim::new(
                    None,
                    None,
                    Some(bt709),
                )),
            );
            let cropped_frame = Frame::new(
                Arc::clone(&planes),
                cropped_interp,
                FrameTiming::at(Timestamp::new(0, tb)),
                Presentation::default(),
            );
            let mut crop_canvas = PixelBuf::new(crop.width, crop.height, dst_fmt.clone()).unwrap();
            renderer
                .render(&cropped_frame, &mut crop_canvas.as_view_mut())
                .unwrap();

            let full_view = full_canvas.as_view();
            let crop_view = crop_canvas.as_view();
            let bpp = dst_fmt.layout.bytes_per_pixel();
            for y in 0..crop.height {
                let expected = &full_view.row(crop.y + y)
                    [(crop.x as usize) * bpp..((crop.x + crop.width) as usize) * bpp];
                let actual = &crop_view.row(y)[..(crop.width as usize) * bpp];
                assert_eq!(
                    actual, expected,
                    "cropped render diverged from full-frame+crop for {dst_fmt:?} crop={crop:?} row={y}"
                );
            }
        }
    }

    // Also test 10-bit YUV -> 8-bit RGB with OrderedDither across non-mod-4 crop origins
    // ((1, 1), (2, 2), (3, 3)) so BOTH chroma halo AND 4×4 Bayer dither phase match
    // full-frame reconstruction + crop with 0-bit exactness!
    let enc10 = SampleEncoding::new(SampleType::U16, 10, 0).unwrap();
    let mut y10 = Vec::with_capacity((cw * ch) as usize);
    for y in 0..ch {
        for x in 0..cw {
            y10.push((120 + x * 67 + y * 53) as u16);
        }
    }
    let mut cb10 = Vec::with_capacity(uw * uh);
    let mut cr10 = Vec::with_capacity(uw * uh);
    for cy in 0..uh {
        for cx in 0..uw {
            cb10.push((180 + cx * 140 + cy * 37) as u16);
            cr10.push((840 - cx * 125 - cy * 61) as u16);
        }
    }
    let planes10: Arc<dyn zencodec_media::frame::Picture> = Arc::new(
        OwnedPlanes::new(
            vec![
                OwnedPlane::U16(PlaneBuf::new(y10, cw as usize, ch as usize, cw as usize).unwrap()),
                OwnedPlane::U16(PlaneBuf::new(cb10, uw, uh, uw).unwrap()),
                OwnedPlane::U16(PlaneBuf::new(cr10, uw, uh, uw).unwrap()),
            ],
            enc10,
        )
        .unwrap(),
    );
    let layout10 = PlanarLayout::new(
        vec![
            PlaneSpec::new(MediaPlaneRole::Luma, PlaneEncoding::Code(enc10)),
            PlaneSpec::new(MediaPlaneRole::Cb, PlaneEncoding::Code(enc10)),
            PlaneSpec::new(MediaPlaneRole::Cr, PlaneEncoding::Code(enc10)),
        ],
        ChromaSubsampling::Yuv420,
        ChromaSiting::Center,
    )
    .unwrap();
    let full10 = Frame::new(
        Arc::clone(&planes10),
        Arc::new(
            FrameInterpretation::new(FrameGeometry::full(cw, ch), layout10.clone())
                .with_color(ColorClaim::new(None, None, Some(bt709))),
        ),
        FrameTiming::at(Timestamp::new(0, tb)),
        Presentation::default(),
    );
    let mut dither_renderer = VideoRenderPlan::new(PixelFormat::RGB8_SRGB).with_conversion(
        zenpixels_convert::ConvertPolicy::permissive()
            .with_depth(zenpixels_convert::DepthPolicy::OrderedDither),
    );
    let mut full_dither = PixelBuf::new(cw, ch, PixelFormat::RGB8_SRGB).unwrap();
    dither_renderer
        .render(&full10, &mut full_dither.as_view_mut())
        .unwrap();
    let full_dither_view = full_dither.as_view();
    for crop in [
        Rect {
            x: 1,
            y: 1,
            width: 5,
            height: 5,
        },
        Rect {
            x: 2,
            y: 2,
            width: 4,
            height: 4,
        },
        Rect {
            x: 3,
            y: 3,
            width: 5,
            height: 5,
        },
    ] {
        let mut geom = FrameGeometry::full(cw, ch);
        geom.crop = crop;
        let cropped10 = Frame::new(
            Arc::clone(&planes10),
            Arc::new(
                FrameInterpretation::new(geom, layout10.clone()).with_color(ColorClaim::new(
                    None,
                    None,
                    Some(bt709),
                )),
            ),
            FrameTiming::at(Timestamp::new(0, tb)),
            Presentation::default(),
        );
        let mut crop_dither =
            PixelBuf::new(crop.width, crop.height, PixelFormat::RGB8_SRGB).unwrap();
        dither_renderer
            .render(&cropped10, &mut crop_dither.as_view_mut())
            .unwrap();
        let crop_view = crop_dither.as_view();
        for y in 0..crop.height {
            let expected = &full_dither_view.row(crop.y + y)
                [(crop.x as usize) * 3..((crop.x + crop.width) as usize) * 3];
            let actual = &crop_view.row(y)[..(crop.width as usize) * 3];
            assert_eq!(
                actual, expected,
                "dithered 10->8 cropped render diverged from full-frame+crop for crop={crop:?} row={y}"
            );
        }
    }
}

#[test]
fn render_enforces_conversion_policy_and_storage_budget_and_capture_invalidates_and_checks_claims()
{
    use zencodec_render::media::MediaBridgeError;
    use zenpixels_convert::{ConvertError, ConvertPolicy, DepthPolicy};

    let (w, h) = (8u32, 6u32);
    let tb = TimeBase::new(1, 30).unwrap();
    let bt709 = Cicp::new(1, 13, 1, false);
    let bt2020 = Cicp::new(9, 16, 9, false);
    let p010_planes = [
        PlaneFormat::new(PlaneRole::Y, SampleType::U16).with_packing(BitPacking::P010),
        PlaneFormat::new(PlaneRole::CbCr, SampleType::U16)
            .with_packing(BitPacking::P010)
            .with_subsampling(2, 2),
    ];

    // 1. Conflicting `config.video.color` is refused in BOTH validate_input and capture.
    let conflict_cfg =
        CodecConfig::new(CodecId::Av1, tb).with_video(VideoTrackInfo::new(w, h).with_color(bt2020));
    let mut cap = VideoCapturePlan::new(
        p010_planes,
        ChromaSiting::Left,
        bt709,
        StorageBudget::new(64 * 1024, 16),
    );
    let err = cap
        .validate_input(&PixelFormat::RGB8_SRGB, &conflict_cfg)
        .unwrap_err();
    assert!(matches!(
        err.downcast_ref::<MediaBridgeError>(),
        Some(MediaBridgeError::ColorClaimConflict { target, config })
            if *target == bt709 && *config == bt2020
    ));
    let src = make_solid_rgb(w, h, [120, 150, 180]);
    let err = cap
        .capture(
            &src.as_view(),
            &conflict_cfg,
            FrameTiming::at(Timestamp::new(0, tb)),
        )
        .unwrap_err();
    assert!(matches!(
        err.downcast_ref::<MediaBridgeError>(),
        Some(MediaBridgeError::ColorClaimConflict { .. })
    ));

    // 2. Matching `config.video.color` is preserved on `container` while `bitstream` stays None.
    let match_cfg =
        CodecConfig::new(CodecId::Av1, tb).with_video(VideoTrackInfo::new(w, h).with_color(bt709));
    let frame = cap
        .capture(
            &src.as_view(),
            &match_cfg,
            FrameTiming::at(Timestamp::new(0, tb)),
        )
        .unwrap();
    assert_eq!(frame.interpretation().color.bitstream, None);
    assert_eq!(frame.interpretation().color.container, Some(bt709));
    assert_eq!(frame.interpretation().color.resolved, Some(bt709));

    // 3. Changing `VideoCapturePlan` settings via builder invalidates cached plan immediately!
    let mut stricter_cap = cap.with_conversion(ConvertPolicy::lossless());
    let err = stricter_cap
        .capture(
            &src.as_view(),
            &match_cfg,
            FrameTiming::at(Timestamp::new(1, tb)),
        )
        .unwrap_err();
    // Full-range RGB8_SRGB -> Narrow-range BT.709 must be rejected under lossless policy!
    assert!(matches!(
        err.downcast_ref::<MediaBridgeError>(),
        Some(MediaBridgeError::Convert(
            ConvertError::RangeConversionForbidden
        ))
    ));

    // 4. Direct render fast path MUST NOT bypass ConvertPolicy::depth = DepthPolicy::Forbid
    //    on 10-bit P010 -> 8-bit RGB8_SRGB!
    let mut strict_depth_renderer = VideoRenderPlan::new(PixelFormat::RGB8_SRGB)
        .with_conversion(ConvertPolicy::permissive().with_depth(DepthPolicy::Forbid));
    let mut canvas8 = PixelBuf::new(w, h, PixelFormat::RGB8_SRGB).unwrap();
    let err = strict_depth_renderer
        .render(&frame, &mut canvas8.as_view_mut())
        .unwrap_err();
    assert!(matches!(
        err.downcast_ref::<MediaBridgeError>(),
        Some(MediaBridgeError::Convert(
            ConvertError::DepthReductionForbidden
        ))
    ));

    // 5. Render F32 fallback obeys `VideoRenderPlan::with_storage` and releases charge on exit!
    let p3_rgb8 = PixelFormat::new(
        zenpixels::SampleLayout::RGB8,
        zenpixels::ColorSpace::DISPLAY_P3,
    );
    let tiny_budget = StorageBudget::new(16, 4);
    let mut tiny_renderer = VideoRenderPlan::new(p3_rgb8.clone()).with_storage(tiny_budget);
    let mut p3_canvas = PixelBuf::new(w, h, p3_rgb8.clone()).unwrap();
    assert!(
        tiny_renderer
            .render(&frame, &mut p3_canvas.as_view_mut())
            .is_err(),
        "tiny StorageBudget must refuse oversized F32 render intermediate"
    );

    let ample_budget = StorageBudget::new(64 * 1024, 8);
    let mut budgeted_renderer = VideoRenderPlan::new(p3_rgb8).with_storage(ample_budget.clone());
    budgeted_renderer
        .render(&frame, &mut p3_canvas.as_view_mut())
        .unwrap();
    assert_eq!(
        ample_budget.reserved_bytes(),
        0,
        "F32 intermediate reservation must be released when render returns"
    );
}
