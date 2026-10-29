# Late descriptions: candidate, not production behavior

Application source: zenpipe PR108 at `5248cabf1483a483d96bd149b621e7d64d3ec424`.
The candidate is held for explicit approval to change established rejection
expectations. No application checkout, assertion or native codec was modified.

## Concrete proposed contract

When a retained route already promises `DiscardDescriptive`, late EXIF/XMP need
not invalidate it merely because their bytes changed. Those source bytes never
reach its encoder. Continue to reject changes to color, orientation, dimensions,
format and density; preserve the existing primary-image/container-scope checks.
`PreserveReported` remains strict. PNG inventory permits only late eXIf/iTXt on
the discard route; other late metadata tags still refuse.

This does not parse arbitrary XML or certify whole-file PII absence. It does not
permit output under a changed interpretation. The same source/encoder/output
owners and verification remain in place; no additional policy callback or
transferable proof is introduced. It removes an unnecessary refusal for an
already-discarded effect, not a general deferred-obligation implementation.

## Native evidence

`consumer-test.patch` constructs real PNG with EXIF and malformed XMP, moves the
unchanged CRC-bearing chunks after IDAT, and tests both direct selection and
UsePreset under an explicit discard requirement. Candidate output equals the
clean-input route byte-for-byte; independent png decoding equals the source
samples. Storage is released. PreserveReported still refuses.

`orientation-control.patch` includes an actual late Rotate90 EXIF carrier. Success
must retain Rotate90, or the operation must refuse. The fixture first verifies
that the header placement reports Rotate90. This control passes on the candidate;
no silent successful loss of rendering orientation is allowed.

`candidate.patch` contains the implementation and separate controls retaining
rejection of geometry/format/ICC/orientation/density drift and preserving strict
EXIF/XMP equality for preservation. The original combined late-metadata rejection
test is deliberately unchanged: it FAILS on the candidate at its obsolete-for-
this-proposal EXIF expectation. That is an approval gate, not a hidden skip.

Observed runs (all serialized, run-heavy 3G/jobs2, timeout180):

- Baseline new native witness fails with MetadataCompleteness: rc101, 8s,
  peak RSS 0.66 GiB, minimum available 13459 MiB.
- Initial broader prototype: native positive witness passes, rc0, 2s,
  peak RSS 0.24 GiB. It was narrowed before the final qualification.
- Final candidate: 29 integration tests, one multi-case predicate control and
  strict library/integration Clippy pass, rc0, 4s, peak RSS 0.28 GiB,
  minimum available 13909 MiB, peak load 0.09.
- Original rejection unit control fails on the candidate as expected: rc101, 2s,
  peak RSS 0.28 GiB. Its assertions were not changed.

These are validation receipts, not codec performance/memory claims. Native pins
are inherited unchanged from the application revision; this does not qualify
later dependency updates or every decoder implementation.

## Reproduce

`just late-metadata-prepare REPOSITORY REVISION OUTPUT` exports the colocated git
repository (not a jj workspace's apparent git root) and creates before/after
copies containing the new controls. It never alters the source checkout.

Run the following inside each export through run-heavy with an isolated target
or `cargo clean -p zencodecs` before switching exports:

```
cargo test -p zencodecs --no-default-features --features std,cms,jpeg,png,checked-transcode --test checked_transcode --locked
cargo test -p zencodecs --no-default-features --features std,cms,jpeg,png,checked-transcode --lib late_description_controls --locked
cargo clippy -p zencodecs --no-default-features --features std,cms,jpeg,png,checked-transcode --lib --test checked_transcode --locked -- -D warnings
```

Baseline failure and the separate unchanged contract control are intentional
observations. A normal all-tests shipping gate cannot pass until the contract
change is approved and its expectations are explicitly revised.

Retained raw logs and immutable exports:
`/home/lilith/tmp/zencodec-late-metadata-20261029` (before.log, after.log,
narrow.log, orientation.log, existing-contract.log, final.log). No binary archive.

## Current-main revalidation (2026-10-29)

The candidate applies without fuzz to zenpipe main
`b6e4f80125ef299e6b1c776437bb09f90e7afa3e`, preserving its current dependency
locks. The baseline native discard witness still fails (rc101, 4s, peak RSS
0.26 GiB). The candidate's 29 native integration controls, field-change unit
control and strict Clippy pass (rc0, 5s, peak RSS 0.32 GiB).

The orientation oracle is now stricter: on failure it requires the typed
`LateMetadata` refusal and zero retained storage, rather than accepting any
error. On success it still requires retained Rotate90 and exact independent
pixel readback. This strengthened control and strict Clippy pass on the same
candidate (rc0, 3s, peak RSS 0.27 GiB). No production behavior or established
assertion was changed. The approval request concerns only the existing blanket
EXIF/XMP rejection under explicit discard; interpretation and preservation
checks remain required.

Sources are frozen under `~/tmp/zencodec-late-current-b6e4f801/` and
`~/tmp/zencodec-late-current-b6e4f801-strict/`. Rust source timestamps were
refreshed before switching exports to prevent stale shared-target artifacts;
source bytes and dependency locks remained frozen. These are validation
resource receipts, not native memory bounds or performance measurements.

The unchanged established blanket-rejection control still fails on the current
candidate (rc101, 2s, peak RSS 0.32 GiB). Its failure is retained in
`established-contract.log`; it has not been removed, ignored or reinterpreted.
Production adoption remains pending explicit approval of that behavior change.
