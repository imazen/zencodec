//! Error chain helpers for codec error inspection.
//!
//! Codec errors are typically nested: `BoxedError` → `At<MyCodecError>` →
//! `LimitExceeded`. [`CodecErrorExt`] provides convenient methods to find
//! common cause types. [`find_cause`] is the generic version for arbitrary
//! error types.
//!
//! Works with `thiserror` `#[from]` variants, `whereat::At<E>` wrappers,
//! and any error type that properly implements `source()`.

use crate::{LimitExceeded, UnsupportedOperation};

/// Extension trait for inspecting codec errors.
///
/// Blanket-implemented for all `core::error::Error + 'static` types.
/// Walks the [`source()`](core::error::Error::source) chain to find
/// common codec error causes without knowing the concrete error type.
///
/// Works through any wrapper that delegates `source()`:
/// `thiserror` `#[from]` variants, `whereat::At<E>`, `Box<dyn Error>`, etc.
///
/// # Example
///
/// ```rust,ignore
/// use zencodec::CodecErrorExt;
///
/// let result = dyn_decoder.decode();
/// if let Err(ref e) = result {
///     if let Some(limit) = e.limit_exceeded() {
///         eprintln!("limit exceeded: {limit}");
///     } else if let Some(op) = e.unsupported_operation() {
///         eprintln!("not supported: {op}");
///     }
/// }
/// ```
pub trait CodecErrorExt {
    /// Find an [`UnsupportedOperation`] in this error's cause chain.
    fn unsupported_operation(&self) -> Option<&UnsupportedOperation>;

    /// Find a [`LimitExceeded`] in this error's cause chain.
    fn limit_exceeded(&self) -> Option<&LimitExceeded>;

    /// Find a cause of arbitrary type `T` in this error's cause chain.
    fn find_cause<T: core::error::Error + 'static>(&self) -> Option<&T>;
}

impl<E: core::error::Error + 'static> CodecErrorExt for E {
    fn unsupported_operation(&self) -> Option<&UnsupportedOperation> {
        find_cause::<UnsupportedOperation>(self)
    }

    fn limit_exceeded(&self) -> Option<&LimitExceeded> {
        find_cause::<LimitExceeded>(self)
    }

    fn find_cause<T: core::error::Error + 'static>(&self) -> Option<&T> {
        find_cause::<T>(self)
    }
}

// Manual impl for trait objects — the blanket impl requires Sized.
impl CodecErrorExt for dyn core::error::Error + Send + Sync + 'static {
    fn unsupported_operation(&self) -> Option<&UnsupportedOperation> {
        find_cause::<UnsupportedOperation>(self)
    }

    fn limit_exceeded(&self) -> Option<&LimitExceeded> {
        find_cause::<LimitExceeded>(self)
    }

    fn find_cause<T: core::error::Error + 'static>(&self) -> Option<&T> {
        find_cause::<T>(self)
    }
}

impl CodecErrorExt for dyn core::error::Error + Send + 'static {
    fn unsupported_operation(&self) -> Option<&UnsupportedOperation> {
        find_cause::<UnsupportedOperation>(self)
    }

    fn limit_exceeded(&self) -> Option<&LimitExceeded> {
        find_cause::<LimitExceeded>(self)
    }

    fn find_cause<T: core::error::Error + 'static>(&self) -> Option<&T> {
        find_cause::<T>(self)
    }
}

impl CodecErrorExt for dyn core::error::Error + 'static {
    fn unsupported_operation(&self) -> Option<&UnsupportedOperation> {
        find_cause::<UnsupportedOperation>(self)
    }

    fn limit_exceeded(&self) -> Option<&LimitExceeded> {
        find_cause::<LimitExceeded>(self)
    }

    fn find_cause<T: core::error::Error + 'static>(&self) -> Option<&T> {
        find_cause::<T>(self)
    }
}

/// Walk an error's [`source()`](core::error::Error::source) chain to find
/// a cause of type `T`.
///
/// Starts with the error itself, then follows `source()` links. Returns
/// the first match.
///
/// Prefer [`CodecErrorExt`] methods for common types. Use this for
/// codec-specific error types not covered by the extension trait.
pub fn find_cause<'a, T: core::error::Error + 'static>(
    mut err: &'a (dyn core::error::Error + 'static),
) -> Option<&'a T> {
    loop {
        if let Some(t) = err.downcast_ref::<T>() {
            return Some(t);
        }
        err = err.source()?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::boxed::Box;
    use alloc::string::String;
    use core::fmt;

    // A simple codec error with source() chain via manual impl
    #[derive(Debug)]
    enum TestCodecError {
        Limit(LimitExceeded),
        Unsupported(UnsupportedOperation),
        Other(String),
    }

    impl fmt::Display for TestCodecError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::Limit(e) => write!(f, "limit: {e}"),
                Self::Unsupported(e) => write!(f, "unsupported: {e}"),
                Self::Other(s) => write!(f, "other: {s}"),
            }
        }
    }

    impl core::error::Error for TestCodecError {
        fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
            match self {
                Self::Limit(e) => Some(e),
                Self::Unsupported(e) => Some(e),
                Self::Other(_) => None,
            }
        }
    }

    #[test]
    fn ext_limit_exceeded_direct() {
        let err = LimitExceeded::Width {
            actual: 5000,
            max: 4096,
        };
        assert_eq!(err.limit_exceeded(), Some(&err));
    }

    #[test]
    fn ext_limit_exceeded_through_source_chain() {
        let inner = LimitExceeded::Pixels {
            actual: 100_000_000,
            max: 50_000_000,
        };
        let err = TestCodecError::Limit(inner.clone());
        assert_eq!(err.limit_exceeded(), Some(&inner));
    }

    #[test]
    fn ext_unsupported_through_source_chain() {
        let err = TestCodecError::Unsupported(UnsupportedOperation::AnimationEncode);
        assert_eq!(
            err.unsupported_operation(),
            Some(&UnsupportedOperation::AnimationEncode)
        );
    }

    #[test]
    fn ext_returns_none_when_absent() {
        let err = TestCodecError::Other("something else".into());
        assert!(err.limit_exceeded().is_none());
        assert!(err.unsupported_operation().is_none());
    }

    #[test]
    fn ext_through_boxed_error() {
        let inner = LimitExceeded::Memory {
            actual: 1_000_000_000,
            max: 512_000_000,
        };
        let err = TestCodecError::Limit(inner.clone());
        let boxed: Box<dyn core::error::Error + Send + Sync> = Box::new(err);
        assert_eq!(boxed.limit_exceeded(), Some(&inner));
    }

    #[test]
    fn ext_find_cause_generic() {
        let err = TestCodecError::Unsupported(UnsupportedOperation::DecodeInto);
        let found: Option<&UnsupportedOperation> = err.find_cause();
        assert_eq!(found, Some(&UnsupportedOperation::DecodeInto));
    }

    // find_cause free function still works
    #[test]
    fn find_cause_free_fn() {
        let err = LimitExceeded::Width {
            actual: 5000,
            max: 4096,
        };
        let found = find_cause::<LimitExceeded>(&err);
        assert_eq!(found, Some(&err));
    }
}
