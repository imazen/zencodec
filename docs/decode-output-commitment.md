# Early refusal of required decoded output

Status: proposal after source review, 2026-10-28. Not a new implemented API.
The caller-facing API stays the same:

```rust
let output = DecodeRequest::new()
    .with_output(PixelDescriptor::RGBA8_SRGB)
    .decode(&codecs, input)?;
```

Today the operation gives that preference to the decoder, reconstructs the
pixels, then checks the returned descriptor. The existing allocation probe
observed 786,880 allocated bytes while refusing a 512 × 512 RGB8 image's
requested BGRA8 output. That measurement is from the reference codec, not a
native-codec memory measurement. See [P2 and its probe](api-usability-review-2026-10-27.md#4-performance-review).

## Smallest candidate on the retained decoder

```rust
trait Decode: Sized {
    type Error: core::error::Error + Send + Sync + 'static;

    // Proposed name; no production implementation yet.
    fn committed_output(&mut self)
        -> Result<Option<PixelDescriptor>, Self::Error>
    {
        Ok(None)
    }

    fn decode(self) -> Result<DecodeOutput, Self::Error>;
}
```

`Some(descriptor)` means this exact decoder, holding the input and negotiated
preferences, will return that descriptor or fail. The query performs no pixel
reconstruction. Mutable access permits caching parsed information for the
subsequent decode. The erased shim forwards it.

`None` retains the current late check. It is not permission to violate the
requested descriptor, nor a workspace guarantee. A wrapper that omits
forwarding loses the early refusal optimization, not output correctness.
An operation must still compare actual output with a supplied commitment;
codec drift is a contract failure, not a fallback route.

The operation would use the commitment before `decode()`, refuse a known
mismatch, and retain the existing actual-output check. Exact names and refusal
reporting need native qualification before this becomes a trait addition.
In particular, the error must distinguish a pre-decode refusal from a mismatch
observed after pixel work; it must not imply that no earlier parsing occurred.

## Why existing probes are insufficient

`DecodeJob::output_info(data)` has no preferred-output argument and reports
`OutputInfo::native_format`. Native format is not the negotiated result.
Comparing a required output against it would wrongly refuse supported
conversions. A push sink's `begin` is also insufficient: some adapters reach it
only after reconstruction, and its return path does not preserve the complete
`DecodeOutput` metadata.

The reviewed still adapters hold the input, preferences and configuration in
the one-shot decoder and perform reconstruction in `decode()`. This makes the
retained decoder the candidate location. These are source observations, not
allocation or latency measurements.

| Harness pin | Native work needed to make a commitment | Qualification risk |
|---|---|---|
| [JPEG 8672e549](https://github.com/imazen/zenjpeg/blob/8672e549793a7e1b515f292bd9fd77109118cb83/zenjpeg/src/codec/decode.rs) | Share negotiation and final labeling across byte, float, CMYK and gain-map routes. Inspect late markers or return unknown. | Header hints and final source-color labeling currently occur at different sites. |
| [PNG 5422b6d2](https://github.com/imazen/zenpng/blob/5422b6d213e41f9764e7c4cf98e5c44eb8acbd85/src/codec.rs) | Resolve IHDR, transparency, color and descriptor conversion feasibility before pixel work. | Negotiation currently permits a failed conversion to fall back to native output. A commitment cannot promise that conversion without proving it is executable. |
| [TIFF in zenextras 1af38ae8](https://github.com/imazen/zenextras/blob/1af38ae83024830a141744fb3a565503bdc61af3/zentiff/src/codec.rs) | Share IFD-derived descriptor construction with actual decode; preferences are currently ignored by this adapter. | A separate probe helper must not drift from final labeling. |
| [JXL a76ac76d](https://github.com/imazen/zenjxl/blob/a76ac76d080295b754d970a3392eed8f964de490/src/codec.rs) | Expose negotiation from the native decoder and resolve any gain-map box affecting output. | Late boxes and reconstruction mode can change output; return unknown when unresolved. |
| [AVIF b25f7604](https://github.com/imazen/zenavif/tree/b25f7604fbb71ead6517ec27fa52646770e3606a/src/codec) | Derive the result from retained native container information, preferences and gain-map/grid choices. | Qualify gray opt-in and actual conversion feasibility; container item-copy cost is unmeasured. |

Incremental decoding keeps its existing Pending, Provisional and Final
interpretation states. Video stays per configuration epoch: native component
frames and requested packed conversion have different admission points. This
proposal does not allow a sequence header to certify every later frame.

## Tests before adoption

- Compare committed and actual descriptors for every supported preference list,
  including native fallback, transparency, word depth, color carriers, CMYK,
  float output and gain maps.
- Reuse exactly the decoder/input retained across the query and decode. Count
  native pixel work and measure allocation before early refusal.
- Exercise typed and erased paths, unknown commitments, native query errors,
  cancellation, and a deliberately dishonest commitment.
- Keep the old late-refusal control until an adapter supplies qualified
  commitments. Never substitute a native-format probe for negotiation.
- For each native adapter, require successful requested conversions to remain
  successful and bit-identical before and after adding the hook.
