//! Planning must describe the effective job, not its underlying native template.
use zencodec::encode::{EncodeJob, Encoder, EncoderConfig};
use zenjpeg::JpegEncoderConfig;
use zenpixels::PixelDescriptor;

#[test]
fn native_template_plan_does_not_describe_adapter_streaming_overrides() {
    for height in [8, 32, 64] {
        let original = JpegEncoderConfig::new();
        let preferred = original.clone().with_prefer_streaming(true);
        // The public inner plan is identical: the wrapper's policy has not yet
        // been folded into it. It cannot authorize a final output schedule.
        assert_eq!(
            format!("{:?}", original.inner().resolve_plan(64, height)),
            format!("{:?}", preferred.inner().resolve_plan(64, height)),
        );
        let mut ordinary = original.job().encoder().unwrap();
        let mut sequential = preferred.job().encoder().unwrap();
        let ordinary = ordinary
            .begin(64, height, PixelDescriptor::RGB8_SRGB)
            .unwrap();
        let sequential = sequential
            .begin(64, height, PixelDescriptor::RGB8_SRGB)
            .unwrap();
        assert!(!ordinary.streams());
        assert_eq!(sequential.streams(), height > 31);
    }
}

#[test]
fn explicit_native_and_progressive_settings_override_streaming_preference() {
    let explicit_progressive = JpegEncoderConfig::new()
        .with_progressive(true)
        .with_prefer_streaming(true);
    let mut explicit_native = JpegEncoderConfig::new();
    // Calling inner_mut makes the native template authoritative even if the
    // caller only inspects it. The output planner must retain this provenance.
    let _ = explicit_native.inner_mut();
    let explicit_native = explicit_native.with_prefer_streaming(true);
    for config in [explicit_progressive, explicit_native] {
        let mut encoder = config.job().encoder().unwrap();
        assert!(
            !encoder
                .begin(64, 64, PixelDescriptor::RGB8_SRGB)
                .unwrap()
                .streams()
        );
    }
}
