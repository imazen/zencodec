//! Format identity.
//!
//! The spike keeps identity open and small. The current crate's format table
//! (extensions, MIME types, magic numbers for every built-in format) carries
//! over unchanged in substance; it is not what this spike reimagines.

use core::fmt;

/// The facts that identify a format.
#[derive(Debug)]
pub struct FormatInfo {
    name: &'static str,
    mime_type: &'static str,
    extension: &'static str,
    probe_len: usize,
    detect: fn(&[u8]) -> bool,
}

impl FormatInfo {
    /// Describe a format. `detect` sees at least `probe_len` bytes, or the
    /// whole input when it is shorter.
    pub const fn new(
        name: &'static str,
        mime_type: &'static str,
        extension: &'static str,
        probe_len: usize,
        detect: fn(&[u8]) -> bool,
    ) -> Self {
        Self {
            name,
            mime_type,
            extension,
            probe_len,
            detect,
        }
    }
}

/// An image format. Two values are the same format when their names match.
#[derive(Clone, Copy)]
pub struct ImageFormat(&'static FormatInfo);

impl ImageFormat {
    /// Identify a format by its static description.
    pub const fn new(info: &'static FormatInfo) -> Self {
        Self(info)
    }

    /// Lowercase name, e.g. `"jpeg"`.
    pub const fn name(&self) -> &'static str {
        self.0.name
    }

    /// Preferred MIME type.
    pub const fn mime_type(&self) -> &'static str {
        self.0.mime_type
    }

    /// Preferred file extension, without the dot.
    pub const fn extension(&self) -> &'static str {
        self.0.extension
    }

    /// Bytes [`matches`](Self::matches) needs to be sure.
    pub const fn probe_len(&self) -> usize {
        self.0.probe_len
    }

    /// Whether `bytes` start an image of this format.
    pub fn matches(&self, bytes: &[u8]) -> bool {
        (self.0.detect)(bytes)
    }
}

impl PartialEq for ImageFormat {
    fn eq(&self, other: &Self) -> bool {
        self.0.name == other.0.name
    }
}
impl Eq for ImageFormat {}

impl core::hash::Hash for ImageFormat {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.0.name.hash(state);
    }
}

impl fmt::Debug for ImageFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ImageFormat({})", self.0.name)
    }
}

impl fmt::Display for ImageFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0.name)
    }
}
