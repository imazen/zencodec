//! Shared sans-I/O output completion for raster and container writers.
//! All offsets are relative to the output stream, never a surrounding file.
//! A driver must validate all patches before any final tail/patch effects.
use alloc::vec::Vec;

/// Bytes to write over earlier output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Patch {
    /// Offset of the first byte relative to the start of this output stream.
    pub offset: u64,
    /// The new bytes.
    pub bytes: Vec<u8>,
}

#[derive(Debug, Default)]
#[non_exhaustive]
/// Remaining output and ordered replacements returned by a producer.
pub struct OutputFinish {
    /// Bytes not yet consumed through `pending_output`.
    pub tail: Vec<u8>,
    /// Earlier bytes to overwrite; empty for a forward-only layout.
    pub patches: Vec<Patch>,
}

impl OutputFinish {
    /// The remaining bytes and the patches to apply after them.
    pub fn new(tail: Vec<u8>, patches: Vec<Patch>) -> Self {
        Self { tail, patches }
    }

    /// Validate the complete finish before appending its tail or applying any
    /// patch. Offsets are relative to this output stream, not a surrounding file.
    /// Returns the final stream length, including the tail.
    ///
    /// This checks ranges and arithmetic only. A writer must separately enforce
    /// its output allowance and whether its chosen layout permits patches.
    /// Overlapping patches apply in their listed order. Successful validation
    /// cannot make an external writer's later I/O failures atomic.
    pub fn validate(&self, emitted_bytes: u64) -> Result<u64, PatchError> {
        let tail = u64::try_from(self.tail.len()).map_err(|_| PatchError::OutOfRange)?;
        let total = emitted_bytes
            .checked_add(tail)
            .ok_or(PatchError::OutOfRange)?;
        for patch in &self.patches {
            let length = u64::try_from(patch.bytes.len()).map_err(|_| PatchError::OutOfRange)?;
            let end = patch
                .offset
                .checked_add(length)
                .ok_or(PatchError::OutOfRange)?;
            if end > total {
                return Err(PatchError::OutOfRange);
            }
        }
        Ok(total)
    }

    /// Append the tail and apply validated patches to an in-memory stream.
    /// Validation or allocation failure leaves its length and contents unchanged.
    /// Capacity may grow on success; this does not impose a memory allowance.
    pub fn apply_to(self, file: &mut Vec<u8>) -> Result<(), PatchError> {
        self.validate(u64::try_from(file.len()).map_err(|_| PatchError::OutOfRange)?)?;
        file.try_reserve(self.tail.len())
            .map_err(|_| PatchError::AllocationFailed)?;
        file.extend_from_slice(&self.tail);
        for patch in self.patches {
            // Successful reservation proves the complete extent fits usize.
            // validate checked every range against that same extent.
            let start = patch.offset as usize;
            file[start..start + patch.bytes.len()].copy_from_slice(&patch.bytes);
        }
        Ok(())
    }
}

/// A patch that does not fit the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PatchError {
    /// The patch lies outside the written bytes.
    OutOfRange,
    /// The in-memory destination could not reserve the remaining output.
    AllocationFailed,
}

impl core::fmt::Display for PatchError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::OutOfRange => "patch or final length lies outside the written stream",
            Self::AllocationFailed => "cannot reserve remaining output",
        })
    }
}
impl core::error::Error for PatchError {}
