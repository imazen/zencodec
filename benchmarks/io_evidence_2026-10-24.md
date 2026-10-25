# Complete IO experiment evidence

Observed and verified 2026-10-25 UTC. The implementation proposal is
[here](../docs/io-contract-proposal.md); measured results are in the
[initial](io_validation_2026-10-24.md) and
[qualification](io_contracts_2026-10-24.md) reports.

The full collected evidence is outside git:

```text
dev:/mnt/v/output/zencodec/io-contracts-2026-10-24/
dev:/mnt/v/output/zencodec/io-contracts-2026-10-24.tar.gz
archive bytes: 85412753
archive SHA-256: ae344032fcecb82ebd4726ac076042e0beb8396f25f69eeb8ac45b507fcf7691
collected files: 513, plus MANIFEST.json
uncompressed collected bytes: 137984293
```

Verify its current presence and content, rather than assuming this observation
still holds:

```sh
ssh dev 'cd /mnt/v/output/zencodec && sha256sum -c io-contracts-2026-10-24.tar.gz.sha256'
```

The bundle contains complete successful and diagnostic-failure logs; authored
MP4/WebM/Matroska/IVF and disk fixtures; strict FFmpeg decode results; preserved
synthetic/QOI prototype binaries and hashes; native AV1 test output; all
zenbench run/object records; perf counters; heaptrack traces and interpreted
reports; dependency locks and fetched-source hashes. It excludes Cargo caches.
The README maps phases to commits and identifies exploratory measurements.

`MANIFEST.json` records every collected file's size and SHA-256.
[`pack-evidence.py`](../perf/io-validation/pack-evidence.py) then reopens the
completed archive and hashes every decompressed member without extracting it,
checking membership and rejecting duplicates. The full archive passed that
verification. The local experiment trees remain preserved too.

Reproduce packaging from a deliberately collected evidence directory using
`just io-validation-pack-evidence <directory> <archive-outside-directory>`.
It refuses existing manifests/archives and does not select or upload files.
New measurements belong in a new bundle, preserving this one.
