# Caller simplicity acceptance

Runtime source: `3999412a` export, corresponding to the advanced-module and
compiled two-line example in the subsequent review head. Workspace tests1453,
doctests, renderer, real-native/ICC controls, strict Clippy and no-default checks
pass. That original aggregate stops at a newly introduced HTML assertion.

Frontend source: `1ba852f0`, docs25 rerun. All29 Node checks and the full browser
suite pass, including original contrast/clipboard/context checks and both caller
and advanced pages at390/768/1440px. Wrapper rc0,70s,peakRSS0.39GiB. Runtime source
is unchanged; the frontend correction checks highlighted code as rendered text
and checks the advanced link as a link. No pre-existing assertion was weakened.
Earlier fixture/compiler and new-test failures remain retained.

The namespace aliases existing modules without maintaining duplicate signature
lists. The caller guide uses actual CodecSet methods; the example's test checks
round-trip samples. Complete cross-media rendering convenience remains open.
