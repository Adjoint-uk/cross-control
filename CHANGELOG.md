# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0-alpha] — Phase 1 close-out

Two cross-control daemons on the same LAN now find each other automatically — no `address = "..."` required in the config. The mDNS advert carries the machine's TLS cert fingerprint so peers can pin before the first QUIC handshake.

### Added

- **mDNS / DNS-SD discovery backend** (`cross-control-discovery::MdnsDiscovery`) — advertises this daemon and browses for peers on `_cross-control._udp.local.`. TXT records carry `MachineId` (`id`) and SHA-256 cert fingerprint (`fp`) so discovered peers can be TOFU-verified before connection.
- **Composable discovery layer** (`DiscoveryAggregator`, `StaticDiscovery`) — multiple discovery backends share one peer stream into the daemon, with dedupe by `MachineId`. Static config peers and mDNS peers flow through the same path. Designed so cross-subnet rendezvous can slot in later without touching the daemon.
- **Daemon-level dial dedupe** — outbound connection attempts are keyed by `SocketAddr`, suppressing duplicate dials when a peer surfaces via more than one discovery backend.
- **`Daemon::set_local_fingerprint`** — wires the local TLS cert fingerprint into the mDNS advertisement; called automatically by the CLI from the loaded cert.
- **[ADR 0002 — TOFU certificate pinning](docs/adr/0002-tofu-pairing.md)** — documents the trust model, what TOFU does and does not protect against, key rotation story, and config layout.

### Changed

- `cross-control-daemon` now starts the discovery aggregator at run time instead of dialing static peers in a one-shot loop. Config-only setups still work unchanged — the static address list becomes a `StaticDiscovery` backend.
- `daemon.discovery = true` (the existing config flag) now actually enables mDNS instead of being a no-op.

### Fixed

- A handful of pre-existing clippy lints under newer toolchains (`similar_names`, `manual_let_else`, `match_wildcard_for_single_variants`, `doc_markdown`, `too_many_lines`, `needless_borrows_for_generic_args`, `ip_constant`). No behavioural changes — `cargo clippy --workspace --all-targets -- -D warnings` is clean again.

### Test coverage

- 64 tests pass workspace-wide (up from 58 at `v0.1.0-alpha.1`).
- 1 ignored test (`mdns_loopback::advertise_and_browse_round_trip`) — requires functional multicast on the host; run with `cargo test -p cross-control-discovery -- --ignored`.

## [0.1.0-alpha.1]

### Added

- Initial workspace scaffold with 8 crates
- Shared types: input events, device descriptors, screen geometry, machine identity, protocol messages
- Trait definitions: `InputCapture`, `InputEmulation`, `ClipboardProvider`, `Discovery`
- Wire format: length-prefixed bincode v2 encoding/decoding
- TLS certificate generation with SHA-256 fingerprinting
- CLI skeleton with `start`, `stop`, `status`, `generate-cert`, `pair` subcommands
- TOML configuration with sensible defaults
- CI pipeline (GitHub Actions): fmt, clippy, build, test on Linux + Windows
