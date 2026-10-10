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

## Pitfalls the first reviews found

Independent reviews of the first twelve codec walkers kept finding the same mistakes. Check each one before
asking for review.

- **Bytes hidden inside a consumed part.** For every consumed unit, find where the decoder stops reading it:
  a size field, framing, the end of a zlib or LZW stream, a parsed field list, the number of palette entries an
  index can reach. Bytes after that point are a child part (`Unreferenced` or `Dropped`), not part of the
  unit. Examples from review:
  - PNG: decompressed data past the last row; an iCCP name longer than 79 bytes.
  - tRNS/PLTE: entries beyond `2^bit_depth`.
  - TGA: colour-map entries no index reaches.
  - JPEG: holes in the MPF index; extended-XMP GUIDs.
  - JPEG 2000: the tail of a marker segment past its parsed fields.
  - JPEG AI: RDI display-mapping blobs.
- **Entropy-coded leaves.** Finding the end of coded data is in scope when it costs O(compressed bytes) with
  O(1) state, as with GIF LZW, PNG zlib and a count-only Huffman pass over sequential JPEG scans. It is out of
  scope when it needs per-coefficient history or memory proportional to the image. Examples: VP8L, compressed
  ALPH, JPEG AC-refinement and arithmetic scans, JPEG AI ANS threads, JPEG 2000 packet data, compressed TIFF
  strips.
  - Keep such a leaf whole as `ImageData`, and say in its `detail` that bytes after the end of the coded data
    are not distinguished.
  - Callers should know that `unconsumed()` does not cover that slack.
- **The decoder's cursor, not the length field.** When the decoder ignores a declared length (JPEG 2000
  SIZ/COD/QCD, a PNM header byte), the bytes between where it stops and the declared end are not consumed.
  When it reads past a declared end, they are.
- **Which copy wins.** Model the decoder's rule exactly, and mark the copies it ignores `Dropped` or
  `Skipped`. The rule is "last wins" for most overwrites, "first wins" for JPEG 2000 `colr`, and "last valid
  wins" for JPEG 2000 `cdef`/`pclr`. Apply it per key, not per segment: JPEG DAC entries, DQT/DHT slots and
  TIFF duplicate tags are all per key.
- **Settings and limits change the answer.**
  - The job's policy (strict, permissive, `allow_progressive`), its `GainMapRender` mode and the decoder's
    default limits (pixels, frames, file size) decide whether a unit is read, or whether the file decodes at
    all.
  - When the decoder would reject the file, every consumed disposition after the point of rejection is
    false. Demote it, or say so in the `detail`.
  - Call the decoder's own limit checks rather than copying their constants.
- **Reuse the decoder's parsers.** Decide dispositions by calling the functions the decode path calls, not by
  re-implementing them. Re-implementations drift; the review findings were mostly drift. Examples:
  - container probes;
  - metadata selection;
  - the UltraHDR stack: zenjpeg's container probe and gain-map selection, ultrahdr-core's XMP parsing,
    `zencodec::gainmap::parse_iso21496_fmt`.

  For identifiers that live in another crate (`zencodec::ISO_21496_1_URN`), grep is not enough. Trace from
  the zencodec entry points.
- **More than one reader.** `probe()`, `decode()` and the animation decoder can each read different bytes:
  WebP has three readers, the GIF probe reads differently, and PNG frames have their own path. Dispositions
  describe the zencodec decode path. When another reader differs, name it in the `detail`.
- **Test in both directions.**
  - Overwriting bytes reported unconsumed must leave the decode output and reported metadata unchanged.
  - Overwriting a consumed leaf must change one of them, or its `detail` must say why not (for example
    "parsed for validity only").
  - A test in the first direction alone cannot find an over-claimed part. For an audit, over-claiming is
    the dangerous direction: `unconsumed()` silently leaves that part out.
- **Bounded work.**
  - Precompute first-occurrence indices; a per-unit linear scan is quadratic.
  - Stop collecting at the part cap, before allocating per unit.
  - Avoid owned per-part strings for repeated gaps.
  - Cap pre-emit allocations such as TIFF IFD entries.
  - Honour `max_input_bytes` and the stop token.
  - Measure adversarial inputs with `/usr/bin/time -v`.
- **Oracles.** Compare by offset and length. Match by role (which region, which frame) and in file order.
  When filtering an oracle's phantom units, derive the filter from the oracle's own structure, not the
  walker's. Pin the list of files the oracle fails on.

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
