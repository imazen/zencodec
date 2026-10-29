# Retained orientation plan qualification

Source: core fe7187fde3fb2d6b86cf04d0c3caa8546b45df3c;
native AVIF acd8d180891fa2c06e2b4c632838789bf9f3596b.
Isolated exports; core and native family path patches in the consumer harness.

Commands: cargo test --locked -p zencodec --lib; cargo check --locked -p
zencodec --example orientation; cargo test --locked -p zencodec --doc;
cargo clippy --locked -p zencodec --lib --example orientation -- -D warnings;
consumer cargo test -p zencodecs --no-default-features --features
std,avif-decode,avif-encode --test native_orientation; matching locked Clippy.

677 core tests; 26 passing doctests, 37 existing ignored; four native tests;
strict core and consumer Clippy passed. Native added matrix checks all eight
orientations in preserve/bake modes, plan/start equality, exact encoded-byte
parity with ordinary encoding, decoded stored dimensions and output orientation.
Core composition test checks coordinate mappings; contradictory facts refuse
before callback invocation.

Serialized run-heavy 3G/jobs2 timeout900: rc0, 8s, peak RSS 0.37GiB,
minimum available 11479MiB. This is validation cost, not a performance benchmark.
Initial native error-category constructor and test closure ownership failures
were retained; corrected without weakening assertions.

Missing: adversarial post-declaration changes, native declaration refusals,
other codecs, independent reader parity, native workspace bounds, broader
color/depth planning, portable lock resolution and updated generated reference.

Follow-up native 0a9eba30 (same library) adds changed width/height/descriptor,
unsupported color/U16 declaration, retry-after-refusal and repeated-start controls.
All six native tests and the unchanged core/doctest/Clippy gates pass: rc0,
13s, peak RSS 0.45GiB, minimum available 11451MiB. These controls supersede
the missing adversarial-declaration item above; exact-depth assertions and
late PixelSlice color attachment still need direct controls.
