# Fuzzing

Coverage-guided fuzz targets for the wire decoder, the code that handles
bytes from the network before a peer is authenticated.

```bash
cargo install cargo-fuzz          # once
cargo +nightly fuzz run decode_message -- -max_total_time=300
cargo +nightly fuzz run decode_control -- -max_total_time=300
```

| Target | What it checks |
|---|---|
| `decode_message` | Arbitrary frames decode to `Ok`/`Err` without panicking, aborting or over-allocating |
| `decode_control` | Same for the control stream, plus decode → encode → decode round-trips |

CI runs each target for a few minutes weekly (`.github/workflows/fuzz.yml`).
A fast deterministic variant runs on every build:
`decoder_survives_random_and_mutated_input` in `crates/cross-control-protocol/src/wire.rs`.

A crash leaves a reproducer in `fuzz/artifacts/<target>/`; replay it with
`cargo +nightly fuzz run <target> fuzz/artifacts/<target>/<file>`.
