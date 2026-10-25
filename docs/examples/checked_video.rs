//! Executable video recipes, qualified by the standalone native AV1 harness.
#![forbid(unsafe_code)]

mod ordinary {
    // EXAMPLE: checked_video
    use zenavif::media::{Av1SessionError, Av1VideoDecoder};
    use zencodec::operation::PolicySet;
    use zencodec_media::policy::PolicyDecoder;

    pub fn open_video() -> Result<PolicyDecoder<Av1VideoDecoder, PolicySet>, Av1SessionError> {
        let native = Av1VideoDecoder::new()?;
        // ANCHOR: checked_video
        let decoder = PolicyDecoder::new(native, PolicySet::new());
        // END: checked_video
        // Configure and drive it through the existing VideoDecoder trait.
        // Every receive checks policy before returning a native Frame.
        Ok(decoder)
    }
    // END EXAMPLE: checked_video
}
pub(crate) use ordinary::open_video;

mod tenant {
    // EXAMPLE: checked_video_rule
    use zenavif::media::{Av1SessionError, Av1VideoDecoder};
    use zencodec::{
        At,
        operation::{Decision, PolicyResolver, Question},
    };
    use zencodec_media::policy::PolicyDecoder;

    #[derive(Debug)]
    pub struct PreviewTransfer;
    impl core::fmt::Display for PreviewTransfer {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.write_str("this preview route requires declared sRGB transfer")
        }
    }
    impl core::error::Error for PreviewTransfer {}

    pub fn open_preview() -> Result<
        PolicyDecoder<Av1VideoDecoder, impl PolicyResolver<Error = PreviewTransfer>>,
        Av1SessionError,
    > {
        let native = Av1VideoDecoder::new()?;
        // ANCHOR: checked_video_rule
        let policy = |question: Question<'_>| -> Result<Decision, At<PreviewTransfer>> {
            if let Question::Frame { interpretation, .. } = question {
                let supported = interpretation
                    .color
                    .resolved
                    .is_some_and(|color| color.transfer_characteristics == 13);
                if !supported {
                    return Err(At::wrap(PreviewTransfer).at_named("preview transfer requirement"));
                }
            }
            Ok(Decision::UsePreset)
        };
        let decoder = PolicyDecoder::new(native, policy);
        // END: checked_video_rule
        // This gate does not convert YUV/GBR, tone-map or prove a RAM bound.
        // The renderer must still validate its complete conversion route.
        Ok(decoder)
    }
    // END EXAMPLE: checked_video_rule
}
pub(crate) use tenant::{PreviewTransfer, open_preview};
