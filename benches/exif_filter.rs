//! EXIF filter benchmark — validates the zero-copy claim as thumbnail size
//! grows (1 KB → 1 MB).
//!
//! The point: a passthrough filter (`retain` with a keep-everything policy)
//! borrows the source and must NOT scale with thumbnail size, while a pruning
//! rewrite (and `to_bytes`) copies the thumbnail and scales linearly. If
//! `retain_passthrough` ever tracks thumbnail size, the zero-copy `Cow`
//! contract has regressed.
//!
//! Run: `cargo bench --bench exif_filter`

use zenbench::prelude::*;

use zencodec::exif::{Exif, ExifPolicy, Retention, retain};

/// A little-endian EXIF blob: IFD0 (orientation) → IFD1 (JPEG thumbnail of
/// `thumb_len` bytes). Thumbnail bytes are synthesized at runtime (never
/// committed).
fn exif_with_thumbnail(thumb_len: usize) -> Vec<u8> {
    let mut v = vec![b'I', b'I', 0x2A, 0x00];
    v.extend_from_slice(&8u32.to_le_bytes());
    // IFD0 @8: 1 entry (orientation), next → IFD1 @26.
    v.extend_from_slice(&1u16.to_le_bytes());
    v.extend_from_slice(&0x0112u16.to_le_bytes()); // Orientation
    v.extend_from_slice(&3u16.to_le_bytes()); // SHORT
    v.extend_from_slice(&1u32.to_le_bytes());
    v.extend_from_slice(&[6, 0, 0, 0]); // Rotate90
    v.extend_from_slice(&26u32.to_le_bytes()); // next = IFD1
    // IFD1 @26: 0x0201 (offset=56), 0x0202 (length), next=0.
    v.extend_from_slice(&2u16.to_le_bytes());
    v.extend_from_slice(&0x0201u16.to_le_bytes());
    v.extend_from_slice(&4u16.to_le_bytes()); // LONG
    v.extend_from_slice(&1u32.to_le_bytes());
    v.extend_from_slice(&56u32.to_le_bytes()); // thumbnail offset
    v.extend_from_slice(&0x0202u16.to_le_bytes());
    v.extend_from_slice(&4u16.to_le_bytes()); // LONG
    v.extend_from_slice(&1u32.to_le_bytes());
    v.extend_from_slice(&(thumb_len as u32).to_le_bytes());
    v.extend_from_slice(&0u32.to_le_bytes()); // next IFD
    // Thumbnail @56.
    v.extend(core::iter::repeat_n(0xABu8, thumb_len));
    v
}

fn build_group(suite: &mut Suite, thumb_len: usize, label: &'static str) {
    let blob = exif_with_thumbnail(thumb_len);
    // A pruning policy that KEEPS the thumbnail but drops GPS → forces a
    // rewrite that must copy the thumbnail.
    let prune = ExifPolicy::KEEP_ALL.with_gps(Retention::Discard);

    suite.group(label, move |g| {
        g.throughput(Throughput::Bytes(thumb_len as u64));
        g.throughput_unit("thumb-byte");

        let b1 = blob.clone();
        g.bench("parse", move |b| {
            b.iter(|| zenbench::black_box(Exif::parse(&b1)))
        });

        // Zero-copy passthrough: should be flat across thumbnail size.
        let b2 = blob.clone();
        g.bench("retain_passthrough", move |b| {
            b.iter(|| zenbench::black_box(retain(&b2, &ExifPolicy::KEEP_ALL)))
        });

        // Pruning rewrite: copies the thumbnail → scales with size.
        let b3 = blob.clone();
        g.bench("retain_prune_rewrite", move |b| {
            b.iter(|| zenbench::black_box(retain(&b3, &prune)))
        });

        let b4 = blob.clone();
        g.bench("parse_then_to_bytes", move |b| {
            b.iter(|| zenbench::black_box(Exif::parse(&b4).map(|x| x.to_bytes())))
        });
    });
}

fn bench_exif_filter(suite: &mut Suite) {
    build_group(suite, 1 << 10, "thumb_1KiB");
    build_group(suite, 64 << 10, "thumb_64KiB");
    build_group(suite, 1 << 20, "thumb_1MiB");
}

zenbench::main!(bench_exif_filter);
