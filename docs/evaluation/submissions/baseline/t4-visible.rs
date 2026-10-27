use std::io::{Error, ErrorKind, Write};
use zencodec::metadata_rewrite::{Action, MetadataPolicy, RewriteRequest};

pub fn rewrite_jpeg(
    source: &[u8],
    mut writer: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let request = RewriteRequest::new(source)
        .with_policy(MetadataPolicy::preserve_color_and_orientation());
    let plan = zenjpeg::metadata::rewrite(&request)?;

    for entry in plan.report() {
        if entry.action() == Action::OpaqueIcc {
            return Err(Error::new(
                ErrorKind::InvalidData,
                "retained ICC has not been inspected; rewrite refused",
            )
            .into());
        }
    }

    plan.write_to(&mut writer)?;
    Ok(())
}
