# Required density and pixel aspect

The checked encode API carries required resolution through generic, erased and
registered jobs. It does not resize pixels. A required value is exact; the
callback cannot turn its failure into permission to round or omit it.

```rust,ignore
let request = zencodec::operation::EncodeRequest::new(zencodec::ImageFormat::Png)
    .with_resolution(zencodec::Resolution {
        x: 254.0,
        y: 127.0,
        unit: zencodec::ResolutionUnit::Inch,
    });
let output = request.encode(&codecs, pixels)?;
```

`EncodeRequest::prepare` and `prepare_send` validate the input and call the
job's checked setter before offering it to the existing policy resolver.
`EncodeOffer::resolution` records the requirement that was confirmed. An
unsupported job refuses rather than inheriting a successful no-op.
The mutable setter leaves the job intact on refusal. The builder form consumes
its job on either success or failure. Replacing a prepared job's checked
requirements invalidates the offer, as with fidelity and encode policy.

## Two different meanings

Known units describe samples per physical distance. Unknown units describe only
pixel aspect: `(2, 1)` and `(1000, 500)` are equivalent. Neither can substitute
for a physical density. Both axes must be finite and strictly positive.

PNG stores integer pixels per metre or an unknown-unit ratio. JPEG JFIF stores
16-bit integer samples per inch/centimetre or an unknown-unit ratio. These
representations cannot preserve every value exactly. The unit relationships
permit 254×127 samples/inch to become exactly 10000×5000 samples/metre;
300×150 samples/inch has no exact integer PNG representation.
Sources: [PNG pHYs](https://www.w3.org/TR/png-3/#11pHYs) and
[JFIF specification](https://www.w3.org/Graphics/JPEG/jfif3.pdf).

`Resolution::is_equivalent_to` uses exact arithmetic on the represented binary
values and unit ratios. It never uses an epsilon. The existing `f64` fields do
not recover an original TIFF rational's lost precision; adapters needing exact
rational preservation must retain the original representation or refuse.
`try_integer_density(unit, maximum)` supplies a common carrier conversion with
checked range and exactness. Unknown-unit ratios are reduced before checking
the carrier's maximum. Invalid values, unit loss, unrepresentable values and
unqualified jobs have distinct `ResolutionError` variants.

## Interpretation and qualification boundaries

X and Y refer to stored sample axes. Retaining an orientation tag leaves these
axes unchanged; physically transposing the samples requires swapping the density
axes too. Video sample aspect, crop geometry and resize dimensions remain
separate operations.

A required native density carrier does not reconcile all metadata copies.
Joint preservation must inspect conflicts with EXIF/XMP or refuse where that
inspection is unavailable. PNG explicitly leaves conflicting Exif/PNG metadata
resolution outside the format specification's scope;
[its Exif guidance](https://www.w3.org/TR/png-3/#eXIf) is not a precedence rule
that a required-preservation adapter can assume.

Core tests establish exact unit conversion, unknown-unit ratio equivalence,
binary64 limits and neighbours, refusal through generic/dynamic/registered
paths, unchanged jobs and no callback on failed preparation. Native positive
carriage, independent output reading, metadata conflict handling and explicitly
permitted quantization are separate qualification gates tracked in
[issue 169](https://github.com/imazen/zencodec/issues/169).

`just resolution-core-check` passed, including strict core/new-test Clippy and
no-default-feature compilation. Public API snapshots regenerated successfully.
The resource-wrapper receipt is for the tests, not image storage:

```text
run-heavy: done rc=0 6s | peak-RSS 0.37GiB | min-avail 15972MiB | peak-load 0.40
```
