# Controlled sink errors cannot be swallowed

Base 75913690; source candidate 77211667. A controlled adversarial decoder
adopts valid final interpretation, ignores a sink failure, attempts finish and
returns Done. The new control fails against the original implementation with
successful Done. It passes after guarding sink failures for begin, final rows,
provisional rows and finish. Each case requires the exact typed original cause,
one sink effect, terminal subsequent steps and no further backend call.

678 core tests, all zencodec-io feature tests/doctests, strict core/I/O all-target
Clippy and seven native JPEG negotiation/completion tests plus strict harness
Clippy pass. Existing assertions are unchanged. Native completion retains exact
pixels, metadata/extras and ownership-pointer checks.

The first post-fix attempt reused the pre-fix root-package executable in the
shared Cargo target (no compilation; panic line belonged to the pre-fix file).
That rc101 artifact is retained and is not qualification. The retry explicitly
removed only zencodec's regenerable package artifacts, then the log confirms
compilation from the after-export before executing the tests. Future before/after
root-package comparisons must use separate targets or force that rebuild.

Serialized run-heavy 3G/jobs2 timeout900 retry: rc0/20s, peak RSS0.68GiB,
minimum available13165MiB. No throughput, native memory-bound or cross-platform
claim follows from this local run. All source changes are private implementation;
no public signatures changed.
