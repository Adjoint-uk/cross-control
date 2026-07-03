# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added — Phase 2: live status readout ([#13]) and TOML layout validation ([#9])

- **`cross-control status` now shows live peers, latency, and focus.** The daemon writes a `StatusSnapshot` (peers with name/state/latency, plus which peer holds focus) to `cross-control.status.json` in the runtime dir every couple of seconds; the `status` command reads and renders it as a table. This is the CLI↔daemon channel the previous `status` lacked — it mirrors the PID-file pattern rather than standing up a full IPC socket. A richer query/subscribe channel can replace the file later without changing the rendered output.
- **Real latency, not a stub.** The daemon pings each peer on a 2-second cadence and records the round-trip time when the `Pong` returns (a new `LatencyTracker` per session). `status` shows `—` until the first ping completes, then `N ms`.
- **`Config::validate()` for screen layouts.** Loading a config now rejects layouts that would silently misroute the cursor: empty or duplicate screen names, a screen sharing this machine's `identity.name`, two screens on the same local edge, self-loop adjacency edges, one screen given two neighbors on the same edge, and `[[screen_adjacency]]` blocks that never connect back to this machine (a typo or dead island). Multi-hop screens introduced only via `[[screen_adjacency]]` are correctly accepted — reachability is checked to a fixpoint, not against `[[screens]]` alone.
- **Documented layout format.** `examples/config.toml` now explains `[[screens]]` vs `[[screen_adjacency]]`, the optional `address`/`fingerprint`, and gives a worked multi-hop example.

### Changed

- `DaemonEvent::SessionReady` now boxes its `PeerSession` payload (the session grew a latency tracker; boxing keeps the enum variants balanced).

### Test coverage

- 84 tests pass workspace-wide (was 70). New: `Config::validate` cases, `StatusSnapshot` JSON round-trip, and `LatencyTracker` ping/pong bookkeeping. Still 2 ignored (`mdns_loopback`, `arboard_backend` — both need host facilities unavailable in CI).

[#9]: https://github.com/Adjoint-uk/cross-control/issues/9
[#13]: https://github.com/Adjoint-uk/cross-control/issues/13

### Added — Phase 2 opener: clipboard text sync

- **`cross-control-clipboard` backends.** `ArboardClipboard` (default feature `arboard`) for the real system clipboard on X11, macOS, Windows, and wlroots-based Wayland; `MockClipboard` (feature `mock`) for tests and headless daemon runs. The trait now requires `Send + Sync + 'static` so the daemon can hold `&self.clipboard` across `.await` on a multi-thread runtime.
- **Daemon clipboard wiring.** On every control hand-off (controller side receives `EnterAck`), the controller sends `Clipboard::Offer`; the controlled side replies with `Clipboard::Request`; the controller answers with `Clipboard::Data`. The controlled side writes it to the local clipboard. Wired through a new `Daemon::set_clipboard` setter — leave it unset to run without clipboard sync (used by the integration tests that don't exercise it).
- **`ControlMessage::Clipboard(ClipboardMessage)` wire variant.** Clipboard traffic rides the existing control stream for now — small text payloads don't justify a third QUIC stream. A dedicated clipboard stream is queued for Phase 2.5 when image/HTML support lands.
- **CLI integration.** `cross-control start` constructs an `ArboardClipboard` at startup; missing display server (headless host) logs a warning and runs without sync rather than refusing to start.

### Known limitations (clipboard)

- Text-only (`ClipboardFormat::PlainText`). HTML and PNG are defined in the wire protocol and accepted by the trait but neither backend implements them yet.
- Wayland coverage is limited to wlroots-based compositors (Sway, Hyprland, river) via `arboard`. GNOME/KDE on Wayland use a different protocol — `wl-clipboard-rs` integration is the planned follow-up.
- One-shot sync at hand-off, not continuous. If the controller's clipboard changes *while* it is controlling the remote, the new content does not propagate until the next hand-off. Continuous sync via the watch API is a follow-up.

### Test coverage

- 70 tests pass workspace-wide (was 64 at `v0.2.0-alpha` code-complete). 5 new in `cross-control-clipboard::mock`, 1 new in `cross-control-daemon::tests::daemon_integration` (`test_clipboard_text_syncs_on_control_handoff`). Still 2 ignored (`mdns_loopback` requires multicast, `arboard_backend::tests::construct_and_set_get_text` requires a display server).

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
