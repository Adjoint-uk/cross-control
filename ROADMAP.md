# cross-control Roadmap

The road from `0.1.0-alpha.1` to `1.0.0`.

This document is the single source of truth for *what's next* and *what "done" means* at each milestone. The competitive reasoning behind cross-control's existence lives in [`docs/research-kvm-landscape.md`](docs/research-kvm-landscape.md); the architectural decisions live in [`docs/adr/`](docs/adr/). This file just orders the work.

Status legend: `[ ]` not started · `[~]` in progress · `[x]` done

---

## Phase 1 — "It actually works" → `v0.2.0-alpha`

**Success criterion:** two Linux laptops on the same LAN, daemon running on each, mouse crosses the screen edge, keystrokes land on the other machine. One demo, recorded as asciinema, linked from the README.

Until this works end-to-end, nothing else matters.

- [ ] QUIC session lifecycle: handshake → bidirectional input stream → graceful close
- [ ] evdev capture wired into the daemon (Linux)
- [ ] uinput emulation wired into the daemon (Linux)
- [ ] Edge-detection switching with one hardcoded two-machine layout
- [ ] mDNS discovery actually announces and finds peers on the LAN
- [ ] TOFU pairing flow that writes a pinned cert to disk and trusts it on reconnect
- [ ] CHANGELOG entry that says, in plain English, *"the daemon now moves a cursor between two machines"*
- [ ] asciinema demo committed under `docs/demos/`

## Phase 2 — "It's pleasant" → `v0.5.0-beta`

**Success criterion:** a developer can install it, follow `docs/setup-guide.md`, and use it as their daily driver between two Linux machines without hitting paper cuts.

- [ ] Clipboard sync — text
- [ ] Clipboard sync — HTML
- [ ] Clipboard sync — images (with size cap + streaming)
- [ ] Multi-monitor support per machine
- [ ] Configurable layouts in TOML (positions, hot corners, switch behaviour)
- [ ] Reconnect / 0-RTT resume after a network blip without losing the session
- [ ] systemd user service that just works (`systemctl --user enable --now cross-control`)
- [ ] Setup guide rewritten with screenshots and a 5-minute "first run" walkthrough
- [ ] `cross-control status` shows peers, latency, current focus

## Phase 3 — "It's portable" → `v0.9.0`

**Success criterion:** Linux ↔ Windows works end-to-end. Distro packages exist. Release binaries are signed.

- [ ] Gate Linux-only crates with `#[cfg(target_os = "linux")]` so Windows can build
- [ ] Windows input backend (SendInput / RawInput)
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
- [ ] ADR 0001 — QUIC as the only transport
- [ ] ADR 0002 — TOFU certificate pinning, no CA
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
