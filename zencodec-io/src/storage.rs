//! Compatibility path for shared allocation ownership.
//!
//! Codecs and authoring helpers use [`zencodec::storage`] directly; allocation
//! ownership does not require an I/O driver. Both paths name the same types.

pub use zencodec::storage::{OwnedStorage, SharedStorage, StorageBudget, StorageError};
