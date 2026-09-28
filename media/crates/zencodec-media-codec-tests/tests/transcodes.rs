use zencodec_media::animation::{AnimationLimits, TimingPolicy, transcode_dyn};
use zencodec_media_codec_tests::{Format, fixtures};

#[test]
fn every_format_pair_preserves_complete_display_canvases_timing_and_plays() {
    for fixture in fixtures() {
        for source in Format::ALL {
            let bytes = fixture
                .encode(source)
                .unwrap_or_else(|e| panic!("{} {source:?} encode: {e}", fixture.name));
            for target in Format::ALL {
                let mut decoder = source.decoder(bytes.clone()).unwrap();
                let (output, report) = transcode_dyn(
                    decoder.as_mut(),
                    |_, plays| target.encoder(plays),
                    TimingPolicy::Exact,
                    AnimationLimits::new(10, 100_000_000).unwrap(),
                    None,
                )
                .unwrap_or_else(|e| panic!("{} {source:?} → {target:?}: {e}", fixture.name));
                assert_eq!(report.frames(), fixture.frames.len() as u64);
                assert_eq!(report.changed_durations(), 0);
                assert_eq!(report.source_plays(), Some(fixture.plays));
                let mut decoded = target.decoder(output.into_vec()).unwrap();
                assert_eq!(
                    decoded.loop_count(),
                    Some(fixture.plays),
                    "{} {source:?} → {target:?}",
                    fixture.name
                );
                for (index, (expected, &duration)) in
                    fixture.frames.iter().zip(&fixture.durations).enumerate()
                {
                    let frame = decoded
                        .render_next_frame_owned(None)
                        .unwrap_or_else(|e| {
                            panic!(
                                "{} {source:?} → {target:?} frame {index}: {e}",
                                fixture.name
                            )
                        })
                        .unwrap();
                    assert_eq!(frame.frame_index(), index as u32);
                    assert_eq!(
                        frame.duration(),
                        duration,
                        "{} {source:?} → {target:?} frame {index}",
                        fixture.name
                    );
                    let actual = frame.pixels();
                    assert_eq!(
                        (actual.width(), actual.rows()),
                        (expected.width(), expected.height())
                    );
                    assert_eq!(
                        actual.descriptor().channel_type(),
                        zenpixels::ChannelType::U8
                    );
                    for y in 0..actual.rows() {
                        assert_eq!(
                            actual.row(y),
                            expected.as_slice().row(y),
                            "{} {source:?} → {target:?} frame {index} row {y}",
                            fixture.name
                        );
                    }
                }
                assert!(decoded.render_next_frame_owned(None).unwrap().is_none());
            }
        }
    }
}
