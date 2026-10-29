# Source catalog ownership and current guide qualification

Runtime/source input: core main 5df2ebe2. Regenerated all eight compiler-derived
catalogs outside workspace overrides; their public signature text is unchanged.
The source-detail generator now follows an implementation's resolved `for` type,
not the first referencing type. It no longer assigns generic blanket methods to
an arbitrary concrete type. The regression permutes the index order and checks
conversion and trait method ownership; it fails before the fix and passes after.

The initial catalog regeneration succeeded but guide assembly refused a stale
local Rustdoc revision (rc1, 19s, peak RSS 0.33 GiB). Rebuilding the eight-crate
Rustdoc reference for the same core revision fixed that prerequisite; the full
source/browser gate passed (rc0, 85s, peak RSS 0.33 GiB). The ownership correction
then regenerated the catalogs again and passed the same complete gate:

- 26 source tests, no failures or skips, plus the Python ownership regression.
- Browser checks cover migration diagrams, all code-first roles, actual source
  scopes, search, pagination, deprecated entries, independent context expansion,
  clipboard contents, keyboard operation and 390/768/1440 px layouts.
- rc0, 83s, peak RSS 0.33 GiB, minimum available 12675 MiB, peak load 3.53.

These are validation receipts, not codec performance or memory bounds. Full logs
on r7900x: `~/tmp/zencodec-jxl-main-20261029/docs.log`, `docs-full.log`, and
`docs-ownership.log`; failing-first regression: `~/tmp/zencodec-catalog-before.log`.
No existing browser assertion or public signature was changed.
