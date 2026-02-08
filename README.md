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
| **Windows** | Yes | Yes | Yes | No | Yes | **Yes** |
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

## Building

```bash
# Requirements: Rust 1.75+ (stable)
cargo build --workspace

# Run tests
cargo test --workspace

# Build release binary
cargo build --release -p cross-control-cli
```

## Usage

```bash
# Generate a TLS certificate (first time)
cross-control generate-cert --output ~/.config/cross-control/

# Start the daemon
cross-control start

# Pair with another machine
cross-control pair 192.168.1.42:24800

# Check status
cross-control status
```

See `examples/config.toml` for configuration options.

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
position = "Right"
```

## Development Status

cross-control is in early development. Current status:

- [x] **Phase 0**: Workspace scaffold, shared types, trait definitions, CI
- [ ] **Phase 1**: Linux-to-Linux MVP (evdev capture, QUIC transport, basic switching)
- [ ] **Phase 2**: Discovery, multi-machine, certificate pinning
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
