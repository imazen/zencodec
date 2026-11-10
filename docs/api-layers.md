# Application calls and advanced contracts

Applications start with a configured `CodecSet`:

```rust,ignore
let image = codecs.decode(bytes)?;
let output = codecs.encode(format, image.pixels())?;
```

This converts one image's pixels. It is compiled and round-trip tested in
`zencodec-testkit/tests/simple_guide.rs`. Register codecs once; callers do not
need to coordinate capability queries, row delivery or plane borrowing.

`zencodec::advanced` groups the existing decode, encode, frame, collection,
source and storage modules for codec and adapter authors. These are aliases,
not alternative versions. There is one protocol and one implementation of each
contract; associated types and errors stay with their defining module.

The homepage is the short application guide. The detailed journeys remain at
`journeys.html`; authoring contracts and migration notes are at `advanced.html`.
The guide and generated Rustdoc default to dark styling.

A single short render operation covering an image, a selected page, an animation
or a video is still missing. It must define selection, orientation, destination
color, metadata and resource defaults, then execute existing advanced contracts.
Adding another low-level trait hierarchy would not close that convenience gap.
