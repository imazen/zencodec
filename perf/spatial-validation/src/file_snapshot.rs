//! Private Linux source adapter. Snapshot at the filesystem boundary, then use
//! the existing positional API; no codec-specific seek or version trait.
use rustix::fs::{Mode, OFlags, ioctl_ficlone, openat};
use std::{fmt, fs::File};
use zencodec_io::{FileReadAt, StdSourceError};

#[derive(Debug)]
pub enum Error {
    Source(std::io::Error),
    NonRegularSource,
    LogicalBytes { actual: u64, limit: u64 },
    Create(std::io::Error),
    Clone(std::io::Error),
    Ready(StdSourceError),
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Source(_) => f.write_str("cannot inspect snapshot source"),
            Self::NonRegularSource => f.write_str("snapshot source must be a regular file"),
            Self::LogicalBytes { actual, limit } => {
                write!(f, "snapshot extent {actual} exceeds limit {limit}")
            }
            Self::Create(_) => f.write_str("cannot create an anonymous snapshot inode"),
            Self::Clone(_) => f.write_str("filesystem refused the required atomic clone"),
            Self::Ready(_) => f.write_str("cannot open positional reads on the cloned inode"),
        }
    }
}
impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Source(e) | Self::Create(e) | Self::Clone(e) => Some(e),
            Self::Ready(e) => Some(e),
            _ => None,
        }
    }
}
fn admit(actual: u64, limit: u64) -> Result<(), Error> {
    if actual > limit {
        Err(Error::LogicalBytes { actual, limit })
    } else {
        Ok(())
    }
}

/// The source and spool directory must be on one filesystem supporting FICLONE
/// and O_TMPFILE. Refuse if either is unavailable: a sequential copy would not
/// establish an atomic source revision under concurrent in-place writes.
///
/// The unnamed inode is owned solely through FileReadAt. No writable handle or
/// path is returned. The original can change after cloning without changing this
/// representation. Closing the source releases the snapshot inode automatically.
///
/// The bound covers logical file extent, checked before and after the clone. It
/// is not a disk-block, metadata-memory, CPU or syscall-latency allowance. Nor
/// does a filesystem instant prove an application finished a multi-write update.
pub fn clone_source(source: &File, spool: &File, max_bytes: u64) -> Result<FileReadAt, Error> {
    let metadata = source.metadata().map_err(Error::Source)?;
    if !metadata.is_file() {
        return Err(Error::NonRegularSource);
    }
    admit(metadata.len(), max_bytes)?;
    let snapshot = openat(
        spool,
        ".",
        OFlags::TMPFILE | OFlags::RDWR | OFlags::CLOEXEC,
        Mode::RUSR | Mode::WUSR,
    )
    .map_err(|e| Error::Create(e.into()))?;
    let snapshot = File::from(snapshot);
    ioctl_ficlone(&snapshot, source).map_err(|e| Error::Clone(e.into()))?;
    admit(snapshot.metadata().map_err(Error::Source)?.len(), max_bytes)?;
    FileReadAt::new(snapshot).map_err(Error::Ready)
}
