# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added — macOS input backend (preview)

- **Native macOS backend** (`[input] backend = "auto"` or `"macos"`):
  - capture uses a Quartz event tap on its own run-loop thread, swallowing local input while you control another machine. The cursor is detached from the mouse then, so motion keeps arriving as deltas; when you control this Mac, the real cursor position is reported, for exact edge crossing.
  - emulation uses `CGEventPost` with layout-independent virtual key codes. Held modifiers' flags go on every event, motion with a button held is sent as "dragged", double clicks are counted, and smooth scrolling keeps its fractional deltas.
  - events the daemon posts carry a marker its own tap ignores, so being controlled never echoes back. A tap that macOS disables (after a slow callback or secure input) is re-enabled.
- **Keymap** (`mac_map`): pure code, unit-tested on every platform. It covers every named key, PC-only keys (Insert as Help, Num Lock as Clear, Print Screen / Scroll Lock / Pause as F13–F15), left/right modifier flags, buttons and scroll signs.
- **`doctor` on macOS** reports the Input Monitoring and Accessibility permissions, with the exact System Settings path to fix them. The first start shows the system prompts.
- **CI builds and tests on macOS.** Tests that need the desktop skip when the runner lacks the permissions.
- **[docs/macos.md](docs/macos.md)** covers how it works, permissions, setup and limits.

### Fixed

- **`stop` could wait for a daemon that had already exited, on systems without `/proc` (macOS).** A zombie (an exited child not yet reaped) counted as running there; the check now uses `ps` and ignores zombies.
- **A too-long control-socket path now gives a clear error.** macOS limits Unix socket paths to 104 bytes, and its temp directory is long. The error suggests a shorter `CROSS_CONTROL_RUNTIME_DIR`.

### Added

- **Releases can be published from the Actions tab.** Run the Release workflow on `main` with "publish" ticked; it creates the `vX.Y.Z` tag for the version in the code. This suits environments that cannot push tags.

## [0.2.0] - 2026-11-21

The first release with ready-built downloads, for Linux (x86_64, aarch64) and Windows (x86_64).

**Highlights**

- **Windows support (preview).** The native input backend captures input with low-level hooks and injects it with `SendInput`. It starts at logon through a Task Scheduler task (`cross-control service install`), with no console window. CI tests it end to end on a real Windows desktop.
- **Secure by default.** Connections use mutual TLS with verified handshake signatures. Peers are refused unless explicitly trusted, through `cross-control pair`, `trust add`, or a pinned fingerprint.
- **Built for automation.**
  - Every command supports `--json`, exit codes are stable, and `config check` and `doctor` work for scripting.
  - Deployment is covered by an Ansible role for Linux, a PowerShell module for Windows, and a remoting example for fleets.
  - The headless backend makes CI and smoke tests possible.
- **Clipboard sync** (text, HTML, images) on hand-off, **multi-monitor layouts**, mDNS discovery, and live `status`.
- **Install scripts** for both platforms verify downloads against `SHA256SUMS`. Every archive has signed build provenance.

**Upgrading from 0.1.0-alpha.1:** peers are now refused unless trusted. Pair your machines once (`cross-control pair <address>` on each side), or pin fingerprints in `config.toml`. See [Security — mutual authentication and explicit peer trust](#security--mutual-authentication-and-explicit-peer-trust) below.

### Added — release pipeline

- **Releases for Linux (x86_64, aarch64) and Windows (x86_64)** are built and published when a `vX.Y.Z` tag is pushed:
  - archives contain the binaries, the docs and an example config, and the Windows zip adds `cross-controlw.exe` and the PowerShell module;
  - each release has `SHA256SUMS` and signed build provenance attestations;
  - its notes come from this file.
- **The release workflow checks before publishing**: the tag must match the version in the code, and every archive is smoke-tested by running it, installing it with the install scripts from a local mirror, and loading the bundled module. PRs that touch the release machinery get the same dry run without publishing.
- **`install.ps1`** installs on Windows (`irm … | iex`) with the PowerShell module.
- **`install.sh` rewritten**:
  - it downloads the release archive and verifies it against `SHA256SUMS`;
  - it installs pre-releases too;
  - it runs `cross-control doctor`, and leaves the systemd unit to `cross-control service install`;
  - `CROSS_CONTROL_VERSION`, `CROSS_CONTROL_INSTALL_DIR` and `CROSS_CONTROL_DOWNLOAD_BASE` (an internal mirror) choose what and where.
- **`scripts/release/release.py`** bumps the version everywhere it appears, checks they agree, extracts release notes and builds archives. See [docs/releasing.md](docs/releasing.md).

### Fixed

- **`install.sh` downloaded a file no release had ever published** (a bare `cross-control-<target>` binary), and the only published binary was Linux x86_64.

### Added — PowerShell module

- **`CrossControl` module** (`deploy/powershell`): idempotent functions over the CLI's `--json` interface. Each reports `Changed`, and those that make changes support `-WhatIf`.
  - It installs the binaries, restarting a running daemon around the copy.
  - It writes `config.toml`, validated before it replaces the current file.
  - It manages trust, either one machine at a time or authoritatively.
  - It starts, stops and restarts the daemon, and manages the logon task.
- **Fleet deployment over PowerShell remoting** (`examples/Deploy-CrossControl.ps1`), the counterpart of the Ansible role.
- **CI runs the module's end-to-end test** on Windows PowerShell 5.1, and on PowerShell 7 on Windows and Linux. The test deploys two machines that must connect to each other.

### Added — clean stop everywhere; start at logon on Windows

- **Local control channel.** The daemon listens on a Unix socket in the runtime directory, or on Windows a named pipe that refuses network clients.
  - `cross-control stop` uses it to ask the daemon to shut down cleanly, including on Windows, which has no SIGTERM. `stop` waits until the process has exited and reports `"graceful"` with `--json`. Only an unresponsive daemon is terminated.
  - It is also the single-instance lock: a second `start` fails with "already running".
- **`service install` on Windows** registers a Task Scheduler task that runs at logon in the user's desktop session. A Windows service would run in session 0, where it can neither capture nor inject input. `service unit` prints the task XML; `service uninstall` stops the daemon and removes the task.
- **`cross-controlw`** runs the daemon without a console window, for the logon task:
  - it logs to `cross-control.log` in the runtime directory;
  - it restarts the daemon on failure, but not on configuration errors;
  - `--env NAME=VALUE` passes directory overrides through.
- **CI registers, starts, stops and removes the real logon task** on the Windows runner (`deploy/windows/tests/logon-task.ps1`).

### Fixed

- **`stop` and `status` counted a zombie process as running.** On Linux, a daemon that had exited but not yet been reaped by its parent still looked alive.
- **Log files no longer get colour codes** when stderr is not a terminal.

### Added — automated end-to-end testing on real Windows

- **`windows_e2e` test** runs on CI's Windows runner:
  - it uses real low-level hooks on one daemon and real `SendInput` on another, connected over QUIC with mutual TLS;
  - it checks crossing at the real screen edge, mouse motion replayed by the receiver, and a captured key pressed system-wide by the receiver;
  - it checks that the release hotkey takes control back and that no key is left pressed.
- **The `test-support` feature** (tests only) lets the hooks accept synthetic input tagged with a marker. The daemon's own injections are untagged and stay ignored.
- **On CI's Windows job, tests that need a desktop fail instead of skipping** (`CROSS_CONTROL_REQUIRE_DESKTOP`).

### Fixed

- **Windows: crossing could be impossible.** Capture turned cursor positions into relative deltas. Windows clamps the cursor at screen edges, so pushing against an edge gave a delta of zero. Once the daemon's tracked cursor was out of step with the real one (it was from the very first movement), it never reached the edge. When not controlling another machine, the backend now reports the real cursor's absolute position, which the daemon maps onto its layout and never forwards to peers.

### Added — Windows input backend (preview)

- **Native Windows backend** (`[input] backend = "auto"` or `"windows"`):
  - capture uses low-level keyboard and mouse hooks on a dedicated thread;
  - local input is swallowed while you control another machine, and the cursor is parked mid-screen so motion never stalls at an edge;
  - emulation uses `SendInput` with layout-independent scan codes;
  - input the daemon injects itself is ignored, so being controlled never echoes back;
  - the process is per-monitor DPI aware.
- The key and mouse translation (`win32_map`) is pure code, unit-tested on every platform. It covers every named key, the Pause/Num Lock scan-code collision, the fake Ctrl that AltGr sends, unknown keys, extra mouse buttons, and the wheel in both axes.
- Windows-only tests check that the hooks install and shut down, that injected input is not captured, and that injected motion moves the cursor. On a runner without a desktop they skip instead of failing.
- `doctor` reports the Windows backend and its limits (no access to elevated windows or the secure desktop).
- `status` and `stop` now work on Windows. Checking whether the daemon is running no longer depends on Linux's `/proc`.
- [docs/windows.md](docs/windows.md) covers how it works, setup and limits.

### Changed — the whole workspace builds and tests on Windows

- **The Linux input backend now compiles only on Linux targets.** Before, `evdev` was gated only by the `linux` feature, which is on by default in the CLI, so the CLI could not compile for Windows at all, and Windows CI excluded it. CI now builds and tests every crate on Windows. On Windows the daemon runs with `[input] backend = "none"` until a native backend lands; any other setting gives a clear configuration error.
- **New `CROSS_CONTROL_CONFIG_DIR` and `CROSS_CONTROL_RUNTIME_DIR` overrides** for the configuration and runtime directories, on every platform. `service install` writes them into the unit. The Ansible role uses them instead of an `XDG_CONFIG_HOME` workaround, so `cross_control_config_dir` can be any path.

### Added — bring-up tooling

- **`cross-control doctor`** checks whether a machine is ready to run cross-control, and prints the exact fix for each problem. With `--json` it gives machine-readable results, and it exits 1 if any check fails. It checks:
  - readable input devices, and the keyboards and mice detected;
  - write access to `/dev/uinput`;
  - the configuration file and this machine's identity;
  - neighbours in the layout that aren't trusted;
  - whether the UDP port is free;
  - clipboard access and the session type.
- **`docs/hardware-test.md`** is a step-by-step checklist for the first test on two physical machines, and lists what to send back when something fails.

### Security

- The Linux emulation backend no longer logs injected events at `debug`, because they are keystrokes and could include passwords. They are now logged only at `trace`.

### Added — Ansible role and headless mode

- **Ansible role** (`deploy/ansible/roles/cross_control`):
  - installs the binary and creates each host's identity;
  - renders a validated `config.toml` from variables;
  - renders an authoritative `known_peers` from the other hosts' fingerprints, so there is no manual pairing, and hosts dropped from the play lose trust;
  - manages the systemd user service.
- **Tested end to end in CI** (`deploy/ansible/tests/run.sh`): two machines are deployed, a re-run must report no changes, the daemons must connect using only the distributed trust, and removing a host must revoke its trust.
- **`[input] backend = "auto" | "evdev" | "none"`.** `none` runs the daemon headless (discovery, authentication, sessions, clipboard, without touching input devices), for smoke-testing deployments in containers and CI. Non-Linux builds now fail with a clear configuration error unless `none` is chosen.

### Added — automation-friendly CLI

- **`--json` and `--config` on every command.** With `--json`, stdout is a single JSON document; logs now always go to stderr, and one-shot commands log only warnings by default.
- **Documented exit codes:** 3 = daemon not running (`status`, `stop`), 4 = invalid or missing configuration, 5 = peer not authorized.
- **`cross-control identity`** prints this machine's name, fingerprint, machine id and file locations, creating the identity on first use; `created` reports whether anything was generated.
- **`cross-control config check [PATH]`** validates a configuration file without starting anything (exit 4 if invalid), e.g. for Ansible's `validate:`.
- **`cross-control service unit | install [--enable] | uninstall`** generates and manages a systemd user unit that runs this binary with the given `--config`; unchanged units are left alone.
- **`trust add` / `trust remove` are idempotent and report whether they changed anything.**
- **`[identity] cert` / `key`** use certificates supplied by your own PKI or deployment tooling instead of generating one.
- See [docs/automation.md](docs/automation.md) for the deployment model and an Ansible example.

### Fixed

- An explicitly given `--config` that does not exist is now an error instead of silently starting with defaults.
- `status` reads the configuration given with `--config` and finds a configured certificate.

### Security — dependency advisories

- **Updated dependencies with published RustSec advisories** (lockfile only, semver-compatible). These include remotely triggerable panics, memory exhaustion and a stream-limit bypass in `quinn-proto` (0.11.13 → 0.11.19) and `quinn-udp`; handshake-message, certificate-decompression and secret-leaking `Debug` issues in `rustls` (0.23.36 → 0.23.46); certificate-validation fixes in `rustls-webpki`; and soundness fixes in `tokio` and `anyhow`. Yanked `spin` and `clipboard-win` versions were replaced too.
- **Dropped the unmaintained `rustls-pemfile`.** Certificates and keys are now parsed with `rustls-pki-types`' built-in PEM support.
- `bincode` is unmaintained with no maintained successor. It is recorded as an explicit, reasoned exception: decoding is bounded and fuzzed, and a codec change (which changes the wire format) is tracked separately.

### Added — project hygiene

- **CI:**
  - `cargo-deny` (advisories, licenses, bans, sources) on every pull request. `deny.toml` is migrated to the v2 format; the old file was rejected by current cargo-deny, so it never actually ran.
  - Clippy over all targets and features.
  - Rustdoc with warnings denied.
  - A Rust 1.89 build check.
- **MSRV is now stated accurately as 1.89.** The previous `1.75` was below what the dependencies already required.
- **Fuzzing:** cargo-fuzz targets for the wire decoder (`fuzz/`), run weekly in CI. A deterministic random/mutation test of the decoder (100k inputs) runs in every build.
- Internal path dependencies carry versions, so the crates are publishable.
- The landing page and README now describe the trust model accurately (mutual authentication; discovery is never trusted).

### Fixed — robustness

- **A stalled peer can no longer freeze the daemon.** Control and input messages are queued to per-stream writer tasks (`QueuedSender`) instead of being written from the event loop. A full queue reports backpressure: input stops being forwarded (control returns to the local machine), and a peer whose control queue stays full for a ping interval is disconnected.
- **Inbound connections cannot stall the accept loop.** The TLS handshake now runs in a per-connection task (`QuicTransport::accept_incoming` / `Incoming`). At most 16 handshakes run at once; further attempts are refused before any cryptographic work. Every handshake, inbound or outbound, must finish within 10 s.
- **Configured peers are redialed.** A peer with a configured `address` is dialed until a session exists and again after it drops, with exponential backoff (1 s doubling to 60 s). Trusting a peer while the daemons run no longer needs a restart.
- **Device limits.** A peer can create at most 16 virtual devices; re-announcing a device id replaces its device instead of leaking the old one; `DeviceGone` releases held keys before destroying the device.
- **Clipboard size matches the wire.** `clipboard.max_size` defaults to, and is capped at, what one protocol message can carry (~1 MiB; previously 10 MiB, which could never be sent). The offer no longer reads the whole clipboard just to report its size.
- **Private files are private.** The TLS private key, machine id, PID and status files are written atomically with mode 0600 in 0700 directories; a world-readable key from an earlier version is tightened on load. The PID and status files no longer fall back to the shared `/tmp`, and the status file is written off the async runtime.

### Security — mutual authentication and explicit peer trust

**Breaking:** peers are now refused unless trusted. After upgrading, pair your machines once (`cross-control pair`) or pin fingerprints in `config.toml`.

- **Handshake signatures are verified.** The certificate verifier introduced in [#2] (below) compared fingerprints but accepted any handshake signature, so presenting a trusted peer's public certificate was enough to pass the pin. Both directions now use rustls' standard signature verification: a peer must hold the private key behind the fingerprint it claims. Regression tests forge a signature in each direction.
- **Mutual TLS.** The server previously accepted any client. Clients now must present their certificate and prove possession of its key; the daemon reads the peer's fingerprint after the handshake.
- **Peer authorization.** A peer is accepted only if its `(name, fingerprint)` is trusted: a `[[screens]] fingerprint` pin, or an entry in the new `known_peers` file (`~/.config/cross-control/known_peers`, mode 0600, written atomically). A trusted name with a different certificate is always refused, and a trusted certificate cannot claim another name. The responder decides before revealing anything about itself, and tells a refused peer why.
- **Trust-on-first-use is opt-in** via `[security] trust_on_first_use` (default off). The old behaviour accepted anyone on first contact.
- **mDNS fingerprints are hints only.** Outbound connections are pinned to the trust store, not to whatever fingerprint a (forgeable) mDNS advert carries.
- **New `cross-control trust` command** (`list` / `add <name> <fingerprint>` / `remove <name>`), which also shows this machine's own fingerprint.
- **`cross-control pair` records the peer.** It shows the fingerprint and asks before trusting it (`--yes` to skip; refuses without a terminal), saves it to `known_peers`, and prints the `trust add` command for the other machine. `--name` covers peers that do not trust us yet and so will not announce their name.
- **Duplicate and stale connections.** When two connections to the same peer race (both sides dial at once), both sides keep the one opened by the lower machine id. Events carry a per-connection id, so a replaced connection's late events or disconnect no longer tear down the live session. A session claiming an existing peer's machine id with a different certificate is dropped.
- `cross-control status` shows this machine's name and fingerprint even when the daemon is stopped.

### Security — TOFU certificate pinning is now enforced ([#2])

- **The QUIC client now verifies peer certificates by SHA-256 fingerprint.** Previously the client skipped certificate verification entirely (an MVP placeholder), so the "trust-on-first-use pinning" of ADR 0002 was documented but not enforced. A new `FingerprintVerifier` closes that gap: when a peer's fingerprint is known — from a `[[screens]] fingerprint` entry or an mDNS TXT advert — the TLS handshake is refused unless the presented certificate hashes to it; on genuine first contact (no pin) the certificate is accepted (TOFU). Fingerprints are compared in canonical form, so the colon-grouped config format and the compact mDNS format interoperate.
- **New `QuicTransport::connect_pinned(addr, name, expected_fingerprint)`**; the daemon dials discovered and configured peers through it, passing the fingerprint each discovery backend already carries. `connect` (first-contact) remains for the unpinned path. The self-signed certificate's signature is trusted because the trust anchor is the fingerprint, not a CA chain.
- **`cross-control pair <host>` is now implemented.** It connects to the peer, reads its certificate fingerprint (`PeerConnection::peer_fingerprint`), completes the handshake to learn the peer's name, and prints a ready-to-paste `[[screens]]` entry with the fingerprint filled in — the SSH-style first-contact flow ADR 0002 calls for. The displayed fingerprint is byte-identical to what `generate-cert`, `status`, and mDNS produce, so the pin matches.

[#2]: https://github.com/Adjoint-uk/cross-control/issues/2

### Added — Phase 2: multi-monitor support ([#8])

- **Machines can expose multiple monitors.** New `DisplayLayout { monitors: Vec<ScreenGeometry> }` type describes a machine's full monitor arrangement, configured via a `[[monitors]]` list (each with `width`, `height`, and top-left `x`/`y` offset). When no monitors are listed, the daemon falls back to a single monitor sized by `daemon.screen_width`/`screen_height`, so existing configs are unchanged.
- **Edge detection spans the combined desktop.** The daemon derives the union bounding box of all monitors and runs all cursor clamping and barrier crossing against it — so on a dual-monitor machine the cursor now traverses the whole desktop before crossing to another machine, instead of stopping at the first monitor's inner edge. All crossing logic is unchanged; only the screen bounds it operates on now come from the layout.
- **Layout travels on the wire.** `Hello`/`Welcome`/`ScreenUpdate` now carry the `DisplayLayout` instead of a single `ScreenGeometry`, so peers see each other's real monitor arrangement. Config validation rejects a monitor with a zero dimension.

### Changed

- **Protocol version → 0.2.** The handshake's screen field changed from `ScreenGeometry` to `DisplayLayout`; major version stays 0, so this is a breaking wire change within the pre-1.0 alpha (rebuild both ends).

[#8]: https://github.com/Adjoint-uk/cross-control/issues/8

### Added — Phase 2: clipboard HTML and images ([#6], [#7])

- **HTML and PNG image clipboard sync.** The clipboard path is no longer text-only. `ClipboardProvider` gained `get_format(format)`, and the `arboard` backend now reads/writes HTML (`get().html()` / `set_html`) and images, converting between the wire's PNG bytes and the raw RGBA the platform clipboard uses (via the `image` crate, PNG feature only). `available_formats()` probes all three formats.
- **Richest-format selection on hand-off.** When the controller offers its clipboard, the controlled side now requests the richest format it can apply — image, else HTML, else plain text — instead of always taking plain text.
- **Size cap enforced ([#7]).** Clipboard payloads over `clipboard.max_size` (default 10 MiB) are dropped with a warning on both the send and receive side, rather than put on the control stream. Chunked streaming of large images over a dedicated QUIC stream remains a follow-up.

[#6]: https://github.com/Adjoint-uk/cross-control/issues/6
[#7]: https://github.com/Adjoint-uk/cross-control/issues/7

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

### Added — loopback demo and systemd user service ([#11])

- **`loopback` example.** `cargo run -p cross-control-daemon --example loopback` runs two daemons in one process on `127.0.0.1` and drives a full session — QUIC handshake, device announce, edge-based cursor crossing, and input forwarding — with no second machine, no root, and no display server. It asserts the forwarded keypress lands on the receiving daemon and narrates each step. Built with `--features linux -- --real`, the receiver injects into a real uinput device so the cursor visibly moves. This makes everything *between* the two physical ends reproducible on one box; only real evdev capture and a live compositor remain for hardware bring-up.
- **systemd user service, fixed up ([#11]).** `systemd/cross-control.service` is now a correct, documented user unit: dropped the `network-online.target` ordering (unavailable in the user manager; the daemon reconnects on its own), added install/prerequisite comments, and reconciled the copy `install.sh` generates so the two no longer diverge. README gains a "Run as a service" section covering enable, logs, lingering, and the `input`/`uinput` prerequisites.

[#11]: https://github.com/Adjoint-uk/cross-control/issues/11

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

### Phase 1 close-out (the untagged 0.2.0-alpha)

Two cross-control daemons on the same LAN now find each other automatically — no `address = "..."` required in the config. The mDNS advert carries the machine's TLS cert fingerprint so peers can pin before the first QUIC handshake.

#### Added

- **mDNS / DNS-SD discovery backend** (`cross-control-discovery::MdnsDiscovery`) — advertises this daemon and browses for peers on `_cross-control._udp.local.`. TXT records carry `MachineId` (`id`) and SHA-256 cert fingerprint (`fp`) so discovered peers can be TOFU-verified before connection.
- **Composable discovery layer** (`DiscoveryAggregator`, `StaticDiscovery`) — multiple discovery backends share one peer stream into the daemon, with dedupe by `MachineId`. Static config peers and mDNS peers flow through the same path. Designed so cross-subnet rendezvous can slot in later without touching the daemon.
- **Daemon-level dial dedupe** — outbound connection attempts are keyed by `SocketAddr`, suppressing duplicate dials when a peer surfaces via more than one discovery backend.
- **`Daemon::set_local_fingerprint`** — wires the local TLS cert fingerprint into the mDNS advertisement; called automatically by the CLI from the loaded cert.
- **[ADR 0002 — TOFU certificate pinning](docs/adr/0002-tofu-pairing.md)** — documents the trust model, what TOFU does and does not protect against, key rotation story, and config layout.

#### Changed

- `cross-control-daemon` now starts the discovery aggregator at run time instead of dialing static peers in a one-shot loop. Config-only setups still work unchanged — the static address list becomes a `StaticDiscovery` backend.
- `daemon.discovery = true` (the existing config flag) now actually enables mDNS instead of being a no-op.

#### Fixed

- A handful of pre-existing clippy lints under newer toolchains (`similar_names`, `manual_let_else`, `match_wildcard_for_single_variants`, `doc_markdown`, `too_many_lines`, `needless_borrows_for_generic_args`, `ip_constant`). No behavioural changes — `cargo clippy --workspace --all-targets -- -D warnings` is clean again.

#### Test coverage

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
