# Joint transcode and publication experiment

This preserves the executable prototype as a patch to a pinned zenpipe source
export. It adds no feature to a shipped crate. Its API spelling is experimental.

```rust,ignore
let ready = request.transcode_for_publication(&codecs, input, &mut policy)?;
let receipt = ready.write_to(&mut writer, &pulse)?;
```

The same prepared job supplies metadata and finish-first evidence to the joint
policy question. Success moves its encoded vector and drops backend extras
before publication. Publication checks Pulse around short writes and flush,
retains typed errors and confirmed byte counts, and retries Interrupted with a
new Pulse check. Other I/O errors are terminal.

Every finite wait-memory requirement refuses this general route: an arbitrary
Pulse can retain a raster, and vector capacity is not its length. The test-only
fixed-array/NoPulse variant explores a closed ownership model. Its `size_of`
claim is an owner-layout calculation, not measured process or stack retention;
it does not qualify a general finite-memory API.

```sh
# Checkout is read only: prepare exports one immutable committed object.
just publication-witness-prepare /path/to/zenpipe /path/to/new-export
# Run under the machine's resource wrapper with a disk-backed TMPDIR.
just publication-witness-check /path/to/new-export /path/to/new-results
```

The first patch pins the four core crates to e59b3803; the second contains the
prototype and controls. The third applies zenpipe's published repository-URL
repair for the unchanged zensim revision, allowing a clean dependency fetch.
No local workspace overrides are needed. Keep exports outside directories with
ancestor Cargo patches. Preparation refuses an existing destination, and
validation preserves complete logs and fails on any command or inventory gap.

The originating run passed seven unit and 23 native controls, with the 16
ordinary native outcomes unchanged; strict Clippy passed with and without the
feature and with async enabled. That run used the previously cached zensim URL.
The combined export was independently replayed: all 7 unit and 23 native
controls passed, all 16 ordinary outcomes were unchanged, five resolved graphs
contained no split core-family sources, and all three strict Clippy variants
passed. The wrapper reported `rc=0 11s | peak-RSS 0.33GiB | min-avail 4726MiB |
peak-load 0.65`. [Receipt](../../benchmarks/publication_replay_2026-10-28.json).
No throughput,
copy-cost, native workspace or whole-container privacy result is claimed.
