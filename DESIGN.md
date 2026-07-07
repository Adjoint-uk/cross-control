# cross-control Design & Direction

The road from `0.1.0-alpha.1` to `1.0.0` — *why* the project exists, *what
"done" means* at each milestone, and the architectural decisions behind it.

> **The live task list is [GitHub Issues](https://github.com/Adjoint-uk/cross-control/issues), grouped by [milestone](https://github.com/Adjoint-uk/cross-control/milestones).** This document is the narrative, not a checklist — it does not track status, so it can't go stale. Issues are the checkboxes; this is the story. The competitive reasoning lives in [`docs/research-kvm-landscape.md`](docs/research-kvm-landscape.md); the decisions in [`docs/adr/`](docs/adr/).

## Where we already are (audit, 2026-05-23)

The codebase is past the v0.2.0-alpha *code* milestone — only hardware bring-up and the demo remain.

- **64 tests passing** across the workspace (`cargo test --workspace`) + 1 ignored (`mdns_loopback`, needs multicast). CI green. Clippy clean under `-D warnings`.
- **`cross-control-types`** (994 LOC, 33 tests) — shared event/device/geometry types.
- **`cross-control-protocol`** (498 LOC) — QUIC connection, TLS, wire format, multi-stream control/input.
- **`cross-control-input`** (1370 LOC) — full Linux backend over evdev/uinput, plus a `mock` backend for tests.
- **`cross-control-daemon`** (~1700 LOC) — async handshake, edge-based barrier crossing, multi-hop adjacency routing, EnterAck flow, hotkey release, virtual device announce/map/destroy, ping/pong, entry-edge bounce suppression, discovery-driven outbound dials with per-address dedupe.
- **`cross-control-discovery`** (~500 LOC) — `Discovery` trait + `MdnsDiscovery` (mdns-sd, fingerprint TXT records) + `DiscoveryAggregator` (multi-backend fan-in with `MachineId` dedupe) + `StaticDiscovery` (wraps config peers).
- **`cross-control-cli`** (269 LOC) — `start`, `stop`, `status`, `generate-cert`, `pair`.
- **`cross-control-certgen`** (125 LOC) — TLS cert generation + SHA-256 fingerprinting.
- **`cross-control-clipboard`** — text, HTML, and PNG image backends shipped over `arboard`, with a per-hand-off size cap. Chunked streaming of large images remains.
- **`cross-control-tui-test`** (884 LOC) — TUI harness for visual testing.

---

## The milestones — what "done" means at each

### Phase 1 — "It actually works on two real machines" → `v0.2.0-alpha`

Two Linux laptops on the same LAN find each other automatically, pair on first contact, and the cursor crosses between them. Recorded as asciinema, linked from the README. The *code* is done; what's left is real-hardware bring-up, the demo, and the tag.

### Phase 2 — "It's pleasant" → `v0.5.0-beta`

A developer can install it, follow `docs/setup-guide.md`, and use it as a daily driver between two Linux machines without paper cuts. Clipboard (text shipped; HTML/images next), multi-monitor, TOML layouts, reconnect/0-RTT resume, a systemd user service, and a `status` that shows peers/latency/focus.

### Phase 3 — "It's portable" → `v0.9.0`

Linux ↔ Windows works end-to-end. A Windows input backend (SendInput/RawInput), macOS receive-side (CGEventPost), Windows back in CI, distro packaging (`.deb`/AUR/Homebrew/Flatpak), `cargo deny` in CI, and signed release binaries.

### Phase 4 — "It's v1.0" → `1.0.0`

The wire protocol is frozen with version negotiation, the security model is documented, and the project can make a SemVer commitment. Fuzzing on the protocol decoder, a verified 3-machine star topology, a project website, and an announcement plan.

**ADRs:** 0001 QUIC-only transport (done) · 0002 TOFU cert pinning, no CA (done) · 0003 Wayland-native via evdev/uinput, not XTest · 0004 protocol freeze + version negotiation.

---

## Beyond v1.0 — speculative

Recorded so they don't get lost. Each becomes an ADR before any code lands.

- **Clinical positioning** — radiotherapy workstation use case (see research doc § *Hardware KVM-over-IP*). Permissive license + Rust + always-on TLS is the angle. Requires hardening, audit, possibly a paid support tier.
- **wlroots-enhanced backend** — integrate wlroots input-method protocols where available for richer per-compositor support.
- **File drag-and-drop** between machines, building on the clipboard infrastructure.
- **Touch and tablet event** forwarding (Wacom, etc.).
- **Mesh topology** — beyond star, peer-to-peer routing for large desk setups.
