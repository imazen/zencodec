# Zencodec contract review batch

Review [PR208](https://github.com/imazen/zencodec/pull/208) against main.
It retains the five implementation layers in commit order, with one complete
native qualification graph. The older layer PRs are superseded views of those
commits, not additional implementations to review or merge.

## Review order

1. **Current pixels, color and fidelity.** Current fallible pixel/plane APIs;
   gray ICC+CICP carriage; HLG synthesis refusal without a rendering target;
   authoritative ICC substitution only when identified safe; source-depth
   negotiation; distinct generation-A fidelity requests.
2. **Frame interchange.** Current ColorContext and alpha association accompany
   mapped sample encoding; geometry, regions and checked planar loans use the
   same validation at direct and erased boundaries. Original claims remain
   provenance, not a replacement for the current interpretation.
3. **Policy evidence.** Conversion offers retain their exact target and timing;
   bounded reports omit opaque payloads. Mandatory metadata/workspace rules and
   late interpretation decisions retain their existing refusal guarantees.
4. **Presentation.** Exact ordered timeline declarations and default-refusing
   mux admission. These signatures do not implement native MP4 edit playback.
5. **Conformance and ordinary use.** Shared frame/session helpers, real AV1/TIFF
   witnesses, complete native consumer qualification and a code-first guide.
   The homepage starts with CodecSet conveniences; advanced contracts live in
   their own page and namespace aliases. Rustdoc and the site default to dark.

[Implementation and measured qualification](core-contract-stack.md) separates
shared guarantees from backend coverage. I/O publication is prepared; publishing
still requires the owner's release approval and all release gates.

## Other pull requests

| Proposal | Disposition |
|---|---|
| PR201/204/205/206 | Their implementation commits are retained in PR208; review the complete batch once. |
| PR199 | Gray/source-depth predecessor is incorporated and extended by the current color baseline. |
| PR200 | No new runtime protocol; its source-history inventory and optional-Pulse documentation correction are retained. Its PNG incremental witness already runs in the complete JPEG/PNG harness. |
| PR189 | Publication manifest switch and release wording are incorporated; no crate is published. |
| PR188 | Timeline declarations are incorporated. Native playback experiments remain on the preserved proposal ref and in issue187; they are not a claim of finished playback. |
| PR157 | Independent opt-in C2PA structural audit and integration design. Not a signing implementation or a prerequisite for frame interchange. Review separately. |
| PR158 | Held for metadata-contract review. Conditional capability probes must not weaken checked retention; native HDR support alone does not prove complete CLL/MDCV retention. |
| PR196 | Separate optional IQA bridge. Its prior pinned graph is not proof of compatibility with the final frame batch; qualify that adapter against the accepted core/pixel pair before landing it. |

## Issue disposition

The batch does not close every product or codec roadmap item. The owning issues
retain these distinct scopes:

- **Current integration gate:**207 (standalone graphs),194 (old perf roots),198
  (canonical-plane API friction). The full selected runtime/lint gates pass; exact scope and receipts are
  recorded in core-contract-stack.md. These integration findings can close
  after the qualified batch lands.
- **Shared requirements already exercised, remaining rollout tracked:**173
  (mandatory requirements),172 (container timing/layout admission),170 (canvas
  rendering),169 (physical resolution),161 (input budgets),160 (fairness versus
  output ownership). Shared controls are not universal native guarantees.
- **Native behavior and codec qualification:**187 (edited playback),171 (TIFF
  old JPEG),166 (late metadata completeness/barriers),159 (per-format streaming
  reality),154 (color seams),132 (reader/default matrix),11 (decoder color
  wiring),202 (actual progress before EOF),203 (ready mux fragments).
- **Separate design or rollout work:**195 (scratch zeroing),134 (decompressed
  sub-inventories),131 (IMIJ model),122 (zenpipe consolidation),121 (Custom format
  sets),120 (calibration authority),104 and26 (achieved/per-codec fidelity),27
  (application checklist),24 (gain maps),12 (lossless modes),1 (paged decoding).

Those issues remain open unless their full stated acceptance criteria are
proved. None is silently reclassified as fixed to produce a green batch.
