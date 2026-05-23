# cross-control Roadmap

The road from `0.1.0-alpha.1` to `1.0.0`.

> **Next up — Phase 2 opener:** implement the **clipboard text backend**. The `cross-control-clipboard` crate is trait-only; `arboard` (X11/macOS) and `wl-clipboard-rs` (Wayland) are mature crates. Steps: add deps → implement `ClipboardProvider` per platform → wire clipboard sync into the daemon's session lifecycle (sync on enter/leave). This is the first step toward `v0.5.0-beta` "it's pleasant."

This document is the single source of truth for *what's next* and *what "done" means* at each milestone. The competitive reasoning behind cross-control's existence lives in [`docs/research-kvm-landscape.md`](docs/research-kvm-landscape.md); the architectural decisions live in [`docs/adr/`](docs/adr/). This file just orders the work.

Status legend: `[ ]` not started · `[~]` in progress · `[x]` done

## Where we already are (audit, 2026-05-23)

The codebase is past the v0.2.0-alpha code milestone — only hardware bring-up and the demo remain.

- **64 tests passing** across the workspace (`cargo test --workspace`) + 1 ignored (`mdns_loopback`, needs multicast). CI green. Clippy clean under `-D warnings`.
- **`cross-control-types`** (994 LOC, 33 tests) — shared event/device/geometry types.
- **`cross-control-protocol`** (498 LOC) — QUIC connection, TLS, wire format, multi-stream control/input.
- **`cross-control-input`** (1370 LOC) — full Linux backend over evdev/uinput, plus a `mock` backend for tests.
- **`cross-control-daemon`** (~1700 LOC) — async handshake, edge-based barrier crossing, multi-hop adjacency routing, EnterAck flow, hotkey release, virtual device announce/map/destroy, ping/pong, entry-edge bounce suppression. Now also: discovery-driven outbound dials with per-address dedupe.
- **`cross-control-discovery`** (~500 LOC) — `Discovery` trait + `MdnsDiscovery` (mdns-sd, fingerprint TXT records) + `DiscoveryAggregator` (multi-backend fan-in with `MachineId` dedupe) + `StaticDiscovery` (wraps config peers).
- **`cross-control-cli`** (269 LOC) — `start`, `stop`, `status`, `generate-cert`, `pair`. Wires the cert fingerprint into the daemon for mDNS advertising.
- **`cross-control-certgen`** (125 LOC) — TLS cert generation + SHA-256 fingerprinting.
- **`cross-control-tui-test`** (884 LOC) — TUI harness for visual testing.

The remaining empty crate:
- **`cross-control-clipboard`** (29 LOC) — *trait only*, no backend. Phase 2's first task.

**Implication:** the v0.2.0-alpha *code* is done. What's left for the tag is bring-up on real hardware, a recorded asciinema demo, and pushing the tag.

---

## Phase 1 — "It actually works on two real machines" → `v0.2.0-alpha`

**Success criterion:** two Linux laptops on the same LAN find each other automatically, pair on first contact, and the cursor crosses between them. Recorded as asciinema, linked from the README.

- [x] **Implement `mdns-sd` backend in `cross-control-discovery`** — `MdnsDiscovery` + `DiscoveryAggregator` + `StaticDiscovery` shipped. Daemon now discovers peers without static `address` entries.
- [x] **CHANGELOG entry** for `[0.2.0-alpha]` describing the working end-to-end story in plain English.
- [x] **ADR 0002** — TOFU certificate pinning model written.
- [ ] **Real two-machine bring-up.** Requires two physical Linux boxes — hand-off task. Document every paper cut in `setup-guide.md`.
- [ ] **TOFU pairing UX polish.** The CLI `pair` subcommand exists and certgen works — verify the first-contact flow is sane (prompt, fingerprint, pin) and document it.
- [ ] **Edge-detection sanity pass.** `check_barrier_crossing` and multi-hop adjacency code are integration-tested but not yet validated with real cursor motion on real hardware.
- [ ] **asciinema demo** committed under `docs/demos/cursor-crossing.cast` and linked from README.
- [ ] **Tag and release** `v0.2.0-alpha` once the demo runs.

## Phase 2 — "It's pleasant" → `v0.5.0-beta`

**Success criterion:** a developer can install it, follow `docs/setup-guide.md`, and use it as their daily driver between two Linux machines without hitting paper cuts.

- [ ] Clipboard backend — text (`arboard` on X11/macOS, `wl-clipboard-rs` on Wayland)
- [ ] Clipboard backend — HTML
- [ ] Clipboard backend — images (with size cap + streaming)
- [ ] Multi-monitor support per machine (extend `ScreenGeometry` to a list)
- [ ] Configurable layouts in TOML (the adjacency map already exists in daemon — expose it cleanly in config)
- [ ] Reconnect / 0-RTT resume after a network blip without losing the session
- [ ] systemd user service that just works (`systemctl --user enable --now cross-control`) — unit file already exists in `systemd/`
- [ ] Setup guide rewritten with screenshots and a 5-minute "first run" walkthrough
- [ ] `cross-control status` shows peers, latency (ping/pong is implemented), current focus

## Phase 3 — "It's portable" → `v0.9.0`

**Success criterion:** Linux ↔ Windows works end-to-end. Distro packages exist. Release binaries are signed.

- [ ] Gate Linux-only crates with `#[cfg(target_os = "linux")]` so Windows can build (commit `44ef176` removed Windows from CI; this re-enables it cleanly)
- [ ] Windows input backend (SendInput / RawInput) in `cross-control-input`
- [ ] Re-add Windows to CI on every push
- [ ] macOS receive-side input backend (CGEventPost)
- [ ] Distro packaging: `.deb`
- [ ] Distro packaging: AUR
- [ ] Distro packaging: Homebrew tap
- [ ] Distro packaging: Flatpak
- [ ] Signed release binaries via cargo-dist (or equivalent), attached to GitHub Releases
- [ ] `cargo deny` enforced in CI on every push (license + advisory)

## Phase 4 — "It's v1.0" → `1.0.0`

**Success criterion:** the wire protocol is frozen, the security model is documented, and the project can make a SemVer commitment to its users.

- [ ] **Protocol freeze** — wire format and message types stable, with version negotiation for forward compatibility
- [x] ADR 0001 — QUIC as the only transport
- [x] ADR 0002 — TOFU certificate pinning, no CA
- [ ] ADR 0003 — Wayland-native via evdev/uinput, not XTest
- [ ] ADR 0004 — protocol freeze and version negotiation
- [ ] `docs/SECURITY-MODEL.md` — threat model, what TOFU does and does not protect against, key rotation story
- [ ] Fuzz testing on the protocol decoder (`cargo fuzz`)
- [ ] 3-machine star topology verified end-to-end in CI or a staging rig
- [ ] Project website with the asciinema demo above the fold
- [ ] Announcement plan — blog post, lobste.rs, Show HN, Bluesky/Mastodon

---

## Beyond v1.0 — speculative

Recorded so they don't get lost. Each becomes an ADR before any code lands.

- **Clinical positioning** — radiotherapy workstation use case (see research doc § *Hardware KVM-over-IP*). Permissive license + Rust + always-on TLS is the angle. Requires hardening, audit, possibly a paid support tier.
- **wlroots-enhanced backend** — beyond evdev/uinput, integrate with wlroots input-method protocols where available for richer per-compositor support.
- **File drag-and-drop** between machines, building on the clipboard infrastructure.
- **Touch and tablet event** forwarding (Wacom, etc.).
- **Mesh topology** — beyond star, peer-to-peer routing for large desk setups.
