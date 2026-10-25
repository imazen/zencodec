//! Stored operation requirements, checked automatically when executed.
//!
//! These requests are reusable configuration. Building one performs no codec
//! work. Execution uses the registered template and checked job primitives;
//! required policies and fidelity targets cannot fall back to legacy setters.
//! Pixel conversion is an explicit pipeline operation, not an implicit retry.

use crate::{
    At, CodecSet, CodecSetError, DecodeOutput, DecodePolicy, EncodeOutput, EncodePolicy,
    ImageFormat, encode::Fidelity,
};
use alloc::borrow::Cow;
use alloc::vec::Vec;
use core::fmt;
use zenpixels::{PixelDescriptor, PixelSlice};

/// A checked operation failure. Native causes remain in the source chain.
#[derive(Debug)]
#[non_exhaustive]
pub enum OperationError {
    /// Codec registration, configuration or execution failed.
    Codec(CodecSetError),
    /// The decoder could not produce the required descriptor without conversion.
    OutputDescriptor {
        /// Required output interpretation and layout.
        requested: PixelDescriptor,
        /// The decoder's actual output interpretation and layout.
        actual: PixelDescriptor,
    },
}

impl fmt::Display for OperationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Codec(e) => write!(f, "operation: {e}"),
            Self::OutputDescriptor { requested, actual } => write!(
                f,
                "required output {requested:?}; decoder produced {actual:?}"
            ),
        }
    }
}
impl core::error::Error for OperationError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Codec(e) => Some(e),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default)]
enum Output {
    #[default]
    Native,
    Prefer(Vec<PixelDescriptor>),
    Require(PixelDescriptor),
}

/// Reusable requirements for decoding one image into owned pixels.
///
/// Set-level controls (limits, stop/pulse, policy) are inherited. An explicitly
/// supplied policy replaces the set's policy and is checked before the decoder
/// receives input. Resource limits keep their codec-defined enforcement scope;
/// this request does not turn an estimate into a whole-operation memory bound.
#[derive(Clone, Debug, Default)]
pub struct DecodeRequest {
    format: Option<ImageFormat>,
    policy: Option<DecodePolicy>,
    output: Output,
}

impl DecodeRequest {
    /// Detect the registered input format and request its native output.
    pub const fn new() -> Self {
        Self {
            format: None,
            policy: None,
            output: Output::Native,
        }
    }

    /// Decode as this format instead of detecting it.
    pub const fn with_format(mut self, format: ImageFormat) -> Self {
        self.format = Some(format);
        self
    }

    /// Record a required policy, replacing the inherited set policy.
    pub const fn with_policy(mut self, policy: DecodePolicy) -> Self {
        self.policy = Some(policy);
        self
    }

    /// Require this output descriptor. An unsupported result is an error;
    /// conversion or lower precision is never selected implicitly.
    pub fn with_output(mut self, descriptor: PixelDescriptor) -> Self {
        self.output = Output::Require(descriptor);
        self
    }

    /// Prefer these descriptors in order, permitting the decoder's native
    /// fallback. Inspect the returned pixels for the selected descriptor.
    pub fn preferring_output(mut self, descriptors: impl Into<Vec<PixelDescriptor>>) -> Self {
        self.output = Output::Prefer(descriptors.into());
        self
    }

    /// Validate configuration and decode. No capability-query choreography is
    /// required. Failures retain a location trace and the native codec cause.
    #[track_caller]
    pub fn decode(
        &self,
        codecs: &CodecSet,
        input: &[u8],
    ) -> Result<DecodeOutput, At<OperationError>> {
        let run = || -> Result<DecodeOutput, OperationError> {
            let format = self
                .format
                .or_else(|| codecs.detect(input))
                .ok_or(OperationError::Codec(CodecSetError::UnrecognizedFormat))?;
            let preferred = match &self.output {
                Output::Native => &[][..],
                Output::Prefer(list) => list,
                Output::Require(descriptor) => core::slice::from_ref(descriptor),
            };
            let job = codecs
                .checked_decode_job(format, self.policy)
                .map_err(OperationError::Codec)?;
            let decoder = job
                .into_decoder(Cow::Borrowed(input), preferred)
                .map_err(|e| OperationError::Codec(e.into()))?;
            let output = decoder
                .decode()
                .map_err(|e| OperationError::Codec(e.into()))?;
            if let Output::Require(requested) = self.output {
                let actual = output.pixels().descriptor();
                if actual != requested {
                    return Err(OperationError::OutputDescriptor { requested, actual });
                }
            }
            Ok(output)
        };
        run().map_err(|e| At::wrap(e).at_named("checked image decode"))
    }
}

/// Reusable requirements for encoding pixels with a registered template.
///
/// A fidelity target is checked at configuration time. This confirms the
/// requested mode/quality selection, not a perceptual score or that arbitrary
/// inputs satisfy a backend's content-dependent lossless restrictions.
#[derive(Clone, Debug)]
pub struct EncodeRequest {
    format: ImageFormat,
    fidelity: Option<Fidelity>,
    policy: Option<EncodePolicy>,
}

impl EncodeRequest {
    /// Use the registered encoder template, inheriting the set's controls.
    pub const fn new(format: ImageFormat) -> Self {
        Self {
            format,
            fidelity: None,
            policy: None,
        }
    }

    /// Require this fidelity target; unsupported, substituted and unreported
    /// targets fail before encoding pixels.
    pub const fn with_fidelity(mut self, fidelity: Fidelity) -> Self {
        self.fidelity = Some(fidelity);
        self
    }

    /// Record a required policy, replacing the inherited set policy.
    pub const fn with_policy(mut self, policy: EncodePolicy) -> Self {
        self.policy = Some(policy);
        self
    }

    /// Validate configuration and encode, preserving strided pixel input and
    /// native errors. The stored request and registered template are reusable.
    #[track_caller]
    pub fn encode(
        &self,
        codecs: &CodecSet,
        pixels: PixelSlice<'_>,
    ) -> Result<EncodeOutput, At<OperationError>> {
        let run = || -> Result<EncodeOutput, CodecSetError> {
            codecs
                .checked_encode_job(self.format, self.fidelity, self.policy)?
                .encode(pixels)
                .map_err(CodecSetError::from)
        };
        run().map_err(|e| At::wrap(OperationError::Codec(e)).at_named("checked image encode"))
    }
}
