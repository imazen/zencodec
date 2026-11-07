//! A `Vec<u8>` has no alignment guarantee beyond 1. This allocator returns odd
//! addresses for byte allocations, which is legal, and which glibc never does.

use std::alloc::{GlobalAlloc, Layout, System};
use zencodec::AnimationFrame;
use zenpixels::{PixelBuffer, PixelDescriptor};

struct OddBytes;

fn widened(layout: Layout) -> Layout {
    Layout::from_size_align(layout.size() + 16, 16).unwrap()
}

// SAFETY: byte allocations come from a 16-aligned block offset by one, and dealloc
// undoes the same offset with the same layout. Others go straight to System.
unsafe impl GlobalAlloc for OddBytes {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
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
    for descriptor in [PixelDescriptor::RGB16_SRGB, PixelDescriptor::RGBAF32_LINEAR] {
        let mut source = PixelBuffer::new(4, 3, descriptor);
        {
            let mut rows = source.as_slice_mut();
            for y in 0..3 {
                rows.row_mut(y).fill(y as u8 + 1);
            }
        }
        let frame = AnimationFrame::new(source.as_slice(), 10, 0);
        let owned = frame.to_owned_frame();
        let copy = owned.pixels();
        for y in 0..3 {
            assert_eq!(
                copy.row(y),
                source.as_slice().row(y),
                "{descriptor:?} row {y}"
            );
        }
    }
}
