use std::io::{Error, ErrorKind, Write};
use zencodec::{
    MetadataPolicy,
    metadata_rewrite::{Action, RewriteRequest},
};

pub fn rewrite_jpeg(
    source: &[u8],
    writer: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let request = RewriteRequest::new(source).with_policy(MetadataPolicy::ColorAndRotation);
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

    for chunk in plan.chunks() {
        writer.write_all(chunk)?;
    }
    Ok(())
}
