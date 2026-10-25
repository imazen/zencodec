# Native JPEG policy qualification

Private experiments against zenjpeg main `a2a128ab0bc36f01338225f61c285f462ae67ca6`.
These do not add a public policy API. QOI remains a simple control in the I/O
harness; JPEG exercises metadata placement, progressive scans, output timing,
orientation, rewriting and refusal before publication.

From a checkout outside the zen workspace path overrides, install `djpeg`
(`libjpeg-turbo-progs` on Ubuntu), then run:

```sh
just jpeg-policy-setup
just jpeg-policy-check
```

Use `just jpeg-policy-sync <export>` to update an existing source export.
The standalone manifest pins zenjpeg's unpublished dependency bridges. Its
`dependencies.lock` is copied to Cargo.lock by setup; no source in the local
zenjpeg checkout is modified. Native JPEG is a separate workspace so its newer
pixel dependencies do not alter the original I/O experiments.

[Evidence and limits](../../docs/policy-jpeg.md). Tests fail when the independent
`djpeg` oracle is missing. Fixtures are generated with the actual encoder and
metadata serializers; there are no committed image binaries.
