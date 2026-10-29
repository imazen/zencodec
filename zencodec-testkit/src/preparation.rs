//! Preparation is optional; claiming facts or retaining state must stay honest.
use super::*;

const CHECK: &str = "decode_preparation";

pub(super) fn check<D: DecoderConfig>(
    cfg: &D,
    bytes: &[u8],
    baseline: &DecodeOutput,
) -> Conformance {
    let decoder = cfg
        .clone()
        .job()
        .decoder(Cow::Borrowed(bytes), &[])
        .map_err(|e| fail(CHECK, format!("typed creation: {e}")))?;
    typed(decoder, baseline)?;
    let mut decoder = cfg
        .clone()
        .job()
        .dyn_decoder(Cow::Borrowed(bytes), &[])
        .map_err(|e| fail(CHECK, format!("erased creation: {e}")))?;
    let facts = decoder
        .prepare_output()
        .map_err(|e| fail(CHECK, format!("erased preparation: {e}")))?;
    let output = decoder
        .decode()
        .map_err(|e| fail(CHECK, format!("erased execution: {e}")))?;
    verify("erased", facts, output, baseline)
}

fn typed<D: Decode>(mut decoder: D, baseline: &DecodeOutput) -> Conformance {
    let facts = decoder
        .prepare_output()
        .map_err(|e| fail(CHECK, format!("typed preparation: {e}")))?;
    let output = decoder
        .decode()
        .map_err(|e| fail(CHECK, format!("typed execution: {e}")))?;
    verify("typed", facts, output, baseline)
}

fn verify(
    path: &str,
    facts: Option<PixelDescriptor>,
    output: DecodeOutput,
    baseline: &DecodeOutput,
) -> Conformance {
    if facts.is_some_and(|descriptor| descriptor != output.pixels().descriptor()) {
        return Err(fail(
            CHECK,
            format!("{path} preparation descriptor contradicts actual output"),
        ));
    }
    if grab(output.pixels()) != grab(baseline.pixels()) {
        return Err(fail(
            CHECK,
            format!("{path} preparation changed pixels or descriptor"),
        ));
    }
    if output.metadata() != baseline.metadata() {
        return Err(fail(CHECK, format!("{path} preparation changed metadata")));
    }
    if !same_ctx(
        output.pixels().color_context(),
        baseline.pixels().color_context(),
    ) {
        return Err(fail(
            CHECK,
            format!("{path} preparation changed color context"),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use zencodec::ImageInfo;
    use zenpixels::PixelBuffer;

    struct Mutant {
        mode: u8,
        prepared: bool,
    }
    impl Decode for Mutant {
        type Error = core::fmt::Error;
        fn prepare_output(&mut self) -> Result<Option<PixelDescriptor>, Self::Error> {
            self.prepared = true;
            match self.mode {
                0 | 3 | 4 | 6 => Ok(None),
                1 => Ok(Some(PixelDescriptor::RGB8_SRGB)),
                2 => Ok(Some(PixelDescriptor::RGBA8_SRGB)),
                _ => Err(core::fmt::Error),
            }
        }
        fn decode(self) -> Result<DecodeOutput, Self::Error> {
            let bytes = if self.prepared && self.mode == 3 {
                vec![9, 2, 3]
            } else {
                vec![1, 2, 3]
            };
            let mut pixels =
                PixelBuffer::from_vec(bytes, 1, 1, PixelDescriptor::RGB8_SRGB).unwrap();
            if self.prepared && self.mode == 6 {
                pixels =
                    pixels.with_color_context(Arc::new(ColorContext::from_cicp(Cicp::BT2100_PQ)));
            }
            let mut info = ImageInfo::new(1, 1, ImageFormat::Pnm);
            if self.prepared && self.mode == 4 {
                info = info.with_xmp(vec![42]);
            }
            Ok(DecodeOutput::new(pixels, info))
        }
    }

    #[test]
    fn prepared_state_cannot_change_pixels_metadata_or_invent_facts() {
        let baseline = Mutant {
            mode: 0,
            prepared: false,
        }
        .decode()
        .unwrap();
        for mode in 0..=6 {
            let result = typed(
                Mutant {
                    mode,
                    prepared: false,
                },
                &baseline,
            );
            if mode < 2 {
                result.unwrap();
            } else {
                let error = result.unwrap_err();
                let expected = match mode {
                    2 => "descriptor contradicts",
                    3 => "changed pixels",
                    4 => "changed metadata",
                    6 => "changed color context",
                    _ => "typed preparation:",
                };
                assert!(error.detail.contains(expected), "{}", error);
            }
        }
    }
}
