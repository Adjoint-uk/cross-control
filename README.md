# cross-control

**Share your keyboard and mouse across machines.** A modern, Rust-based virtual KVM that lets you seamlessly move your cursor between computers on the same network.

[![CI](https://github.com/Adjoint-uk/cross-control/actions/workflows/ci.yml/badge.svg)](https://github.com/Adjoint-uk/cross-control/actions/workflows/ci.yml)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE-MIT)

## Why cross-control?

The software KVM space is stagnant. Barrier is unmaintained since 2022, Input Leap and Deskflow have broken Wayland support, Synergy went proprietary, and the Rust alternatives (RKVM, LAN Mouse) are either Linux-only or use incompatible protocols.

cross-control takes a different approach:

| | Barrier | Input Leap | Synergy | RKVM | LAN Mouse | **cross-control** |
|---|---|---|---|---|---|---|
| **Active** | Dead (2022) | Slow | Yes | Yes | Yes | **Yes** |
| **Wayland** | No | Partial | Partial | Yes | Yes | **Native** |
| **Windows** | Yes | Yes | Yes | No | Yes | **Planned** |
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
- **Certificate pinning** - SSH-style trust-on-first-use authentication. No central authority needed.
- **Multi-machine** - Star topology supports any number of machines.
- **Permissive license** - MIT OR Apache-2.0. Use it anywhere.

## Requirements

- **Linux** (x86_64 or aarch64) — Windows support is planned
- **Rust 1.75+** (if building from source)
- User must be in the `input` group to access keyboard/mouse devices
- `/dev/uinput` must be accessible for virtual device emulation

## Installation

### Install script (recommended)

```bash
curl -fsSL https://raw.githubusercontent.com/Adjoint-uk/cross-control/main/install.sh | bash
```

### Prebuilt binaries

Download the latest release from [GitHub Releases](https://github.com/Adjoint-uk/cross-control/releases).

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

### 2. Generate certificates on both machines

```bash
cross-control generate-cert --output ~/.config/cross-control/
```

Note the fingerprint printed for each machine.

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
fingerprint = "SHA256:..."  # laptop's fingerprint
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
fingerprint = "SHA256:..."  # workstation's fingerprint
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

## Development Status

cross-control is in early development. Current status:

- [x] **Phase 0**: Workspace scaffold, shared types, trait definitions, CI
- [x] **Phase 1**: Linux-to-Linux MVP (evdev capture, QUIC transport, multi-hop switching, 57+ tests)
- [ ] **Phase 2**: Discovery, certificate pinning, pairing workflow
- [ ] **Phase 3**: Windows support, clipboard sharing
- [ ] **Phase 4**: Wayland-native backends, polish
- [ ] **Phase 5**: v1.0 stable release

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
