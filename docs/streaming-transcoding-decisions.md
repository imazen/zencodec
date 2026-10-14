# Streaming Transcoding Architecture & Design Decisions

This document records the architectural analysis, design evolution, and operational models for high-performance streaming image transcoding, asynchronous runtime integration, and cooperative cancellation in the `zen` ecosystem.

---

## 1. Evolution of Architectural Models

### Model 1: The Monolithic Async / `spawn_blocking` Approach
In early designs, codecs were treated as monolithic blocks wrapped in Tokio's `tokio::task::spawn_blocking` or given `async fn` entry points.

* **The Failure of `async fn` inside SIMD Codecs:**
  - Putting `.await` points inside transform math (IDCT, wavelets, SIMD swizzles) destroys compiler auto-vectorization and loop unrolling by forcing register spills (AVX-512/NEON) to the heap on every yield.
  - Running compute-heavy codecs directly on Tokio worker threads causes **event loop starvation**: a 20 ms decode freezes network I/O, timers, and TLS handshakes across peer connections assigned to that thread.
* **The Failure of Fine-Grained `spawn_blocking`:**
  - Hopping between Tokio worker threads and Tokio's blocking thread pool costs 2–5 µs per switch and invalidates CPU L1/L2 caches.
  - Under high concurrency (e.g. 1,000 req/sec), Tokio's blocking pool expands up to its default limit (512 threads), causing OS thread contention, CPU context switching thrashing, and out-of-memory (OOM) failures.

### Model 2: The Pure Manual Sans-I/O State Machine
Following the strict Sans-I/O model (e.g. Firezone's network protocol pattern, `quinn-proto`, `str0m`), the decoder performs zero I/O and is authored entirely as a synchronous state machine (`handle_input`, `poll_output`).

* **The Strength:** 100% deterministic, zero async runtime lock-in, fully testable in-memory, portable to WASM and C# FFI.
* **The Critical Flaw:** Authoring multi-stage container parsers (TIFF IFD chains, JXL box trees, ISOBMFF box navigation, progressive restart markers) as manual enum state machines is notoriously verbose, tedious, and bug-prone. As Firezone noted in their evaluation, sequential workflows become painful to model manually.

### Model 3: Compiler-Synthesized Coroutines (`zencodec::coroutine`)
To solve Model 2's authoring ergonomics without inheriting Model 1's runtime contagion, `zencodec` repurposes Rust's `async`/`await` syntax strictly as a **compile-time coroutine synthesizer**.

* **Authoring View:** Codec developers write natural, straight-line procedural code:
  ```rust
  let header = ctx.read_exact_at(0, 16).await?;
  for y in 0..height {
      let strip = decode_strip(y);
      ctx.yield_scanlines(y, &strip).await?;
  }
  ```
* **Compiler Synthesis:** `rustc` compiles this into an allocation-free state machine with resume points and saved stack variables.
* **Caller View:** `CoroutineDecoder` wraps the future and exposes a 100% synchronous, pure Sans-I/O [`IncrementalDecode`](../src/incremental.rs) interface (`push_range`, `pull_batch_into`, `needs_input`).
* **Runtime Dependency:** Exactly zero. No Tokio, no `block_on`, no thread pools.

### Model 4: The Decoupled Bounded Actor Pipeline (Streaming Transcoder)
For production image proxy servers, the combination of Sans-I/O codecs with bounded channels yields the optimal system topology:

```
[TOKIO ASYNC RUNTIME]                                  [DEDICATED CPU WORKER POOL]
(Network Multiplexing & Sockets)                       (SIMD Math, Zero Async, Sized to Cores)

Inbound HTTP Body (AsyncRead)
         │
         │  read().await
         ▼
[Async Ingest Task]
         │
         │  in_tx.send(chunk).await ────(Bounded Channel)───► [Sync Transcode Thread]
                                                                  │
                                                                  │ 1. decoder.push_range(chunk)
                                                                  │ 2. decoder.pull_batch_into(&mut strip)
                                                                  │ 3. resizer.push_strip(&strip)
                                                                  │ 4. resizer.pull_strip(&mut out_strip)
                                                                  │ 5. encoder.push_strip(&out_strip)
                                                                  │
Outbound HTTP Body (AsyncWrite) ◄───(Bounded Channel)──────── out_tx.send(compressed_bytes)
         ▲
         │  write_all().await
[Async Egress Task]
```

* **Cache Locality:** The transcode loop (Decode Strip → Resize Strip → Encode Strip) runs synchronously inside CPU L1/L2 cache.
* **Memory Boundedness:** Working memory is bounded to $(2R + 1)$ scanlines (where $R$ is the vertical resizer radius), preventing the OOM cliff under concurrency.
* **Automatic Backpressure:** Bounded channels naturally throttle the inbound TCP stream when the outbound socket or CPU pool slows down.

---

## 2. Immediate Client Disconnects via `&dyn Stop` and `how_far::Pulse`

When an HTTP client aborts (e.g. user navigates away or closes the browser tab), spending further CPU cycles transcoding an abandoned image wastes server capacity.

Disconnection handling operates through two complementary mechanisms: **reactive channel breaking** and **active cooperative polling**.

```
[Client Drops Connection]
         │
         ├────────────────────────────────────────┐
         ▼                                        ▼
[Reactive Channel Break]                [Active Cooperative Token]
Tokio drops `out_rx`                    Async task trips `stop_trigger.stop()`
         │                                        │
         ▼                                        ▼
`out_tx.send()` fails immediately       `stop.check()?` fails at next strip boundary
         │                                        │
         └──────────────────┬─────────────────────┘
                            ▼
               [Worker Thread Aborts Immediately]
               (Frees CPU cores in < 200 microseconds)
```

### 1. Reactive Channel Break (Zero-Polling Circuit Breaker)
If the worker thread is in the middle of sending an encoded strip or waiting on backpressure:
- Tokio detects client socket disconnection when `socket.write()` returns `BrokenPipe` or `ConnectionReset`.
- Tokio drops the outbound receiver `out_rx`.
- The synchronous worker thread's next call to `out_tx.send(bytes)` immediately returns `Err(SendError)`.
- The worker terminates the pipeline instantly with zero polling overhead.

### 2. Active Cooperative Polling (`&dyn Stop` / `how_far::Pulse`)
If the worker is actively computing SIMD transforms (IDCT or loop filtering):
- The async task holds a `StopTrigger` (or `AtomicBool`) and passes it into the pipeline as an `Option<&dyn Stop>`.
- `how_far::Pulse` implements `enough::Stop` (`Pulse: Stop`), allowing progress measurement and cancellation to share a single token.
- At every strip boundary (e.g. every 8 or 16 scanlines, taking ~50–150 µs of CPU), the loop executes:
  ```rust
  if let Some(stop) = stop {
      stop.check()?; // Returns Err(StopReason::Cancelled) if client dropped
  }
  ```
- Because strips are fine-grained, the latency to abort compute upon client disconnect is **< 200 microseconds**, freeing the CPU core immediately.

---

## 3. Deadline Estimation (<30s) and Adaptive Fallback

In edge proxies, gateways enforce hard request timeouts (typically 15s to 30s). Completing an expensive transcode in 31 seconds is worse than failing in 50 milliseconds, as it consumes 30 seconds of CPU for a response that the gateway will reject with HTTP 504.

### Phase 1: Pre-flight Heuristic Check (Zero Compute)
Immediately upon parsing the image header (dimensions $W \times H$, bit depth, number of channels):
```rust
let pixel_count = width as u64 * height as u64;
let estimated_mega_pixels = pixel_count as f64 / 1_000_000.0;

if estimated_mega_pixels > MAX_ALLOWED_MEGAPIXELS {
    return Err(CodecError::ResourceLimitExceeded("Image dimensions exceed deadline budget"));
}
```

### Phase 2: Dynamic In-Flight Rate Estimation
Network conditions, server load, and image entropy vary dynamically. The pipeline uses `how_far::Pulse` to project completion time during active processing:

$$\text{Fraction Done } F = \frac{\text{Scanlines Processed}}{\text{Total Height}}$$
$$T_{\text{projected}} = \frac{T_{\text{elapsed}}}{F}$$

After processing an initial warm-up batch (e.g. 5% of scanlines or first 32 rows):
1. **Compute Projected Time:**
   ```rust
   let elapsed = start_time.elapsed();
   let fraction = pulse.fraction().unwrap_or(y as f64 / height as f64);
   
   if fraction > 0.05 {
       let projected_total = elapsed.div_f64(fraction);
       let remaining_time_budget = deadline.saturating_duration_since(Instant::now());
       
       if projected_total > remaining_time_budget {
           // We will not make the deadline!
           handle_deadline_breach(pipeline, remaining_time_budget);
       }
   }
   ```

2. **Mitigation Strategies on Projected Deadline Breach:**
   * **Strategy A: Early Hard Drop**
     If the image is non-essential or cannot be simplified, immediately terminate the transcode and return HTTP 422 or 504. This prevents wasting the remaining 28 seconds of CPU time.
   * **Strategy B: Dynamic Encoder Downgrade**
     Encoding compute dominates transcoding time (e.g. AVIF slow presets or JXL effort 7 can take 10x–50x longer than decode).
     Because the strip pipeline is modular, the pipeline can adaptively downgrade the output encoder on the fly:
     - Drop JPEG XL effort from 7 to 3.
     - Drop AVIF speed preset from 4 to 8.
     - Switch encoder entirely to high-speed WebP or ZenJPEG (which encodes in tens of milliseconds).

---

## 4. Contract Stability Summary

| Tier | Contract | Role | Mutability |
|---|---|---|---|
| **Core Codec** | [`IncrementalDecode`](../src/incremental.rs) | Pure synchronous Sans-I/O state machine. Zero runtime dependencies. | **Frozen / Stable** |
| **Seekable Source** | [`ReadAt`](../src/source.rs) / [`SeekableDecode`](../src/source.rs) | Random-access pull interface matching `imij`. | **Frozen / Stable** |
| **Authoring Layer** | [`CoroutineDecoder`](../src/coroutine.rs) | Synthesizes state machines from linear `async`/`await` code at compile time. | **Internal Utility** |
| **Concurrency Bridge** | Bounded Channel Actor | Bridges Tokio async sockets to synchronous SIMD worker threads with cooperative cancellation. | **Orchestrator Layer** |

---

## 5. Architectural Retrospective: Mistakes Avoided & Blind Spots Resolved

Looking at this architecture through the lens of **mistakes identified and avoided** by peer review, the user, and our technical evaluations reveals why earlier prototypes had to be abandoned:

### Mistake 1: Conflating Whole-File Security Audits with Progressive Streaming (The "Inventory" Trap)
* **The Initial Design:** Relying on `zencodec::inventory::Inventory` to parse and validate 100% of bytes (0..len) upfront before decoding, guaranteeing no unparsed polyglots or hidden payloads.
* **The Mistake:** In cloud/network streaming (e.g. S3 range requests on a 10 GB TIFF or a progressive JXL stream), the client does not possess the whole file. Demanding a full inventory upfront completely broke progressive cloud decoding.
* **The Correction:** Decoupled post-facto security audits (`Inventory`) from progressive multi-stage range handshakes ([`IncrementalDecode`](../src/incremental.rs)).

### Mistake 2: The "Function Coloring" Async Runtime Lock-In
* **The Initial Instinct:** Declaring `async fn decode()` or `async trait` inside the codec core to allow decoders to fetch byte ranges directly.
* **The Mistake:** Hard-coded the codec to an async runtime (Tokio vs async-std). Contaminated pure SIMD math with async polling machinery, broke WASM without asyncify, broke C# FFI ([`imageflow-dotnet`](https://github.com/imazen/imageflow-dotnet)), and made `no_std` support impossible.
* **The Correction:** Kept the codec core 100% synchronous Sans-I/O (`push_range`, `pull_batch_into`). Decoupled the codec from all I/O and runtime dependencies.

### Mistake 3: The `spawn_blocking` Fallacy (CPU Math on Async Runtimes)
* **The Initial Instinct:** Calling `tokio::task::spawn_blocking` for fine-grained scanline or chunk decoding.
* **The Mistake:** Context-switch ping-pong (2–5 µs per hop) destroyed throughput. Cache eviction degraded vector throughput. Under load, Tokio's blocking pool expanded up to 512 threads, causing OS context thrashing and OOM. Running compute directly on Tokio workers froze network epoll loops.
* **The Correction:** Decoupled Bounded Actor Pipeline. Dedicated worker threads sized strictly to physical cores (`num_cpus - 2`) run synchronous SIMD loops in L1/L2 cache; Tokio tasks only handle network I/O and channels.

### Mistake 4: Hand-Written State Machine Explosion (The Firezone Critique)
* **The Initial Pitfall:** Adopting strict Sans-I/O forced handwriting complex enum state machines for multi-stage binary formats (JXL box trees, TIFF IFD pointer chains, ISOBMFF boxes, MCU restart markers).
* **The Mistake:** As Firezone documented in their blog post, manually authoring sequential state machines becomes verbose, tedious, and fragile.
* **The Correction:** Repurposed Rust's `async`/`await` strictly as a **compile-time coroutine synthesizer** ([`CoroutineDecoder`](../src/coroutine.rs)). The compiler synthesizes the tedious state machine; the outside world sees a pure synchronous Sans-I/O interface.

### Mistake 5: "Push-Only" Dogmatism (Ignoring Naturally Seekable Sources)
* **The Initial Assumption (PR #143):** Treating push (`NeedRanges`) as the sole abstraction for all range decoding.
* **The Mistake Identified by User & `imij`:** When reading from an already-seekable source (local file, mmap slice, or S3 client), forcing straight-line directory-walking codecs (like TIFF or `imij`) into an inverted push state machine is backwards and adds unnecessary friction.
* **The Correction:** Added the first-class [`ReadAt`](../src/source.rs) pull tier (PR #145), mirroring `imij`, with zero-overhead adapters (`drive_ranges`, `BufferedReadAt`, `SeekableIncrementalAdapter`) bridging push and pull.

### Mistake 6: Unbounded Buffering When Simulating Seeks (Silent Memory Bloat)
* **The Pitfall:** When an unseekable stream needs seeking, naively buffering bytes in an unbounded `Vec<u8>`.
* **The Mistake:** A malicious or pathological file with an EOF directory pointer at byte 10 GB causes silent buffering of 10 GB into RAM, crashing the server.
* **The Correction:** `BufferedReadAt` enforces explicit policies (`Window`, `Cache`, `Spool`) with hard memory caps (`ResourceLimits`). Exceeding caps or reading past a window fails loudly with `SourceError::BeyondLimit` or `BeyondWindow`—never silent buffer growth.

### Mistake 7: Full-Image Materialization in Proxy Pipelines (The OOM Cliff)
* **The Legacy Trap:** Decoding the entire image into an RGB buffer, passing it to the resizer, and then passing the resized image to the encoder.
* **The Mistake:** 1,000 concurrent 4K/8K requests require >100 GB of RAM, causing server crashes under traffic spikes.
* **The Correction:** Strip-by-strip sliding-window streaming (`pull_batch_into` feeding resizer ring buffers). Memory is bounded to $(2R + 1)$ scanlines (~1-2 MB per request) regardless of total image megapixel size.

### Mistake 8: The "Late Timeout" Waste (Burning 30s of CPU on Doomed Requests)
* **The Trap:** Allowing a heavy transcode (e.g. 8K AVIF preset 4 or JXL effort 7) to run for 31 seconds before the HTTP gateway drops it at 30 seconds.
* **The Mistake:** The server burned 30 seconds of expensive SIMD compute only to return HTTP 504.
* **The Correction:** Dynamic rate estimation via `how_far::Pulse`. After 5% of scanlines, calculate $T_{\text{projected}} = \frac{T_{\text{elapsed}}}{F}$. If $T_{\text{projected}} > \text{Deadline}$, immediately hard-abort (saving 28s of CPU) or adaptively downgrade encoder effort.

### Mistake 9: Slow / Inefficient Disconnection Handling
* **The Trap:** Relying solely on coarse timeouts or checking cancellation only at the end of the full image.
* **The Mistake:** Worker threads continue crunching CPU on abandoned requests for seconds after the client has closed the connection.
* **The Correction:** Dual-vector disconnection: Reactive channel breaking (`out_tx.send()` fails instantly when Tokio drops `out_rx`) plus active cooperative checking (`stop.check()?` at strip boundaries every ~100 µs).
