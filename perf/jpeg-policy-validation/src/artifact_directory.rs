//! Exclusive directories for retained test artifacts, including repeated runs.
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub(crate) fn create(parent: &Path, label: &str) -> std::io::Result<PathBuf> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    std::fs::create_dir_all(parent)?;
    loop {
        let ordinal = NEXT.fetch_add(1, Ordering::Relaxed);
        let directory = parent.join(format!("{label}-{}-{ordinal}", std::process::id()));
        match std::fs::create_dir(&directory) {
            Ok(()) => return Ok(directory),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
}

#[test]
fn repeated_labels_preserve_prior_artifacts() {
    let parent = PathBuf::from(std::env::var_os("JPEG_POLICY_ARTIFACTS").unwrap());
    let first = create(&parent, "directory-control").unwrap();
    std::fs::write(first.join("sentinel"), b"retained").unwrap();
    let second = create(&parent, "directory-control").unwrap();
    assert_ne!(first, second);
    assert_eq!(std::fs::read(first.join("sentinel")).unwrap(), b"retained");
    assert!(!second.join("sentinel").exists());
}
