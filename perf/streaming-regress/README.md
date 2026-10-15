# streaming-regress

Main-vs-PR interleaved benchmarks for the streaming-output rollout (zencodec
PR 149 and the codec PRs implementing it). Standalone: never built by the
workspace or CI.

```
./setup.sh                     # zencodec snapshot + shimmed main branches
STREAMING_REGRESS_PHOTO=photo.jpg cargo run --release -- --cpu-policy=unrestricted
python3 summarize.py run*.log  # markdown table
```

Private repos (zqi) need `CARGO_NET_GIT_FETCH_WITH_CLI=true`. Results:
`benchmarks/streaming_regress_2026-10-14.md`.
