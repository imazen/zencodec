//! Contracts for codec authors and custom pipeline adapters.
//!
//! Applications normally start with [`crate::CodecSet`]. These are the same
//! contracts used internally by the convenience APIs, not a separate protocol
//! or a second version of a codec. Existing import paths remain available.
//!
//! Choose this layer when implementing a codec, controlling incremental input,
//! borrowing component planes, or supplying your own storage/transport.

// Aliases keep associated types, errors and future additions together with
// their defining modules, without a second maintained list of signatures.
pub use crate::{collection, decode, encode, frame, source, storage};
