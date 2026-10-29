//! Native JPEG guide examples. The second adds a private policy admission gate.
#![forbid(unsafe_code)]

mod ordinary {
    // EXAMPLE: jpeg_rewrite
    use zencodec::{MetadataPolicy, decode::BoxedError, metadata_rewrite::RewriteRequest};

    pub(crate) fn derivative(source: &[u8]) -> Result<Vec<u8>, BoxedError> {
        // Actual JPEG rewrite; scan data is borrowed without pixel decoding.
        // ANCHOR: jpeg_rewrite
        let request = RewriteRequest::new(source).with_policy(MetadataPolicy::ColorAndRotation);
        let plan = zenjpeg::metadata::rewrite(&request)?;
        // END: jpeg_rewrite
        // This preserves opaque ICC. Success does not certify its privacy.
        Ok(plan.to_vec()?)
    }
    // END EXAMPLE: jpeg_rewrite
}
pub(crate) use ordinary::derivative;

mod checked {
    // EXAMPLE: jpeg_icc_refusal
    use std::io::Write;
    use zencodec::{
        MetadataPolicy,
        decode::BoxedError,
        metadata_rewrite::{Action, RewriteRequest},
    };

    #[derive(Debug)]
    pub(crate) struct UninspectedIcc;
    impl std::fmt::Display for UninspectedIcc {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("retained ICC has not been inspected")
        }
    }
    impl std::error::Error for UninspectedIcc {}

    pub(crate) fn checked_derivative(
        source: &[u8],
        writer: &mut dyn Write,
    ) -> Result<(), BoxedError> {
        let request = RewriteRequest::new(source).with_policy(MetadataPolicy::ColorAndRotation);
        let plan = zenjpeg::metadata::rewrite(&request)?;
        // ANCHOR: jpeg_icc_refusal
        if plan
            .report()
            .iter()
            .any(|entry| entry.action() == Action::OpaqueIcc)
        {
            return Err(UninspectedIcc.into());
        }
        // All checks precede the first write. Writer errors can leave partial output.
        for chunk in plan.chunks() {
            writer.write_all(chunk)?;
        }
        // END: jpeg_icc_refusal
        Ok(())
    }
    // END EXAMPLE: jpeg_icc_refusal
}
pub(crate) use checked::{UninspectedIcc, checked_derivative};

mod inspected {
    // EXAMPLE: jpeg_icc_admission
    use crate::{
        policy::{Carrier, Context, Engine, PolicyResolver, PolicySet},
        policy_disclosure::{Obligation, Prepared, Privacy, TranscodeRoute},
    };
    use std::io::Write;
    use zencodec::{
        MetadataPolicy,
        decode::BoxedError,
        metadata_rewrite::{Action, RewriteRequest},
    };

    pub(crate) fn require_inspected_icc<R: PolicyResolver>(
        source: &[u8],
        writer: &mut impl Write,
        resolver: &mut R,
    ) -> Result<(), BoxedError> {
        // An additional, deliberately narrow requirement: retained ICC must be
        // inspected. Other carrier obligations need their own qualification.
        let request = RewriteRequest::new(source).with_policy(MetadataPolicy::ColorAndRotation);
        let plan = zenjpeg::metadata::rewrite(&request)?;
        let mut engine = Engine::new(PolicySet::default(), 8);
        // ANCHOR: jpeg_icc_admission
        for entry in plan.report() {
            if entry.action() == Action::OpaqueIcc {
                Prepared::opaque(
                    &source[entry.source_range()],
                    Carrier::Icc,
                    Obligation::UnknownCritical,
                    Privacy::OrientationOnlyV1,
                    TranscodeRoute::Rewrite,
                )
                .resolve(&mut engine, Context::default(), resolver, None)?;
            }
        }
        // END: jpeg_icc_admission
        // No bytes escape before every retained ICC obligation passes. This
        // prototype has no qualified ICC sanitizer, so opaque ICC refuses.
        for chunk in plan.chunks() {
            writer.write_all(chunk)?;
        }
        // A writer error can leave partial output; success is not a disk fsync.
        Ok(())
    }
    // END EXAMPLE: jpeg_icc_admission
}
pub(crate) use inspected::require_inspected_icc;
