use std::error::Error;
use std::io::{self, Write};
use zencodec::metadata_rewrite::{Action, RewriteRequest};
use zencodec::MetadataPolicy;

pub fn rewrite_jpeg(
    source: &[u8],
    writer: &mut dyn Write,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let request = RewriteRequest::new(source).with_policy(MetadataPolicy::ColorAndRotation);
    let plan = zenjpeg::metadata::rewrite(&request)?;

    if plan
        .report()
        .iter()
        .any(|entry| entry.action() == Action::OpaqueIcc)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "retained ICC profile has not been inspected",
        )
        .into());
    }

    for chunk in plan.chunks() {
        writer.write_all(chunk)?;
    }
    Ok(())
}
