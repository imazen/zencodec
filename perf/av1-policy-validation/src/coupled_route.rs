//! Executing fixed-output witness: retained YCbCr plan + native AV1 encoder.
use crate::{epoch_route::Candidate, fixtures::*};
use zencodec::how_far::NoPulse;
use std::{
    cell::Cell,
    error::Error,
    path::PathBuf,
    sync::{Arc, atomic::AtomicUsize},
};
use zenavif::media::Av1VideoDecoder;
use zencodec::{
    At,
    encode::Fidelity,
    frame::{
        ChromaSiting, ColorClaim, Frame, FrameGeometry, FrameInterpretation, PlaneRole,
        PlaneSamples, SampleEncoding, Subsampling,
    },
    operation::{
        Decision, DecisionCode, FrameConversionOffer, FrameFacts, PolicyFailure, PolicyResolver,
        PolicySet, Question, Selection,
    },
    storage::StorageBudget,
};
use zencodec_media::frame::{OwnedPlane, OwnedPlanes, PlaneData};
use zenpixels::{
    ChannelType, Cicp, PixelSliceMut,
    sample::{ChromaSiting as PixelSiting, ChromaSubsampling, CodePlane},
};
use zenpixels_convert::ycbcr::YcbcrToRgbF32;
type Cause = Box<dyn Error + Send + Sync>;

pub(crate) trait ConversionRoute {
    fn push_conversion(
        &mut self,
        input: &Frame,
        target: &FrameInterpretation,
        offer: FrameConversionOffer,
        execute: impl FnOnce() -> Result<Frame, Cause>,
    ) -> Result<(), Cause>;
}
impl<R: PolicyResolver> ConversionRoute for Candidate<R> {
    fn push_conversion(
        &mut self,
        input: &Frame,
        target: &FrameInterpretation,
        offer: FrameConversionOffer,
        execute: impl FnOnce() -> Result<Frame, Cause>,
    ) -> Result<(), Cause> {
        self.push_route(
            input,
            (target, input.timing()),
            Some(offer),
            execute,
            &NoPulse,
        )
    }
}
impl<
    E: zencodec_media::session::VideoEncoder,
    M: zencodec_media::container::Muxer,
    D: zencodec_media::output::Destination,
    R: PolicyResolver,
> ConversionRoute for zencodec_media::staged::StagedEncode<E, M, D, R>
{
    fn push_conversion(
        &mut self,
        input: &Frame,
        target: &FrameInterpretation,
        offer: FrameConversionOffer,
        execute: impl FnOnce() -> Result<Frame, Cause>,
    ) -> Result<(), Cause> {
        self.push_converted(input.clone(), target, offer, |_| execute(), &NoPulse)
            .map_err(Into::into)
    }
}

pub(crate) fn push(
    operation: &mut impl ConversionRoute,
    input: &Frame,
    pool: &StorageBudget,
    executions: &Cell<usize>,
    expected: &mut Vec<Frame>,
) -> Result<(), Cause> {
    let info = input.interpretation();
    if info.geometry != FrameGeometry::full(info.geometry.coded_width, info.geometry.coded_height)
        || info.roles != [PlaneRole::Luma, PlaneRole::Cb, PlaneRole::Cr]
        || !matches!(info.siting, ChromaSiting::Unknown | ChromaSiting::Center)
    {
        return Err(std::io::Error::other("unqualified conversion interpretation").into());
    }
    let mapping = input.map()?;
    let encoding = zenpixels::sample::SampleEncoding::new(
        info.encoding.storage(),
        info.encoding.code_bits(),
        info.encoding.bit_shift(),
    )?;
    let view = |i| -> Result<CodePlane<'_>, Cause> {
        let p = mapping
            .plane(i)
            .ok_or_else(|| std::io::Error::other("missing plane"))?;
        Ok(match p.as_strided_samples() {
            PlaneSamples::U8(s) => {
                CodePlane::from_u8(s, p.width(), p.height(), p.stride_bytes(), encoding)?
            }
            PlaneSamples::U16(s) => {
                CodePlane::from_u16(s, p.width(), p.height(), p.stride_bytes(), encoding)?
            }
        })
    };
    let sub = match info.subsampling {
        Subsampling::Yuv444 => ChromaSubsampling::Yuv444,
        Subsampling::Yuv422 => ChromaSubsampling::Yuv422,
        Subsampling::Yuv420 => ChromaSubsampling::Yuv420,
        _ => return Err(std::io::Error::other("unsupported sampling").into()),
    };
    let plan = YcbcrToRgbF32::new(
        [view(0)?, view(1)?, view(2)?],
        sub,
        PixelSiting::Center,
        info.color
            .resolved
            .ok_or_else(|| std::io::Error::other("unknown color"))?,
    )?;
    let (w, h) = (
        info.geometry.coded_width as usize,
        info.geometry.coded_height as usize,
    );
    let stride = w
        .checked_add(3)
        .ok_or_else(|| std::io::Error::other("stride overflow"))?;
    let count = stride
        .checked_mul(h)
        .ok_or_else(|| std::io::Error::other("extent overflow"))?;
    let planes = [
        pool.prepare_filled(count, 0u8)?,
        pool.prepare_filled(count, 0u8)?,
        pool.prepare_filled(count, 0u8)?,
    ];
    let row = pool.prepare_filled(w, [0f32; 3])?;
    let output_encoding = SampleEncoding::new(ChannelType::U8, 8, 0).unwrap();
    let mut target = FrameInterpretation::new(
        info.geometry,
        Subsampling::Yuv444,
        ChromaSiting::Unknown,
        output_encoding,
        vec![PlaneRole::Green, PlaneRole::Blue, PlaneRole::Red],
    );
    target.color = ColorClaim::new(None, None, Some(Cicp::new(1, 13, 0, true)));
    let mut offer = FrameConversionOffer::new(FrameFacts::new(&target, input.timing()));
    offer.assumed_chroma_siting =
        (info.siting == ChromaSiting::Unknown).then_some(ChromaSiting::Center);
    offer.admitted_bytes =
        planes.iter().map(|p| p.reserved_bytes()).sum::<usize>() + row.reserved_bytes();
    // Float conversion followed by explicit round-to-nearest U8 saturation.
    // Lossless AV1 preserves these converted codes, not the source samples.
    let output_target = Arc::new(target.clone());
    operation.push_conversion(input, &target, offer, move || {
        executions.set(executions.get() + 1);
        let [g, b, r] = planes;
        let mut planes = [g.allocate()?, b.allocate()?, r.allocate()?];
        let mut row = row.allocate()?;
        for y in 0..h {
            let dst = PixelSliceMut::new(
                bytemuck::cast_slice_mut(row.as_mut_slice()),
                w as u32,
                1,
                w * 12,
                plan.output_descriptor(),
            )?;
            plan.convert_rows(0, y, dst)?;
            for (x, rgb) in row.as_slice().iter().enumerate() {
                for (c, channel) in [1, 2, 0].into_iter().enumerate() {
                    if !rgb[channel].is_finite() {
                        return Err(std::io::Error::other("nonfinite conversion").into());
                    }
                    planes[c].as_mut_slice()[y * stride + x] =
                        (rgb[channel].clamp(0.0, 1.0) * 255.0).round() as u8;
                }
            }
        }
        let planes = planes
            .into_iter()
            .map(|p| OwnedPlane {
                data: PlaneData::AccountedU8(p.freeze()),
                width: w,
                height: h,
                stride_bytes: stride,
            })
            .collect();
        let converted = Frame::new(
            Arc::new(OwnedPlanes::new(planes, output_encoding)?),
            output_target,
            input.timing(),
            input.presentation(),
        );
        expected.push(converted.clone());
        Ok(converted)
    })
}

pub(crate) fn sources() -> Vec<Frame> {
    let mut result = Vec::new();
    for (i, (bits, sub, full)) in [
        (8, Subsampling::Yuv420, true),
        (10, Subsampling::Yuv422, false),
        (12, Subsampling::Yuv444, true),
    ]
    .into_iter()
    .enumerate()
    {
        let mut s = settings(bits, sub, false);
        s.color = Cicp::new(1, 13, 1, full);
        let source = frame(&s, i);
        let (config, packets) = encode(s, &[source]);
        let mut decoded = decode(
            &mut Av1VideoDecoder::new().unwrap(),
            &config,
            &packets,
            &NoPulse,
        )
        .unwrap();
        result.append(&mut decoded);
    }
    result
}

#[test]
fn coupled_native_retiming_keeps_source_provenance_and_fixed_output_clock() {
    use zencodec::{
        frame::FrameTiming,
        time::{TimeBase, Timestamp},
    };
    let settings = settings(8, Subsampling::Yuv420, false);
    let originals: Vec<_> = (0..3).map(|i| frame(&settings, i)).collect();
    let (config, packets) = encode(settings, &originals);
    let inputs = decode(
        &mut Av1VideoDecoder::new().unwrap(),
        &config,
        &packets,
        &NoPulse,
    )
    .unwrap();
    let clock = TimeBase::hz(90_000).unwrap();
    // Explicit variable-rate output, neither a clock-only relabel nor inferred FPS.
    let timings = [(9000, 1800), (10800, 4500), (15300, 2700)].map(|(pts, duration)| {
        FrameTiming::at(Timestamp::new(pts, clock)).with_duration_ticks(duration)
    });
    let calls = Cell::new(0usize);
    let executions = Cell::new(0usize);
    let storage = StorageBudget::new(1 << 20, 64);
    let live = Arc::new(AtomicUsize::new(0));
    let mut operation = Candidate::new(
        |q: Question<'_>| -> Result<Decision, At<std::convert::Infallible>> {
            let Question::FrameRoute { facts, choices, .. } = q else {
                panic!()
            };
            let i = calls.get();
            assert_eq!(executions.get(), i);
            assert_eq!(facts.input.timing, inputs[i].timing());
            assert_eq!(
                facts.output.previous.map(|p| p.timing),
                i.checked_sub(1).map(|j| inputs[j].timing())
            );
            assert_eq!(facts.output.output_committed, i > 0);
            let choice = choices.first().unwrap();
            let conversion = choice.value().conversion.unwrap();
            assert_eq!(conversion.encoder_input.timing, timings[i]);
            assert!(conversion.preserves_samples);
            calls.set(i + 1);
            Ok(Decision::choose(choice))
        },
        false,
        false,
        storage.clone(),
        live.clone(),
    );
    let mut expected = Vec::new();
    for (i, input) in inputs.iter().enumerate() {
        let mut offer =
            FrameConversionOffer::new(FrameFacts::new(input.interpretation(), timings[i]));
        offer.preserves_samples = true;
        offer.preserves_extended_range = true;
        operation
            .push_route(
                input,
                (input.interpretation(), timings[i]),
                Some(offer),
                || {
                    executions.set(executions.get() + 1);
                    let retimed = input.represent(timings[i], input.presentation());
                    assert!(retimed.shares_picture(input));
                    expected.push(retimed.clone());
                    Ok(retimed)
                },
                &NoPulse,
            )
            .unwrap();
        if i == 0 {
            operation
                .publish_header(
                    |config| {
                        assert_eq!(config.time_base, clock);
                        Ok(())
                    },
                    &NoPulse,
                )
                .unwrap();
        }
    }
    let segments = operation.finish(&NoPulse).unwrap();
    assert_eq!(segments.len(), 1);
    assert_eq!(calls.get(), inputs.len());
    assert_eq!(executions.get(), inputs.len());
    let segment = &segments[0];
    assert_eq!(segment.inputs, timings);
    assert_eq!(segment.packets.len(), timings.len());
    for (packet, timing) in segment.packets.iter().zip(timings) {
        assert_eq!(packet.pts(), timing.pts);
        assert_eq!(packet.duration_ticks(), timing.duration_ticks);
    }
    let root = PathBuf::from(std::env::var_os("AV1_POLICY_ARTIFACTS").unwrap());
    std::fs::create_dir_all(&root).unwrap();
    oracle_on_clock(
        &root,
        "coupled-retimed",
        &segment.settings,
        &segment.packets,
        &expected,
        clock,
    );
    drop(segments);
    assert_eq!(live.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert_eq!(storage.reserved_bytes(), 0);
}

#[test]
fn coupled_native_changes_convert_to_one_committed_encoder_with_disclosed_losses() {
    let inputs = sources();
    let pool = StorageBudget::new(1 << 20, 64);
    let encoded = StorageBudget::new(1 << 20, 64);
    let live = Arc::new(AtomicUsize::new(0));
    let executions = Cell::new(0);
    let calls = Cell::new(0);
    let retained = Cell::new(0);
    let mut operation = Candidate::new(
        |q: Question<'_>| -> Result<Decision, At<std::convert::Infallible>> {
            let Question::FrameRoute { facts, choices, .. } = q else {
                panic!()
            };
            assert_eq!(executions.get(), calls.get());
            assert_eq!(
                facts.input,
                FrameFacts::new(
                    inputs[calls.get()].interpretation(),
                    inputs[calls.get()].timing()
                )
            );
            let choice = choices.first().unwrap();
            let offer = choice.value();
            assert_eq!(offer.fidelity, Some(Fidelity::Lossless));
            let conversion = offer.conversion.unwrap();
            assert!(!conversion.preserves_samples);
            assert!(!conversion.preserves_extended_range);
            assert_eq!(
                conversion.encoder_input.color.resolved,
                Some(Cicp::new(1, 13, 0, true))
            );
            assert_eq!(
                conversion.admitted_bytes,
                pool.reserved_bytes() - retained.get()
            );
            assert_eq!(facts.output.output_committed, calls.get() > 0);
            calls.set(calls.get() + 1);
            Ok(Decision::choose(choice))
        },
        false,
        false,
        encoded.clone(),
        live.clone(),
    );
    let mut expected = Vec::new();
    let mut header = None;
    for (i, input) in inputs.iter().enumerate() {
        push(&mut operation, input, &pool, &executions, &mut expected).unwrap();
        retained.set(pool.reserved_bytes());
        // The witness owns expected outputs for independent decoder comparison;
        // exclude those previous owners from the next preparation accounting.
        if i == 0 {
            operation
                .publish_header(
                    |config| {
                        header = Some(config.clone());
                        Ok(())
                    },
                    &NoPulse,
                )
                .unwrap();
        }
    }
    let segments = operation.finish(&NoPulse).unwrap();
    assert_eq!(segments.len(), 1);
    assert_eq!(calls.get(), 3);
    assert_eq!(executions.get(), 3);
    assert_eq!(segments[0].config.video, header.unwrap().video);
    let root = PathBuf::from(std::env::var_os("AV1_POLICY_ARTIFACTS").unwrap());
    std::fs::create_dir_all(&root).unwrap();
    oracle(
        &root,
        "coupled-fixed-rgb",
        &segments[0].settings,
        &segments[0].packets,
        &expected,
    );
    drop(expected);
    drop(segments);
    assert_eq!(pool.reserved_bytes(), 0);
    assert_eq!(encoded.reserved_bytes(), 0);
}

#[test]
fn coupled_native_required_range_refuses_before_converter_execution() {
    let inputs = sources();
    let pool = StorageBudget::new(1 << 20, 64);
    let executions = Cell::new(0);
    let mut expected = Vec::new();
    let mut operation = Candidate::new(
        PolicySet::new()
            .allow_assumed_chroma_siting(true)
            .require_extended_range(true),
        false,
        false,
        StorageBudget::new(1 << 20, 64),
        Arc::new(AtomicUsize::new(0)),
    );
    let error = push(
        &mut operation,
        &inputs[0],
        &pool,
        &executions,
        &mut expected,
    )
    .unwrap_err();
    let failure = zencodec::find_cause::<PolicyFailure>(&*error).unwrap();
    assert_eq!(failure.record().code, DecisionCode::Declined);
    assert_eq!(executions.get(), 0);
    assert_eq!(pool.reserved_bytes(), 0);
    assert!(operation.is_failed());
    assert!(expected.is_empty());
    assert!(!matches!(
        failure.record().selected,
        Some(Selection::FrameRoute(_))
    ));
}

#[test]
fn coupled_native_exact_sample_callback_refuses_lossless_encoder_with_lossy_conversion() {
    let inputs = sources();
    let pool = StorageBudget::new(1 << 20, 64);
    let executions = Cell::new(0);
    let mut expected = Vec::new();
    let mut operation = Candidate::new(
        |q: Question<'_>| -> Result<Decision, At<std::convert::Infallible>> {
            let Question::FrameRoute { choices, .. } = q else {
                panic!()
            };
            assert_eq!(
                choices.first().unwrap().value().fidelity,
                Some(Fidelity::Lossless)
            );
            Ok(choices
                .iter()
                .find(|c| c.value().conversion.is_some_and(|c| c.preserves_samples))
                .map(Decision::choose)
                .unwrap_or_else(|| {
                    Decision::decline(zencodec::operation::Decline::NoPreferredChoice)
                }))
        },
        false,
        false,
        StorageBudget::new(1 << 20, 64),
        Arc::new(AtomicUsize::new(0)),
    );
    let error = push(
        &mut operation,
        &inputs[0],
        &pool,
        &executions,
        &mut expected,
    )
    .unwrap_err();
    assert!(zencodec::find_cause::<PolicyFailure>(&*error).is_some());
    assert_eq!(executions.get(), 0);
    assert_eq!(pool.reserved_bytes(), 0);
    assert!(operation.is_failed());
}

#[test]
fn coupled_native_conversion_failure_and_target_drift_are_terminal_after_selection() {
    let inputs = sources();
    for drift in [0, 1, 2] {
        let live = Arc::new(AtomicUsize::new(0));
        let storage = StorageBudget::new(1 << 20, 64);
        let mut operation = Candidate::new(
            PolicySet::new(),
            false,
            false,
            storage.clone(),
            live.clone(),
        );
        operation.push(&inputs[0], &NoPulse).unwrap();
        operation.publish_header(|_| Ok(()), &NoPulse).unwrap();
        let target = inputs[0].interpretation();
        let mut conversion = FrameConversionOffer::new(FrameFacts::new(target, inputs[1].timing()));
        conversion.preserves_samples = false;
        let error = operation
            .push_route(
                &inputs[1],
                (target, inputs[1].timing()),
                Some(conversion),
                || {
                    if drift == 2 {
                        // Correct target interpretation, wrong promised timestamp.
                        Ok(inputs[0].clone())
                    } else if drift == 1 {
                        Ok(inputs[1].clone())
                    } else {
                        Err(std::io::Error::other("conversion failed after policy").into())
                    }
                },
                &NoPulse,
            )
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            if drift != 0 {
                "prepared conversion target changed"
            } else {
                "conversion failed after policy"
            }
        );
        let Some(Selection::FrameRoute(selected)) =
            operation.report().decisions().last().unwrap().selected
        else {
            panic!()
        };
        assert_eq!(
            selected.conversion.unwrap().encoder_input,
            conversion.encoder_input
        );
        assert!(operation.is_failed());
        assert_eq!(live.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert_eq!(storage.reserved_bytes(), 0);
        assert!(operation.push(&inputs[0], &NoPulse).is_err());
    }
}
