# Development Guide

## Prerequisites

- Rust 1.75+ (stable): https://rustup.rs
- Linux or Windows

## Building

```bash
cargo build --workspace
```

## Testing

```bash
cargo test --workspace
```

## Linting

```bash
cargo fmt --all -- --check
cargo clippy --workspace -- -D warnings
```

## Project Structure

See `docs/architecture.md` for the workspace layout and crate dependencies.

## Adding a New Platform Backend

1. Create a new module in the relevant crate (e.g. `cross-control-input/src/wayland.rs`)
2. Implement the trait (`InputCapture` or `InputEmulation`)
3. Gate behind a Cargo feature flag
4. Add CI coverage for the new platform

## Running the CLI

```bash
# Debug build
cargo run -p cross-control-cli -- generate-cert --output /tmp/

# Release build
cargo run --release -p cross-control-cli -- start
```

## Useful Commands

```bash
# Check dependency licenses
cargo deny check licenses

# Check for known vulnerabilities
cargo deny check advisories

# Generate documentation
cargo doc --workspace --no-deps --open
```
