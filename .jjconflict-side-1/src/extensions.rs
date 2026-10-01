//! TypeMap-style extension storage for output types.
//!
//! [`Extensions`] stores multiple independently-typed values keyed by
//! [`TypeId`]. Values are stored as `Arc<dyn Any + Send + Sync>` for cheap
//! cloning. At most one value per concrete type.
//!
//! Uses linear scan over a `Vec` — optimal for 0–5 entries (typical codec use).

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::any::{Any, TypeId};

/// A type-map storing multiple independently-typed extension values.
///
/// Each concrete type `T: Any + Send + Sync + 'static` can be stored at most
/// once. Values are wrapped in `Arc` for cheap cloning.
///
/// # Example
///
/// ```rust
/// use zencodec::Extensions;
///
/// let mut ext = Extensions::new();
/// ext.insert(42u32);
/// ext.insert(99.5f64);
///
/// assert_eq!(ext.get::<u32>(), Some(&42));
/// assert_eq!(ext.get::<f64>(), Some(&99.5));
/// assert_eq!(ext.get::<i32>(), None);
/// assert_eq!(ext.len(), 2);
/// ```
#[derive(Clone, Default)]
pub struct Extensions {
    entries: Vec<(TypeId, Arc<dyn Any + Send + Sync>)>,
}

impl Extensions {
    /// Create an empty extension map.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a typed value, replacing any previous value of the same type.
    ///
    /// Returns the previous value if one existed and this is the sole `Arc`
    /// reference, otherwise returns `None`.
    pub fn insert<T: Any + Send + Sync + 'static>(&mut self, value: T) -> Option<T> {
        let id = TypeId::of::<T>();
        let new_arc: Arc<dyn Any + Send + Sync> = Arc::new(value);

        for (tid, arc) in &mut self.entries {
            if *tid == id {
                let old = core::mem::replace(arc, new_arc);
                return old
                    .downcast::<T>()
                    .ok()
                    .and_then(|a| Arc::try_unwrap(a).ok());
            }
        }
        self.entries.push((id, new_arc));
        None
    }

    /// Borrow a typed value if present.
    pub fn get<T: Any + Send + Sync + 'static>(&self) -> Option<&T> {
        let id = TypeId::of::<T>();
        for (tid, arc) in &self.entries {
            if *tid == id {
                return arc.downcast_ref();
            }
        }
        None
    }

    /// Remove and return a typed value.
    ///
    /// Returns `Some(T)` only when this is the sole `Arc` reference.
    /// Returns `None` if the type is not present or other references exist.
    pub fn remove<T: Any + Send + Sync + 'static>(&mut self) -> Option<T> {
        let id = TypeId::of::<T>();
        let pos = self.entries.iter().position(|(tid, _)| *tid == id)?;
        let (_, arc) = self.entries.swap_remove(pos);
        let arc_t: Arc<T> = arc.downcast().ok()?;
        Arc::try_unwrap(arc_t).ok()
    }

    /// Check whether a value of this type is present.
    pub fn contains<T: Any + Send + Sync + 'static>(&self) -> bool {
        let id = TypeId::of::<T>();
        self.entries.iter().any(|(tid, _)| *tid == id)
    }

    /// Number of stored extension values.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the map is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl core::fmt::Debug for Extensions {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Extensions")
            .field("len", &self.entries.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty() {
        let ext = Extensions::new();
        assert!(ext.is_empty());
        assert_eq!(ext.len(), 0);
        assert!(ext.get::<u32>().is_none());
    }

    #[test]
    fn insert_and_get() {
        let mut ext = Extensions::new();
        ext.insert(42u32);
        ext.insert(99.5f64);

        assert_eq!(ext.get::<u32>(), Some(&42));
        assert_eq!(ext.get::<f64>(), Some(&99.5));
        assert_eq!(ext.get::<i32>(), None);
        assert_eq!(ext.len(), 2);
        assert!(!ext.is_empty());
    }

    #[test]
    fn insert_replaces() {
        let mut ext = Extensions::new();
        let old = ext.insert(42u32);
        assert!(old.is_none());

        let old = ext.insert(99u32);
        assert_eq!(old, Some(42));
        assert_eq!(ext.get::<u32>(), Some(&99));
        assert_eq!(ext.len(), 1);
    }

    #[test]
    fn remove() {
        let mut ext = Extensions::new();
        ext.insert(42u32);
        ext.insert(99.5f64);

        let removed = ext.remove::<u32>();
        assert_eq!(removed, Some(42));
        assert!(ext.get::<u32>().is_none());
        assert_eq!(ext.len(), 1);
        assert_eq!(ext.get::<f64>(), Some(&99.5));
    }

    #[test]
    fn remove_missing() {
        let mut ext = Extensions::new();
        assert!(ext.remove::<u32>().is_none());
    }

    #[test]
    fn contains() {
        let mut ext = Extensions::new();
        assert!(!ext.contains::<u32>());
        ext.insert(42u32);
        assert!(ext.contains::<u32>());
        assert!(!ext.contains::<f64>());
    }

    #[test]
    fn clone_shares_arcs() {
        let mut ext = Extensions::new();
        ext.insert(42u32);

        let cloned = ext.clone();
        assert_eq!(cloned.get::<u32>(), Some(&42));
        assert_eq!(cloned.len(), 1);
    }

    #[test]
    fn remove_after_clone_fails() {
        let mut ext = Extensions::new();
        ext.insert(42u32);

        let _cloned = ext.clone();
        // Two Arc refs exist, so try_unwrap fails
        let removed = ext.remove::<u32>();
        assert!(removed.is_none());
    }

    #[test]
    fn debug_format() {
        let mut ext = Extensions::new();
        ext.insert(42u32);
        let s = alloc::format!("{:?}", ext);
        assert!(s.contains("Extensions"));
        assert!(s.contains("len: 1"));
    }
}
