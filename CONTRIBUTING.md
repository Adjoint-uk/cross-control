# Contributing to cross-control

Thank you for your interest in contributing to cross-control! This document provides guidelines for contributing.

## Getting Started

1. Fork the repository
2. Clone your fork: `git clone https://github.com/<you>/cross-control.git`
3. Create a branch: `git checkout -b my-feature`
4. Make your changes
5. Run checks: `cargo fmt --all && cargo clippy --workspace -- -D warnings && cargo test --workspace`
6. Push and open a pull request

## Development Requirements

- Rust 1.75+ (stable)
- Linux or Windows (macOS support planned)

## Code Standards

- **Format**: Run `cargo fmt --all` before committing
- **Lint**: `cargo clippy --workspace -- -D warnings` must pass
- **Test**: `cargo test --workspace` must pass
- **No unsafe code**: The workspace denies `unsafe_code`

## Architecture

The project is organised as a Cargo workspace with 8 crates. See `docs/architecture.md` for the full overview.

| Crate | Purpose |
|-------|---------|
| `cross-control-types` | Shared types (leaf crate, no platform code) |
| `cross-control-protocol` | QUIC transport and wire format |
| `cross-control-input` | Platform input capture and emulation |
| `cross-control-clipboard` | Clipboard synchronisation |
| `cross-control-discovery` | mDNS/DNS-SD zero-config |
| `cross-control-daemon` | Core state machine and event routing |
| `cross-control-cli` | User-facing binary |
| `cross-control-certgen` | TLS certificate generation |

## Pull Requests

- Keep PRs focused on a single change
- Write descriptive commit messages
- Add tests for new functionality
- Update documentation if behaviour changes
- Reference related issues in the PR description

## Reporting Issues

Use the GitHub issue templates for bug reports and feature requests.

## License

By contributing, you agree that your contributions will be licensed under the MIT OR Apache-2.0 license.
