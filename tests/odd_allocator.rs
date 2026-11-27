//! A `Vec<u8>` has no alignment guarantee beyond 1. This allocator returns odd
//! addresses for byte allocations, which is legal, and which glibc never does.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

thread_local! {
    static FAIL_BYTES: Cell<usize> = const { Cell::new(0) };
}
use zencodec::AnimationFrame;
use zenpixels::{PixelBuf, PixelFormat};

struct OddBytes;

fn widened(layout: Layout) -> Layout {
    Layout::from_size_align(layout.size() + 16, 16).unwrap()
}

// SAFETY: byte allocations come from a 16-aligned block offset by one, and dealloc
// undoes the same offset with the same layout. Others go straight to System.
unsafe impl GlobalAlloc for OddBytes {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if layout.size() != 0
            && FAIL_BYTES
                .try_with(|size| {
                    if size.get() == layout.size() {
                        size.set(0);
                        true
                    } else {
                        false
                    }
                })
                .unwrap_or(false)
        {
            return std::ptr::null_mut();
        }
        if layout.align() == 1 {
            let base = unsafe { System.alloc(widened(layout)) };
            if base.is_null() {
                base
            } else {
                unsafe { base.add(1) }
            }
        } else {
            unsafe { System.alloc(layout) }
        }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if layout.align() == 1 {
            unsafe { System.dealloc(ptr.sub(1), widened(layout)) }
        } else {
            unsafe { System.dealloc(ptr, layout) }
        }
    }
}

#[global_allocator]
static ODD: OddBytes = OddBytes;

/// The copy allocated bytes with no room to align them, then `.expect`ed the wrap, so
/// a wide format on such an allocator panicked.
#[test]
fn to_owned_frame_copies_wide_formats_on_any_allocator() {
    for descriptor in [PixelFormat::RGB16_SRGB, PixelFormat::RGBAF32_LINEAR] {
        let mut source = PixelBuf::new(4, 3, descriptor.clone()).unwrap();
        {
            let mut rows = source.as_view_mut();
            for y in 0..3 {
                rows.row_mut(y).fill(y as u8 + 1);
            }
        }
        let frame = AnimationFrame::new(source.as_view(), 10, 0);
        let owned = frame.try_to_owned_frame().unwrap();
        let copy = owned.pixels();
        for y in 0..3 {
            assert_eq!(
                copy.row(y),
                source.as_view().row(y),
                "{descriptor:?} row {y}"
            );
        }
    }
}

#[test]
fn frame_copy_returns_the_allocation_failure_and_leaves_the_borrow_valid() {
    let samples = PixelBuf::new(137, 3, PixelFormat::RGB16_SRGB).unwrap();
    let frame = AnimationFrame::new(samples.as_view(), 35, 4);
    FAIL_BYTES.set(137 * 3 * 6);
    let failure = frame.try_to_owned_frame().unwrap_err();
    assert_eq!(FAIL_BYTES.get(), 0);
    assert!(matches!(
        failure.error(),
        zenpixels::BufferError::AllocationFailed
    ));
    assert_eq!(frame.frame_index(), 4);
    assert_eq!(frame.pixels().row(2), samples.as_view().row(2));
    assert_eq!(
        frame.try_to_owned_frame().unwrap().pixels().row(2),
        frame.pixels().row(2)
    );
}
