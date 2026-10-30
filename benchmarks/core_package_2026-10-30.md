# Core release-content qualification — 2026-10-30

Main source a3548902ab7446262c811622231403ddaad22e79 passes default workspace
all-target tests, workspace doctests and strict all-target Clippy, plus core
all-feature/all-target tests, doctests, Clippy and strict Rustdoc. An isolated
export generated its lock without the zen workspace's path overrides. This
still uses the explicit git-patched paired pixel source, not registry-only
resolution. Main's CI, documentation and fuzz runs also passed on that SHA:
https://github.com/imazen/zencodec/actions/runs/41004321646
https://github.com/imazen/zencodec/actions/runs/41004321711
https://github.com/imazen/zencodec/actions/runs/41004321838

The actual normalized crate exposed a packaging defect: examples/frame_policy_probe.rs
imports perf/frame-policy-cost.rs, absent from the package include list. Checking
that example in the unpacked original crate fails with the missing-file diagnostic.
The corrected manifest includes only that required source file, not the full perf
workspace. No library source or public signature changes.

Corrected source aa6f0582782cae6fbee04a4766e68434c5343e12 was packaged and unpacked
outside the workspace. All packaged all-feature/all-target tests, doctests and
strict Clippy pass. The gate uses the pinned zenpixels source explicitly; this
qualifies tarball contents, not availability of unpublished dependencies. The
new script has separate paired-source and unpatched registry modes, with no
automatic fallback. Reproduce: just core-package-paired-check NEW_DIRECTORY;
after dependencies publish, just core-package-check NEW_DIRECTORY.

All runs used run-heavy --mem 3G --jobs 2. Full logs on r7900x under
/home/lilith/tmp/zencodec-core-release-20261030:

- run.log: source gates/package creation, rc0, 103s, peak RSS 0.46 GiB,
  minimum available 13261 MiB, peak load 1.29. resolved.lock retains the graph.
- package-before.log: failing example, rc101, 2s, peak RSS 0.25 GiB,
  minimum available 13822 MiB, peak load 0.60.
- package-fixed.log: corrected actual tarball gates, rc0, 2s, peak RSS 0.14 GiB,
  minimum available 13820 MiB, peak load 0.28.

These wrapper measurements describe validation only. No compile-time comparison,
codec-memory bound or registry publication is claimed. Required release approval,
paired dependency availability and current release-head CI remain gates. Media
PR188 is separate: its library changes are in the unpublished media crate, not
core src/. The complete image/video/transcoding objective remains open.
