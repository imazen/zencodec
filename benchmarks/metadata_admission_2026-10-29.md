# Checked metadata admission candidate

Source tested: `3795b19c139972997b1b86084d143c4afa534641`, based on
main `529b57975b8e8e7d7e7d62a49bbfc315f2367afe`. Later spec/recipe edits
change no tested Rust source. Native adoption remains unqualified.

`just metadata-admission-check`, isolated ARM64 export, run-heavy 3G/jobs2,
900-second timeout:

- Core all-feature library tests: 686 passed, none failed or ignored.
- Testkit operation-policy tests: 13 passed, none failed or ignored.
- Core all-feature/all-target strict Clippy: passed.
- Wrapper: rc0, 67s, peak RSS 0.51 GiB, minimum available 14224 MiB,
  peak load 1.50. These describe the validation process, not codec memory/performance.

New controls cover default refusal of nonempty metadata and empty clearing,
retained job identity/no native startup after rejection, checked builder/dyn
forwarding, concrete native error preservation and atomic metadata state,
ordinary/Send request refusal before execution, and continued operation when
no metadata requirement was requested. The positive shim fixture qualifies
forwarding only: it is not an encoder or native retention oracle.

Preserved failed attempts: the first export omitted the ignored workspace
Cargo.lock and refused `--locked` before compilation (rc101, 3s, 0.06 GiB).
The next used the prior qualified isolated workspace lock, SHA256
`cceafcb184efae78e61716d4a31166d257e0beba84a6a81e5463a4af23b1a9a3`,
and failed on two incorrectly named HDR fields in the new test (rc101, 65s,
0.44 GiB). Correcting that new fixture did not change existing assertions.
All attempts remain in the validation run directory `metadata-admission-20261029`.

Required next qualification: actual JPEG/PNG rejection or retention, unchanged
owner after native rejection, later setter/declaration conflict before output,
independent positive metadata readback, and reusable native testkit coverage.
The best-effort setter is not deprecated until the replacement is qualified.

All eight compiler-derived catalogs regenerated. Strict eight-crate Rustdoc and
26 site source tests passed with no skips (rc0, 29s, peak RSS 0.35 GiB,
minimum available 14122 MiB, peak load 1.07). The catalog run had first stopped
after generation because the archive lacked Git metadata and the runner omitted
`GITHUB_SHA` (rc1, 52s, 0.35 GiB). Supplying the exact exported source revision
and the existing locked site dependencies repaired the runner; no assertion was
changed. Browser interaction was not rerun: this changes signatures/catalogs,
not layout or navigation. Native retention remains the blocking acceptance gate.

## Shared readback oracle

`check_metadata_retained` compares the resolved expected output record with
completed native/independent readback. It never consults capability flags.
Opaque bytes, CICP, CLL, mastering display, diffuse white and orientation are
checked; extra metadata and privacy require separate conformance. Explicitly
requested baking may normalize orientation/EXIF only with transformed pixels.

Source `55baffbaf705ae58522b32a0e75f4ffa2c8d5d9b` passed the two focused controls
(including each field's omission/substitution and payload-free diagnostics),
testkit lib/tests strict Clippy, eight catalog generations and strict Rustdoc,
and all 26 site source tests. Wrapper rc0, 71s, peak RSS 0.34 GiB,
minimum available 14079 MiB, peak load 1.02. No native result is implied.

Preserved fixture compilation failures: nonexistent root DiffuseWhite re-export
(rc101, 25s, 0.36 GiB), corrected to zenpixels::hdr; incorrect CICP fixture field
(rc101, 9s, 0.33 GiB), corrected to color_primaries. The intermediate helper also
passed before stronger per-field substitution controls were added (rc0, 107s,
0.35 GiB). No pre-existing expectation or native output was changed.
