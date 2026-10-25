//! Private policy proposal, compiled by the native qualification test.
//! crate::native and crate::policy are experimental modules, not library exports.
#![forbid(unsafe_code)]

mod default_example {
    // EXAMPLE: policy_default
    use crate::{
        native::{Report, Request, RunResult},
        policy::Profile,
    };
    use std::{convert::Infallible, io::Write};
    use zencodec::{ImageFormat, Metadata};
    use zenpixels::PixelSlice;

    pub(crate) fn derivative(
        pixels: PixelSlice<'_>,
        metadata: &Metadata,
        writer: &mut impl Write,
    ) -> RunResult<Report, Infallible> {
        // This native fixture supports QOI/BMP, with no EXIF carrier.
        // Required supplied rights therefore cause a refusal before encoding.
        let request = Request::new(ImageFormat::Qoi, Profile::web_exif()).via_registry();
        // ANCHOR: policy_default
        let report = request.encode(pixels, metadata, writer)?;
        // END: policy_default
        // Success means bytes were accepted by Write, not durable publication.
        Ok(report)
    }
    // END EXAMPLE: policy_default
}
pub(crate) use default_example::derivative;

mod custom_example {
    // EXAMPLE: policy_custom
    use crate::{
        native::{Report, Request, RunResult},
        policy::{Decision, Decline, Profile, Question},
    };
    use std::{error::Error, fmt, io::Write};
    use zencodec::{At, ImageFormat, Metadata};
    use zenpixels::PixelSlice;

    #[derive(Debug)]
    pub(crate) struct AccountRuleUnavailable;
    impl fmt::Display for AccountRuleUnavailable {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("account publication rule unavailable")
        }
    }
    impl Error for AccountRuleUnavailable {}

    pub(crate) fn account_derivative(
        pixels: PixelSlice<'_>,
        metadata: &Metadata,
        writer: &mut impl Write,
        may_publish: Option<bool>,
    ) -> RunResult<Report, AccountRuleUnavailable> {
        // The request's required EXIF effects remain in force.
        let request = Request::new(ImageFormat::Qoi, Profile::web_exif()).via_registry();
        // ANCHOR: policy_custom
        let mut resolver = |_: Question<'_>| -> Result<Decision, At<AccountRuleUnavailable>> {
            match may_publish {
                None => Err(At::wrap(AccountRuleUnavailable).at_named("account publication rule")),
                Some(false) => Ok(Decision::reject(Decline::ApplicationRule)),
                Some(true) => Ok(Decision::UsePreset),
            }
        };
        let report = request.encode_with(pixels, metadata, &mut resolver, writer)?;
        // END: policy_custom
        // An account refusal, a missing rule, and a native error remain distinct.
        Ok(report)
    }
    // END EXAMPLE: policy_custom
}
pub(crate) use custom_example::{AccountRuleUnavailable, account_derivative};
