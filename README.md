# cross-control

> **Status: `0.3.0`.** The core KVM works end to end on Linux, with Windows and macOS as previews. Downloads for Linux, Windows and macOS are on the [releases page](https://github.com/Adjoint-uk/cross-control/releases). It is pre-1.0, so the wire protocol can still change between minor versions: upgrade every machine together.

**Share your keyboard and mouse across machines.** A modern, Rust-based virtual KVM that lets you seamlessly move your cursor between computers on the same network.

[![CI](https://github.com/Adjoint-uk/cross-control/actions/workflows/ci.yml/badge.svg)](https://github.com/Adjoint-uk/cross-control/actions/workflows/ci.yml)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE-MIT)
[![Website](https://img.shields.io/badge/web-cross--control.adjoint.uk-3BE0CE.svg)](https://cross-control.adjoint.uk)

**Website:** [cross-control.adjoint.uk](https://cross-control.adjoint.uk) · the landing page source lives in [`site/`](site/).

## Why cross-control?

The software KVM space is stagnant. Barrier is unmaintained since 2022, Input Leap and Deskflow have broken Wayland support, Synergy went proprietary, and the Rust alternatives (RKVM, LAN Mouse) are either Linux-only or use incompatible protocols.

cross-control takes a different approach:

| | Barrier | Input Leap | Synergy | RKVM | LAN Mouse | **cross-control** |
|---|---|---|---|---|---|---|
| **Active** | Dead (2022) | Slow | Yes | Yes | Yes | **Yes** |
| **Wayland** | No | Partial | Partial | Yes | Yes | **Native** |
| **Windows** | Yes | Yes | Yes | No | Yes | **Preview** |
| **macOS** | Yes | Yes | Yes | No | Yes | **Preview** |
| **Protocol** | Synergy v1 | Synergy v1 | Proprietary | Custom | Custom | **QUIC** |
| **Encryption** | Optional TLS | Optional TLS | TLS | TLS | None | **TLS 1.3 (always)** |
| **Discovery** | No | No | No | No | No | **mDNS** |
| **License** | GPL-2.0 | GPL-2.0 | Proprietary | GPL-3.0 | GPL-3.0 | **MIT/Apache-2.0** |

## Features

- **QUIC transport** - Multiplexed streams, built-in TLS 1.3, 0-RTT reconnection. No other KVM uses QUIC.
- **Wayland-native** - Works on Wayland from day one via evdev/uinput, with enhanced support for wlroots compositors.
- **Zero-config discovery** - Machines find each other automatically via mDNS/DNS-SD.
- **Clipboard sharing** - Text, HTML, and images synced when you switch machines.
- **Position-based switching** - Move your cursor to the screen edge to switch, just like macOS Universal Control.
- **Mutual authentication** - every peer proves it holds the key for a pinned certificate fingerprint; SSH-style pairing, no central authority.
- **Multi-machine** - Star topology supports any number of machines.
- **Permissive license** - MIT OR Apache-2.0. Use it anywhere.

## Requirements

- **Linux** (x86_64 or aarch64)
- **Windows** 10/11 — preview: tested end to end on CI's Windows desktop, not yet validated on physical machines (see [docs/windows.md](docs/windows.md))
- **macOS** 11+ — preview: native backend built and tested on CI's macOS runners; needs the Input Monitoring and Accessibility permissions; downloads for Apple silicon and Intel (see [docs/macos.md](docs/macos.md))
- **Rust 1.89+** (if building from source)
- Linux: user in the `input` group to access keyboard/mouse devices
- Linux: `/dev/uinput` accessible for virtual device emulation

## Installation

### Install script (recommended)

Linux and macOS:

```bash
curl -fsSL https://raw.githubusercontent.com/Adjoint-uk/cross-control/main/install.sh | bash
```

Windows (PowerShell):

```powershell
irm https://raw.githubusercontent.com/Adjoint-uk/cross-control/main/install.ps1 | iex
```

Both download the newest release and verify its checksum. They install it
for the current user without needing admin rights, then run
`cross-control doctor` to show anything left to set up. On Windows, the
[CrossControl PowerShell module](deploy/powershell/README.md) is installed
too. Set `CROSS_CONTROL_VERSION` to install a specific version.

### Prebuilt binaries

[GitHub Releases](https://github.com/Adjoint-uk/cross-control/releases) has
archives for Linux (x86_64, aarch64) and Windows (x86_64), with
`SHA256SUMS` and signed build provenance; see
[releasing](docs/releasing.md#what-a-release-contains) for how to verify
them.

### Build from source

```bash
git clone https://github.com/Adjoint-uk/cross-control.git
cd cross-control
cargo install --path crates/cross-control-cli
```

### Linux permissions setup

```bash
# Add your user to the input group (required for keyboard/mouse access)
sudo usermod -aG input $USER

# Ensure uinput is accessible
sudo modprobe uinput
echo 'KERNEL=="uinput", MODE="0660", GROUP="input"' | sudo tee /etc/udev/rules.d/99-uinput.rules
sudo udevadm control --reload-rules && sudo udevadm trigger

# Log out and back in for group changes to take effect
```

## Quick Start

Set up two Linux machines on the same network:

### 1. Install on both machines

```bash
cargo install --path crates/cross-control-cli
```

Then run `cross-control doctor`: it checks input-device permissions, uinput,
config, trust and the network port, and prints the fix for anything missing.

### 2. Pair the machines

Every connection is mutually authenticated: each machine has a self-signed
certificate, and a peer is accepted only if its certificate fingerprint is
trusted for the name it announces. Nothing is trusted by default.

Start the daemon on both machines (`cross-control start`), then show the
laptop's fingerprint on the laptop:

```bash
cross-control identity       # prints this machine's name and fingerprint
```

On the workstation, pair with it and check the fingerprint matches:

```bash
cross-control pair 192.168.1.20 --name laptop
```

`pair` records the laptop in `~/.config/cross-control/known_peers` and prints
a `cross-control trust add "workstation" SHA256:…` command: run it on the
laptop so it trusts the workstation back. A machine with the other's
`address` configured keeps retrying (backing off to once a minute), so they
connect on their own shortly after.

Alternatively, pin fingerprints directly with `fingerprint = "SHA256:…"` in
the `[[screens]]` entries below.

### 3. Create configuration

**Machine A** (left workstation at 192.168.1.10):

```toml
# ~/.config/cross-control/config.toml
[identity]
name = "workstation"

[[screens]]
name = "laptop"
address = "192.168.1.20:24800"
position = "Right"
```

**Machine B** (right laptop at 192.168.1.20):

```toml
# ~/.config/cross-control/config.toml
[identity]
name = "laptop"

[[screens]]
name = "workstation"
address = "192.168.1.10:24800"
position = "Left"
```

### 4. Start the daemon on both machines

```bash
cross-control start
```

### 5. Use it

Move your cursor to the right edge of Machine A's screen. It will appear on Machine B. Move it back to the left edge to return.

Press **Ctrl+Shift+Escape** to immediately release input and return control to the local machine.

### 6. Check status

```bash
cross-control status
```

See [docs/setup-guide.md](docs/setup-guide.md) for detailed setup instructions and troubleshooting.

### Run as a service

To keep the daemon running across logins, install the systemd **user** unit:

```bash
mkdir -p ~/.config/systemd/user
cp systemd/cross-control.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now cross-control
systemctl --user status cross-control     # check it's running
journalctl --user -u cross-control -f      # follow logs
```

The install script offers to do this for you. Two prerequisites, since input
capture/emulation needs device access: your user must be in the `input` group,
and `/dev/uinput` must be group-readable/writable (see **Linux permissions
setup** above). To have the service start before you log in, enable lingering:
`sudo loginctl enable-linger $USER`. The unit assumes the binary is on
`~/.cargo/bin`; if you installed elsewhere, adjust `ExecStart` with
`systemctl --user edit cross-control`.

## Try it on one machine

No second computer? The `loopback` example runs two daemons in one process and
drives a full session — handshake, cursor crossing, input forwarding — end to
end on `127.0.0.1`:

```bash
cargo run -p cross-control-daemon --example loopback
```

On a Linux desktop with `/dev/uinput` access, add `--features linux -- --real`
to have the receiving daemon inject into a real virtual device so you can watch
the cursor move. This covers everything except the physical input ends
(real evdev capture and a live compositor).

## Architecture

```
Physical Input                                              Virtual Input
  (keyboard/mouse)                                           (on remote machine)
       |                                                          ^
       v                                                          |
  +-----------+    +------------+    +-------+    +------------+  +-----------+
  | Input     | -> | Protocol   | -> | QUIC  | -> | Protocol   |->| Input     |
  | Capture   |    | Serialize  |    | Wire  |    | Deserialize|  | Emulation |
  +-----------+    +------------+    +-------+    +------------+  +-----------+
  (evdev/RawInput)  (bincode v2)    (quinn)       (bincode v2)   (uinput/SendInput)
       |                                                          |
  +-----------+                                            +-----------+
  | Clipboard | <------------- Clipboard Sync ------------>| Clipboard |
  +-----------+                                            +-----------+
       |                                                          |
  +-----------+                                            +-----------+
  | Discovery | <------------- mDNS/DNS-SD --------------->| Discovery |
  +-----------+                                            +-----------+
```

The project is organised as a Cargo workspace with 8 crates:

| Crate | Purpose |
|-------|---------|
| `cross-control-types` | Shared types: events, devices, screens, identifiers |
| `cross-control-protocol` | QUIC transport (quinn), wire format (bincode v2) |
| `cross-control-input` | Platform-abstracted input capture and emulation |
| `cross-control-clipboard` | Clipboard synchronisation |
| `cross-control-discovery` | mDNS/DNS-SD zero-config discovery |
| `cross-control-daemon` | Core state machine, barrier logic, event routing |
| `cross-control-cli` | User-facing binary |
| `cross-control-certgen` | TLS certificate generation |

## Configuration

Configuration lives at `~/.config/cross-control/config.toml`:

```toml
[daemon]
port = 24800
discovery = true

[identity]
name = "my-workstation"

[[screens]]
name = "laptop"
address = "192.168.1.42:24800"
position = "Right"
fingerprint = "SHA256:..."

# For 3+ machines, define adjacency
[[screen_adjacency]]
screen = "laptop"
neighbor = "tablet"
position = "Right"
```

See `examples/config.toml` for all configuration options.

### Automated deployment

Every command takes `--json` and returns meaningful exit codes, and trust can
be distributed without interactive pairing (`cross-control identity`,
`trust add`, `config check`, `service install`). An
Ansible role lives in [deploy/ansible](deploy/ansible); see
[docs/automation.md](docs/automation.md).

## Development Status

cross-control is in early development. Current status:

- [x] **Phase 0**: Workspace scaffold, shared types, trait definitions, CI
- [x] **Phase 1** (`v0.2.0-alpha`): Linux-to-Linux end-to-end — evdev capture, QUIC transport, multi-hop switching, mDNS discovery, certificate pinning, 64 tests
- [x] **Phase 2** (`v0.2.0`): Clipboard sharing, multi-monitor, reconnect, polished setup (`doctor`, start at login, clean stop)
- [ ] **Phase 3**: Windows backend (preview, shipped in 0.2.0), macOS backend (preview, shipped in 0.3.0), release downloads with provenance (shipped), real-hardware validation, distro packaging
- [ ] **Phase 4** (`1.0.0`): Protocol freeze, ADRs, security model, fuzz testing

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for development guidelines.

## Security

See [SECURITY.md](SECURITY.md) for the security policy and reporting vulnerabilities.

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

---

Built by [Adjoint Ltd](https://adjoint.uk)
