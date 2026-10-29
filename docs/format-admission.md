# Open format admission without another identity type

Status: consumer migration proposal for
[issue 121](https://github.com/imazen/zencodec/issues/121), not a new core API.

Core already supports open format identity. `ImageFormat::Custom` compares and
hashes its definition by the unique `ImageFormatDefinition::name`, and
`CodecSet::{decodable_formats, encodable_formats}` enumerate the actual
registrations. A registry iterator is not an exhaustive list of all possible
formats. Built-in and custom definitions describing the same format must use
consistent registration identity; do not infer aliases from extensions or MIME.

The current zencodecs `FormatSet` remains a closed named-format bitmask. It
cannot store arbitrary custom definitions. Its docs now explicitly describe
`all()` as all named formats, but silently ignoring `with(Custom(..))` remains
unsuitable for an open application allowlist. The custom decode side-channel
only covers its hard-coded names, not arbitrary plugin registration.

## Replacement semantics

Use the existing `ImageFormat` values in a finite ordered list. No new core
format ID, capability query or callback domain is needed.

```rust
// Semantics for the consumer's retained policy, not a proposed public struct.
let unrestricted: Option<Vec<zencodec::ImageFormat>> = None;
let deny_everything: Option<Vec<zencodec::ImageFormat>> = Some(vec![]);
let named_only = Some(vec![zencodec::ImageFormat::Png]);
```

- Absent restriction permits any registered format, including future/custom ones.
- An empty list denies every format. It must never become the unrestricted default.
- An explicit list admits exactly those identities; an unavailable encoder still
  returns a runtime error. Membership does not prove a codec can meet the job.
- Intersect constraints without resetting disabled codecs, implementation
  allowlists, preferences, cancellation or fallback settings.
- Generate candidates from registered implementations, then filter. Never build
  “all” by converting a fixed bitmask to a list.
- Unknown is not an encodable format. Validate it as an invalid configured entry;
  do not invent a custom-format wildcard or broaden to all formats.

The borrowed list needs no allocation. An owned convenience can retain a Vec;
this changes the current Copy bitmask cost and must be measured at the policy
construction boundary, not hidden behind per-operation registry cloning.

## Migration boundary

Keep FormatSet for named-format constants and internal compact flags. Add a
checked open-list builder and corresponding inspection path to the consumer
policy, deprecating the old builder when the replacement is available. Migrate
selection to a membership predicate rather than interpreting the legacy
`allowed_formats() == None` getter as unrestricted. Otherwise a new custom-list
policy could appear unrestricted to an old caller.

CodecIntent's format restrictions, policy intersection, available-format results
and registry controls must migrate together. A new setter alone leaves the
bitmask-based selection path dropping identities. Existing named-format callers
must preserve their exact meaning; do not silently make an old finite set a
wildcard. These signatures belong in the consumer API PR before implementation.

## Executable core control

`zencodec-testkit/tests/codec_set.rs` now exercises registered named/custom
formats under unrestricted, empty, exact-custom and unavailable-name filters.
The admitted custom format then performs real reference-wire detection and
decode with exact pixel comparison. This is a reference-codec protocol control,
not native PDF/RAW/JXL or browser qualification. The consumer's legacy bitmask
has not been changed, and issue 121 remains open until that migration lands.

Qualification on core 1279f174 plus the new control: all 18 codec-set tests and
strict scoped Clippy pass in an isolated export (rc0, 5s, peak RSS 0.31 GiB).
No production signatures, policy behavior or established assertions changed.
