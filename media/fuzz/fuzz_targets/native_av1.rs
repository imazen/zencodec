#![no_main]
use libfuzzer_sys::fuzz_target;
use zencodec_media::av1::Av1IvfDecoder;

fuzz_target!(|data: &[u8]| {
    let mut settings = rav1d_safe::Settings::default();
    settings.frame_size_limit = 256 * 256;
    settings.all_layers = false;
    let index_settings = settings.clone();
    let Ok(mut decoder) = Av1IvfDecoder::new(data, 65536, settings) else {
        return;
    };
    let mut retained = None;
    for _ in 0..32 {
        match decoder.next_frame() {
            Ok(Some(frame)) => {
                let mapped = frame.map();
                for index in 0..3 {
                    if let Some(plane) = mapped.plane(index).unwrap() {
                        assert!(plane.width() > 0 && plane.height() > 0);
                        let maximum = (1_u32 << plane.encoding().code_bits()) - 1;
                        assert!(
                            u32::from(plane.code(plane.width() - 1, plane.height() - 1).unwrap())
                                <= maximum
                        );
                    }
                }
                drop(mapped);
                if retained.is_none() {
                    retained = Some(frame);
                }
            }
            Ok(None) => {
                assert!(decoder.next_frame().unwrap().is_none());
                break;
            }
            Err(_) => {
                assert!(decoder.next_frame().is_err());
                break;
            }
        }
    }
    drop(decoder);
    if let Ok(mut index) = zencodec_media::av1_index::IndexedAv1::build(
        std::io::Cursor::new(data),
        65536,
        32,
        4096,
        index_settings,
        None,
    ) {
        use zencodec_media::{
            av1_index::{FrameScope, FrameSelection},
            time::{TimeBase, Timestamp},
        };
        let request = data
            .get(..8)
            .map_or(0, |b| i64::from_le_bytes(b.try_into().unwrap()));
        for selection in [
            FrameSelection::Nearest,
            FrameSelection::AtOrBefore,
            FrameSelection::AtOrAfter,
        ] {
            let result = index
                .extract(
                    Timestamp::new(request, TimeBase::new(1, 90000).unwrap()),
                    selection,
                    FrameScope::All,
                )
                .expect("a successfully indexed unchanged stream must extract");
            if let Some(frame) = result {
                frame
                    .frame()
                    .map()
                    .plane(0)
                    .unwrap()
                    .unwrap()
                    .code(0, 0)
                    .unwrap();
            }
        }
    }
    if let Some(frame) = retained {
        let mapping = frame.map();
        mapping.plane(0).unwrap().unwrap().code(0, 0).unwrap();
    }
});
