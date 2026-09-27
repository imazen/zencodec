# Publishing metadata without surprises

Use `job.with_metadata_policy(meta, MetadataPolicy::ColorAndRotation)` when
publishing without attribution. Use `MetadataPolicy::Web` when Copyright,
Artist, Photographer and ImageEditor are intentionally public. These fields can
contain names, email addresses or other contact information; `Web` is not
anonymous publication. CameraOwnerName is ownership, not attribution, and both
presets drop it.

| Metadata | ColorAndRotation | Web |
|---|---|---|
| Orientation and validated EXIF color declarations | Keep | Keep |
| ICC (unless recognized redundant sRGB), CICP, HDR | Keep | Keep |
| Copyright, Artist, Photographer, ImageEditor | Drop | Keep |
| CameraOwnerName / Photoshop RAW OwnerName | Drop | Drop |
| Body/lens/DNG/Photoshop RAW serials, HostComputer, CameraLabel | Drop | Drop |
| ImageUniqueID | Drop | Drop |
| GPS, timestamps and UTC offsets | Drop | Drop |
| Thumbnails, MakerNotes, comments and unknown/private EXIF | Drop | Drop |
| XMP, including copies embedded inside EXIF | Drop | Drop |

## Explicit opt-ins

Start from a publishing preset. Starting from `KEEP_ALL` intentionally preserves
unknown fields and can preserve nested XMP/IPTC or proprietary payloads even if
some known identity fields are removed.

```rust
use zencodec::{ExifPolicy, MetadataPolicy, Retention};

// Camera/lens descriptions and capture dates, without enabling serial numbers,
// ownership, image identity or UTC offsets.
let exif = ExifPolicy::ORIENTATION_ONLY
    .with_camera(Retention::Keep)
    .with_datetimes(Retention::Keep);
let policy = MetadataPolicy::Custom(
    MetadataPolicy::ColorAndRotation.fields().with_exif(exif)
);
```

Device IDs, camera ownership, ImageUniqueID and UTC offsets have independent
`with_device_ids`, `with_camera_owner`, `with_image_unique_id`, and
`with_time_offsets` setters. `with_camera(Keep)` only enables descriptive tags;
`with_datetimes(Keep)` only enables dates. They do not reset previous explicit
choices. `with_camera(Discard)` also clears IDs/ownership/image identity, and
`with_datetimes(Discard)` also clears offsets, so existing removal chains do not
start leaking after the category split. Explicit setters can opt back in later.

ImageUniqueID identifies the image, not the camera. It can still link a published
copy to an identifiable original. Keeping it is appropriate for provenance and
archival workflows, not an implicit side effect of keeping camera descriptions.

## Untrusted and malformed EXIF

Publishing uses an allowlist; it does not depend on recognizing every vendor's
sensitive tag. The filter rebuilds EXIF from retained entries and leaves behind
unreferenced bytes, padding, unmodeled IFDs, extra IFD chains and trailing data.
Unparseable EXIF is dropped. Valid fields may be salvaged from truncated tables;
unreadable/out-of-bounds entries are skipped. KEEP_ALL is deliberately different:
it passes the source through, even if malformed.

Prunes drop MakerNotes entirely: they may contain GPS, serials, owner names and
previews, and their internal offsets cannot safely be relocated by this parser.
Retaining a thumbnail explicitly retains its opaque JPEG bytes, including any
metadata inside that JPEG. Both publishing presets discard the thumbnail.

Retained display/attribution tags must also belong to the correct directory:
Orientation/Artist/Copyright in IFD0 (or a retained IFD1), ColorSpace/Gamma and
Photographer/ImageEditor in the Exif IFD. Misplaced declarations are discarded.

Interop is not an opaque color blob. Only fixed-size R98/R03/THM Index and a
four-digit Version survive pruning; unknown/private fields and RelatedImage*
are removed. Orientation must be one SHORT/LONG value in 1..=8; ColorSpace one
SHORT; Gamma one unsigned rational with nonzero numerator and denominator.
Malformed display tags cannot preserve arbitrary strings or large payloads just
because their tag numbers are recognized. Attribution must have a text type.

## Scope: filtering metadata is not sanitizing a whole file

`Metadata::filtered` operates on its EXIF/XMP/ICC/CICP/HDR fields. It cannot remove
data the caller carries through some other path. In particular:

- ICC profiles are retained for correct color and may contain author names,
  descriptions, copyright or private tags. Converting pixels to a known output
  color space and emitting a clean profile/signaling is a separate operation.
  Removing or replacing the profile without a matching pixel transform can
  change the displayed colors.
- Container comments, IPTC/Photoshop resources, PNG text, duplicate metadata
  segments, trailers, MPF secondaries, gain maps and embedded previews require
  handling by the codec/pipeline. Copying original segments around the filtered
  `Metadata` record defeats its policy.
- TIFF/RAW files include image structure and sometimes necessary calibration
  in their metadata trees. Filtered EXIF output is a metadata blob for a fresh
  encode, not a rewritten standalone TIFF/RAW image.
- Filenames, filesystem metadata and identifiable image content are outside
  this API. No preset promises anonymity or removal of deliberately hidden data.

For an entire-file guarantee, the pipeline must construct the output from the
intended pixels and explicitly retained channels, handle every auxiliary image,
and verify the resulting container. The EXIF audit below does not certify that
larger operation.

## ExifTool audit (2026-09-27)

Reviewed installed ExifTool **13.50**, notably
`/usr/share/perl5/Image/ExifTool/Exif.pm`: standard OwnerName/SerialNumber,
DNG CameraSerialNumber (`0xC62F`), CameraLabel (`0xC7A1`), Photoshop RAW aliases
(`0xFDE8`/`0xFDE9`), Windows XP text fields, comments, embedded XMP/IPTC and
Interop. The explicit identity categories include the alternate owner/serial
tags; unrecognized/private carriers remain excluded by publishing presets.

References: [ExifTool EXIF tag definitions](https://exiftool.org/TagNames/EXIF.html),
[ExifTool source](https://github.com/exiftool/exiftool/blob/master/lib/Image/ExifTool/Exif.pm),
[ExifTool deletion limits](https://exiftool.org/exiftool_pod.html),
[IPTC ImageUniqueID guidance](https://www.iptc.org/std/photometadata/documentation/mappingguidelines/).
ExifTool itself documents that deleting metadata is format-dependent and does
not guarantee complete removal from every file type.

Reproduce the external audit:

```sh
python scripts/audit-exif-privacy.py \
  /mnt/v/output/corpus-builder/repro-images/manifest.jsonl --limit 10000
```

The script selects deterministic, deduplicated manifest entries, prioritizes
metadata-related issue titles, limits input size, and writes only to a fresh
private `/tmp/zencodec-exif-privacy-*` directory. It records source paths and EXIF
hashes there, without printing metadata values. ExifTool extracts source blobs;
small TIFFs are supplied directly because TIFF is itself the metadata tree.
The Rust example filters both presets and checks orientation, parseability and
idempotence. ExifTool independently reads every surviving output and rejects
anything outside the publishing allowlist, including unknown tags and nested
XMP/IPTC/MakerNotes. Synthetic tests cover both byte orders, duplicate/misplaced
identifiers, hidden Interop payloads, malformed display fields, truncations,
invalid counts/offsets and aliases. The testkit also mutation-tests separate
identifier checks when camera descriptions and timestamps are retained.

Final run: **10,000** manifest-listed files scanned; **2,324** distinct input
blobs (2,222 files with extractable EXIF plus 175 small TIFFs, deduplicated).
Three blobs were unparseable and dropped. The source scan reported 481 files
with warnings and one with an error; 51 TIFFs exceeded the 1 MiB direct-TIFF cap.
Surviving outputs: 1,748 Web + 1,686 ColorAndRotation = **3,434**, all checked by
ExifTool with **zero unexpected tags/errors**. Source formats included JPEG,
PNG, WebP, AVIF, HEIC/HEIF, JXL, TIFF and an unsupported BigTIFF (dropped).

The first expanded audit found an out-of-directory ColorSpace in a TIFF, which
motivated the directory validation and synthetic regression test. The final
run above includes that fix. Source hashes/paths and machine-readable results:
`/tmp/zencodec-exif-privacy-vho7697e/{sources,report}.json`. Corpus files were not
modified; no real metadata values were committed. This is sampled EXIF coverage,
not a claim that every repro or whole encoded container was sanitized.
