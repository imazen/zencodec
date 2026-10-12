# Structural inventories: implementer guide

`zencodec::inventory` gives every decoder one way to say where each byte of a file is and
whether the decoder reads it. Its purpose is auditing. A decode → re-encode pipeline, a
lossless rewrite or a plain copy can carry metadata the decoder never looks at: camera serials
in a private chunk, text after the logical end, stale data in a `free` box. A decoder's
metadata API shows only what it understood; the inventory shows everything else too.

This guide is for codec authors implementing `DecodeJob::inventory`.

## The contract

`DecodeJob::inventory(&self, data) -> Result<Inventory, At<CodecError>>`:

- **Opting in.** The trait default returns `UnsupportedOperation::Inventory` in the
  shared error envelope. Codecs gate walkers and inventory-only dependencies behind a
  default-off `inventory` Cargo feature. Enabled implementations set
  `DecodeCapabilities::with_inventory(true)` and return an inventory or an error.
  The method stays callable when the feature is off. This adds no constructor bound
  to the decoder's native error type. Dynamic dispatch retains `BoxedError` and the
  shared envelope/cause inside it.
- **Full coverage.** Every byte of `data` belongs to exactly one top-level part, and
  `Inventory::validate` checks it.
- **Bad input still gets an inventory.** For malformed or truncated input, record what you
  understood, mark the rest `Malformed` with a `detail`, and return `Ok`. Errors are only for
  cancellation and resource limits, including `InventoryError::TooManyParts`.
- **Cost.** Walk the container: no pixel decoding, and no allocation proportional to the image
  dimensions.

## Dispositions describe this decoder

| Disposition | Use for |
|---|---|
| `Structure` | framing and index data the decoder reads: signatures, chunk/box headers, `iloc`, IFD entries |
| `ImageData` | coded data that becomes the decoded pixels |
| `Metadata(kind)` | data reported through `ImageInfo` / `DecodeOutput` (EXIF, XMP, ICC, CICP, HDR, gain map, orientation, resolution, animation, supplements) |
| `Dropped` | parsed, but nothing reaches the caller |
| `Skipped` | a recognised type the decoder doesn't read (thumbnails, comments, secondary images) |
| `Unknown` | a type the decoder doesn't recognise |
| `Trailing` | after the format's logical end |
| `Unreferenced` | inside a container, referenced by nothing |
| `Padding` | filler the format defines as meaningless |
| `Malformed` | could not be interpreted |

- **Native-only payloads.** A payload the decoder exposes only through its native API, not through
  zencodec, is `Skipped` or `Dropped`. Say so in `detail`.
- **Keep it in sync.** When decoding behaviour changes, update the inventory in the same commit.

## Nesting and coverage

- **Ranges.** A part's `range` includes its header. Children nest inside their parent and don't
  overlap each other.
- **Containers with a body.** A container whose children account for every byte after its header
  declares that sub-range with `Part::with_body`, and `validate` requires the children to tile it.
  After recording the children you found, call `fill_gaps(Some(parent), Unreferenced)` or
  `fill_gaps(Some(parent), Malformed)`.
- **Containers without a body** may list some children or none. Their own disposition covers the
  bytes they don't list.
- **Finishing a walk.** End every walk with `fill_gaps(None, Trailing)`.
- **Items stored elsewhere.** A logical unit whose bytes live elsewhere is split in two. A HEIF item
  is an `Item` part for its `infe` entry, plus one `Extent` part per `iloc` extent inside the
  `mdat`/`idat` that holds it. Both carry the same `Code(item_id)` tag.
- **Labels.** `label` is the identifying string an auditor scans for: an APPn signature, a PNG text
  keyword, a GIF application identifier, a HEIF item name or `content_type`. Copy it from the file,
  keep it to about 64 bytes, and never interpret it. `detail` is for the walker's own remarks.

## Coverage per format

Minimums; more is welcome where the codec already parses it.

- **JPEG:** every marker; scan data with its RSTn markers; fill bytes; bytes before SOI; MPF images
  after EOI, walked recursively.
- **PNG:** every chunk, with keywords as labels; data after IEND.
- **GIF:** headers; colour tables; every extension; image data; trailer.
- **WebP:** every RIFF chunk and ANMF sub-chunk; pad bytes; bytes past the RIFF size.
- **ISOBMFF:** every box and `ipco` property; item extents inside `mdat`/`idat`; slack in them as
  `Unreferenced`.
- **JPEG XL:** container boxes (`brob` labelled by its inner type); the codestream.
- **TIFF/DNG:** every IFD reached; out-of-line values; strips and tiles.
- **Bitmap formats:** headers, comments, colour tables, profiles, gaps.

## Rules the reviews kept enforcing

- **Find each consumed unit's internal end.** That is the size field, the stream end, the last
  parsed field, or the palette entries an index can reach. Bytes past it are a child part:
  - `Unreferenced` or `Dropped` when the caller never receives them;
  - the parent's `Metadata(kind)` when the decoder hands the whole blob over (an ICC profile with
    bytes past its declared size, an XMP item with its trailer). Say so in `detail`.
- **Entropy-coded leaves.** Locate the end of coded data when that costs O(compressed bytes) with
  O(1) state, for example LZW, zlib, or sequential Huffman. Otherwise keep the leaf whole and say in
  `detail` that bytes past the coded data are not distinguished.
- **Follow the decoder's cursor, not the length field,** wherever the two disagree.
- **Model which copy wins** (first, last, or last valid) per key, as the decoder does. Mark the
  copies it ignores.
- **Settings and limits.** The job's policy, its render mode and the decoder's limits change what
  is read. When the decoder would reject the file, demote the consumed claims after that point.
  Call the decoder's own checks; don't copy their constants.
- **Reuse the decoder's parsers.** Decide dispositions by calling the functions the decode path
  calls. Re-implementations drift.
- **Name other readers.** Dispositions follow the zencodec decode path. If `probe()` or another
  reader takes different bytes, name it in `detail`.
- **Bound the work.** Stop collecting at the part cap, before allocating per unit. Avoid quadratic
  lookups. Honour `max_input_bytes` and the stop token.
- **When a work budget runs out, report the unverified bytes unconsumed.** Only adversarial input
  exhausts a budget, so the safe answer is the right one there. Report what you couldn't verify as
  `Unknown` or `Malformed`, with a detail saying the decoder may read it. A consumed claim there is a
  hiding place any file can trigger.

## Testing

1. Run `zencodec_testkit::check_inventory` on encoder output and on the codec's corpus files. It
   checks a valid file, the same file with junk appended, and every truncation, and it is part of
   `check_all`, including the disabled-feature unsupported result. Also run
   `check_inventory_dispatch` to check generic/dynamic parity; erased configuration
   dispatch requires a `'static` config, without adding that bound to `check_all`.
2. Pin a fixture's full part list: every unit type the format defines, plus a private unit and
   trailing data.
3. Mutate in both directions:
   - Overwriting unconsumed bytes must leave the decode output and the reported metadata
     unchanged.
   - Overwriting a consumed leaf must change one of them, or its `detail` must say why it doesn't.

   The second direction is the one that catches over-claims.
4. Fuzz an `inventory` target that asserts `validate`.
5. Cross-check offsets and lengths against an independent dumper such as `exiftool -v3`. Run it as
   a test that executes when an environment variable names the tool, never one that skips itself
   when the tool is missing.
