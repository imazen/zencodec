# Frozen PNG synchronization probe

This standalone consumer checks current main's shared incremental contract on
native RGB/RGBA PNG. Run from an export outside ambient workspace overrides:

```sh
just sansio-sync-png-check
```

The pinned PNG currently fails dependency resolution before compilation:
stable-only archmage/magetypes does not admit the pixel cohort's beta.2. This is
an intentionally retained integration gate, not a skipped or passing test. No
PNG source or existing assertions are changed here. See
[the synchronization inventory](../../docs/sansio-branch-sync.md).
