# Registry configuration cloning

Before: `6653e2ec`; after: `ce5ef910` (production source unchanged by subsequent
qualification documentation/catalog updates).

The control registers four counted decoder configs, four ordinary encoder
configs and four transferable encoder configs. Before the change, cloning the
registry fails the first assertion with four decoder clones instead of zero.
Afterward, cloning invokes no decoder or encoder config clones. Starting a
controlled operation clones exactly its selected decoder once; two independent
encode jobs clone the selected encoder twice. The unsupported-controlled route
still refuses. Registration priority and extension isolation are covered by the
existing registry suite; required policies and fidelity remain covered by the
operation-request suite.

Implementation: registration lists remain independently owned vectors; their
entries hold shared immutable templates. Config cloning into each selected job
is unchanged. Collection factories already used shared ownership. Clone still
copies the lists and increments per-entry reference counts; this is not a
zero-allocation or constant-time claim. No wall-time, native codec allocation,
or end-to-end throughput improvement has been measured.

`just registry-sharing-check` reproduces qualification. Results:

- Before control: fails as intended, rc=101, 4 seconds, peak RSS 0.33 GiB.
- After registry/request controls: 24 passed; strict core/testkit all-target
  Clippy passed. rc=0, 9 seconds, peak RSS 0.38 GiB.
- Pinned native JPEG controlled execution: 9 passed, including registered
  dispatch, suspension, final-fact admission, refusal, unwind cleanup and exact
  sample parity. rc=0, 7 seconds, peak RSS 0.70 GiB.

RSS and elapsed values are run-heavy qualification-process receipts, not codec
performance comparisons. Full logs and immutable exports remain in
`~/tmp/zencodec-registry-sharing-20261029` on the validation host. Only inactive,
regenerable Cargo test executables were removed for disk space; the deletion
inventory, original failure and all qualification logs were retained.
