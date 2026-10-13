# New contract coverage

This matrix tracks behavioral contracts, not a line-coverage percentage. A
passing reference codec alone does not certify real codec adapters. Format
fixtures must run enabled and disabled feature configurations where applicable.

| Contract | Reusable check | Real-codec integration |
|---|---|---|
| Exact timing, representability, overflow and rejected-frame atomicity | `check_animation_timing` with explicit cases | Pending |
| Ordinary quantization and returned encoded duration | `check_animation_timing` with independent clock expectations | Pending |
| Borrowed/owned/erased duration, frame index and pixel parity | `check_animation_durations` | Pending |
| Checked encode-policy admission, typed errors, delivery and rejection atomicity | `check_encode_policy_admission` | Pending |
| Checked decode-policy admission, typed errors and rejection atomicity | `check_decode_policy_admission` | Pending |
| Inventory enabled/disabled and dispatch equivalence | `check_inventory_dispatch` | Existing codec tests; extended matrix pending |
| Inventory input cap and exact-boundary admission | `check_inventory_input_limit` | Pending |
| Rewrite chunks, report bounds, repeatability, input/output/carrier caps and cancellation | `check_metadata_rewrite` | Pending |
| Rewrite pixels, timing, color, orientation and metadata retention | Caller-supplied observer plus codec wire/oracle tests | Pending |
| Nested plan provenance, packet expansion and regenerated metadata budgets | zenmeta rewrite regression tests | Shared implementation |
| Allocation failure classification and topology preservation at part cap | zencodec inventory regression tests | Shared implementation; allocator failure injection absent |

## API gaps

Animation sink decoding returns `OutputInfo`, which exposes geometry and pixel
layout but no frame duration or index. Timing checks cover borrowed, owned and
erased owned results; sink pixel checks remain useful, but cannot certify sink
timing. A frame-aware sink result is queued for the breaking release.

Checked-policy admission does not imply required fidelity or complete carrier
preservation. Tests use explicit expected decoded metadata to observe supported
fields; format wire inspection is needed for unknown carriers and ancillary
images. There is no general delivery-report contract yet.
