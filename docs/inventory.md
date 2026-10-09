# Structural inventories: implementer guide

`zencodec::inventory` gives every decoder one way to say where each byte of a file is and
whether the decoder reads it. Its purpose is auditing: a decode → re-encode pipeline, a
lossless rewrite or a plain file copy can carry metadata the decoder never looks at (camera
serials in a private chunk, text after the logical end, stale data in a `free` box). A
decoder's metadata API only shows what it understood. The inventory shows everything else
too.

This guide is for codec authors implementing `DecodeJob::inventory`.

## The contract

`DecodeJob::inventory(&self, data) -> Result<Option<Inventory>, Self::Error>`:

- `Ok(None)` means the codec doesn't implement inventories (the trait default). A codec that
  implements it sets `DecodeCapabilities::with_inventory(true)` and never returns `None`.
- Every byte of `data` belongs to exactly one top-level part. `Inventory::validate` checks
  this, and `zencodec-testkit`'s `check_inventory` runs it for you on a valid file, on the
  same file with 37 junk bytes appended, and on every truncation from `truncation_lengths`.
- Malformed or truncated input still yields an inventory. Record what you understood, mark
  the rest `Disposition::Malformed` with a `detail` saying why, and return `Ok`. Errors are
  for cancellation and resource limits, including `InventoryError::TooManyParts`.
- No pixel decoding, no allocation proportional to image dimensions. Walk the container.

## Dispositions must be true

A disposition describes what *this decoder* does with the bytes, not what the format allows:

| Disposition | Use for |
|---|---|
| `Structure` | framing and index data the decoder reads: signatures, chunk/box headers, `iloc`, IFD entry tables, the MPF index |
| `ImageData` | coded data that becomes the decoded pixels (scan data, `IDAT`, extents of the decoded item, strips, frames) |
| `Metadata(kind)` | data the decoder reports through `ImageInfo` / `DecodeOutput` (EXIF, XMP, ICC, CICP, HDR static metadata, gain map, orientation, resolution, animation timing, supplements) |
| `Dropped` | parsed, but nothing reaches the caller |
| `Skipped` | a recognised type the decoder doesn't read (thumbnails, comments, Photoshop resources, secondary images it doesn't decode) |
| `Unknown` | a type the decoder doesn't recognise |
| `Trailing` | after the format's logical end |
| `Unreferenced` | inside a container but referenced by nothing (slack in `mdat`) |
| `Padding` | filler the format defines as meaningless (`free`/`skip`, fill bytes, RIFF pad bytes) |
| `Malformed` | could not be interpreted |

If the decoder only exposes a payload through its native API and not through zencodec, it is
`Skipped` or `Dropped` from zencodec's point of view; say so in `detail` (for example
`"native PngInfo::text_chunks only"`). When decoding behaviour changes, update the
inventory in the same commit. The test that pins a fixture's inventory catches the drift.

## Nesting and coverage

- A part's `range` includes its header. Children nest inside the parent's range and must not
  overlap each other.
- A container whose children account for every byte after its header declares that sub-range
  with `Part::with_body`. `validate` then requires the children to tile the body exactly. Use
  `Inventory::fill_gaps(Some(parent), Disposition::Unreferenced)` (or `Malformed`) after
  recording the children you found.
- A container without a body may have partial children (an `APP13` segment listing a few
  Photoshop resources) or none; its own disposition then covers its unlisted bytes.
- Finish every walk with `inv.fill_gaps(None, Disposition::Trailing)`. Anything after the
  logical end that you did not otherwise identify becomes a `Trailing` gap.
- Every part has a byte range, so a logical unit whose bytes live elsewhere is split in two.
  A HEIF item's declaration is its `infe` entry: record it as a part of kind `Item` (tag
  `Code(item_id)`, label = item name or type). Its data is one `Extent` part per `iloc` extent,
  nested in the `mdat`/`idat` part that holds it, with the same `Code(item_id)` tag.

## Labels and details

`label` holds the short identifying string an auditor scans for: the APPn signature
(`"Exif"`, `"http://ns.adobe.com/xap/1.0/"`, `"ICC_PROFILE"`, `"Photoshop 3.0"`,
`"JUMB"`), the PNG text keyword, the GIF application identifier, the HEIF item name or
`content_type`, the `auxC` URN. Copy it from the file, bounded to a sensible length (64 bytes
is plenty), and never interpret or translate it. `detail` is for the walker's own remarks.

## Per-format expectations

The minimum each walker covers. More detail (nested IFDs, Photoshop resources) is welcome
when the codec already parses it.

- **JPEG:**
  - Every marker from SOI to EOI, with APPn and COM labelled by signature.
  - `ScanData` after each SOS, including RSTn markers.
  - Fill bytes and stray bytes as `Padding` / `Malformed`.
  - Bytes before SOI.
  - After EOI: MPF images (`EmbeddedImage`, recursively walked as JPEG), container items an
    XMP `Container:Directory` points at, and anything else as `Trailing`.
- **PNG:**
  - The signature, then every chunk, with its CRC status in `detail` when it is bad.
  - Text keywords as labels.
  - Data after IEND.
- **GIF:**
  - Header plus logical screen descriptor.
  - Colour tables, every extension (application identifier as label).
  - Image descriptors and image data, the trailer, and bytes after it.
- **WebP:**
  - The RIFF header.
  - Every chunk, including chunks after a simple-format image and every ANMF sub-chunk.
  - Pad bytes.
  - Bytes past the RIFF size.
- **ISOBMFF (AVIF, HEIC):**
  - Every box, recursively for container boxes, and every `ipco` property.
  - Item extents inside `mdat`/`idat`, with unreferenced bytes as `Unreferenced`.
  - Bytes after the last box.
- **JPEG XL:**
  - Signature or container boxes, with `brob` labelled by its inner type.
  - Codestream boxes as `ImageData`, and bytes after the last box.
- **TIFF / DNG:**
  - The header and every IFD in the chain plus SubIFD/EXIF/GPS/Interop IFDs.
  - Out-of-line values and strips/tiles.
  - Unreferenced bytes between them.
- **BMP / PNM / farbfeld:**
  - Headers and PNM comment lines.
  - The BMP colour table, embedded profile, gap before the pixel array, and trailing data.

## Testing

1. `zencodec_testkit::check_inventory(config, &bytes)` on encoder output (it is part of
   `check_all` once the capability is declared) and on the codec's own corpus fixtures.
2. A fixture-pinned test: one file containing every unit type the format defines, plus a
   private/unknown unit and trailing data, with the expected parts asserted.
3. Fuzz: add `inventory` to the codec's fuzz targets. It must not panic, and `validate` must
   pass on every input.
4. Cross-check against an independent dumper (`exiftool -v3`, `tiffdump`, `jxlinfo`,
   `heif-info`) on a sample of real files. Every unit it lists should appear with the same
   offset and length. Write it as a test that runs when an environment variable names the
   tool's binary, set by the justfile or CI job that has the tool, never a test that skips
   itself when the tool is missing.
