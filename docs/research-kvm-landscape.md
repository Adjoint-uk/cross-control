# Software KVM Landscape Research (March 2026)

Competitive analysis for cross-control positioning.

## Software KVM Solutions

### Barrier — Dead
- Last release: v2.4.0 (2021)
- Dropped from Debian. Maintainers refuse to archive, causing confusion.
- No security fixes or improvements.
- https://github.com/debauchee/barrier

### Input Leap — Barrier's Successor
- Current: v3.0.3 (June 2025)
- Created by Barrier's active maintainers.
- Packaged in Arch, Fedora, Manjaro, Solus.
- **Problems**: v3.0.3 shipped without Windows installers (only .tar.gz and debug builds, unsigned). v3.0.2 available via Chocolatey/winget.
- Wayland support via libei (GNOME works, clipboard not yet on Wayland).
- Development pace slower than Deskflow.
- https://github.com/input-leap/input-leap

### Deskflow — Most Active Open Source
- Current: v1.26.0 (February 2026). 15k+ GitHub stars.
- Official upstream of Synergy (the commercial product).
- **Strengths**: MSI installer for Windows (silent install via `msiexec /i ... /qn`), TLS encryption, daemon/service mode, Wayland via libei/libportal, active development, in Debian and Arch repos.
- **Concerns**: Multiple open issues around TLS configuration (cert generation failures, Windows↔Linux connection errors). Configuration friction rather than fundamental bugs.
- Network-compatible with Barrier and Input Leap (Synergy v1 protocol).
- https://github.com/deskflow/deskflow

### Synergy — Commercial
- Commercial version of Deskflow, by Symless.
- Personal: ~$30 one-time. Business: subscription, per concurrent user.
- Used by Google, Amazon, Intel, Nvidia, Cisco, GE, HP.
- Same underlying code as Deskflow — you're paying for polish and support.
- https://symless.com/synergy

### RKVM — Rust, Linux-only
- Rust-based, Linux-only. No Windows support.
- GPL-3.0.
- https://github.com/htrefil/rkvm

### LAN Mouse — Rust, Cross-platform
- Rust-based, cross-platform (Linux, Windows, macOS).
- Custom protocol. GPL-3.0.
- https://github.com/feschber/lan-mouse

## Hardware KVM-over-IP (Clinical Grade)

For safety-critical clinical environments (treatment rooms), hardware KVM is the standard:

| Vendor | Notes |
|---|---|
| **Adder Technology** | Markets specifically to healthcare/radiotherapy. IP-based, dedicated hardware. |
| **IHSE** | German. Strong in control rooms and medical. 24/7 continuous operation, hotswap, redundant failover. |
| **Raritan (Legrand)** | Dominion KX III. Dual power, dual GigE with auto-failover. AES + FIPS 140-2. CAC/2FA. IEEE 802.1X. |

### Why hardware for clinical
- No software to crash, no OS dependencies
- Deterministic latency
- Redundant power and network with auto-failover
- FIPS 140-2 and medical-grade security
- Works at BIOS/boot level — OS-independent

## cross-control Positioning

| | Barrier | Input Leap | Deskflow | Synergy | RKVM | LAN Mouse | **cross-control** |
|---|---|---|---|---|---|---|---|
| **Status** | Dead | Slow | Active | Active | Active | Active | **Active** |
| **Language** | C++ | C++ | C++ | C++ | Rust | Rust | **Rust** |
| **Protocol** | Synergy v1 | Synergy v1 | Synergy v1 | Proprietary | Custom | Custom | **QUIC** |
| **Encryption** | Optional TLS | Optional TLS | Optional TLS | TLS | TLS | None | **TLS 1.3 always** |
| **Discovery** | No | No | No | No | No | No | **mDNS** |
| **Wayland** | No | Partial | Partial | Partial | Yes | Yes | **Native** |
| **Windows** | Yes | Yes | Yes | Yes | No | Yes | **Planned** |
| **License** | GPL-2.0 | GPL-2.0 | GPL-2.0 | Proprietary | GPL-3.0 | GPL-3.0 | **MIT/Apache-2.0** |

### Key differentiators
1. **QUIC transport** — no other KVM uses this. Built-in TLS 1.3, multiplexed streams, 0-RTT reconnect.
2. **mDNS zero-config** — machines find each other automatically.
3. **Permissive license** — MIT/Apache-2.0 enables commercial and clinical deployment without GPL concerns.
4. **Rust** — memory safety without garbage collection. Suitable for long-running daemon processes.
5. **Clinical use case** — designed with radiotherapy workstation environments in mind.

### Critical path to production
- **Phase 2**: Discovery + certificate pinning (mDNS already scaffolded)
- **Phase 3**: Windows support (required for clinical — OIS and TPS run on Windows)
- **Phase 4**: Wayland-native backends
- **Phase 5**: v1.0 stable release
