# Architecture

## Overview

cross-control is a virtual KVM that shares keyboard and mouse input across machines connected on the same network. It uses QUIC for transport, bincode v2 for serialisation, and platform-specific backends for input capture/emulation.

## Workspace Structure

```
cross-control/
  crates/
    cross-control-types/       # Shared types (leaf crate, no platform code)
    cross-control-protocol/    # QUIC transport, wire format, messages
    cross-control-input/       # Platform-abstracted capture + emulation
    cross-control-clipboard/   # Clipboard sync
    cross-control-discovery/   # mDNS/DNS-SD zero-config
    cross-control-daemon/      # Core state machine, event routing, IPC
    cross-control-cli/         # User-facing binary
    cross-control-certgen/     # TLS certificate generation
```

## Dependency Graph

```
types <- protocol <- daemon <- cli
types <- input    <- daemon
types <- clipboard <- daemon
types <- discovery <- daemon
         certgen  <- cli
```

## Data Flow

1. Physical input is captured by the `InputCapture` backend (evdev on Linux, Raw Input on Windows)
2. When the cursor hits a screen-edge barrier, the daemon sends an `Enter` control message
3. The remote daemon acknowledges with `EnterAck`
4. Input events are serialised with bincode v2 and sent over a QUIC unidirectional stream
5. The remote `InputEmulation` backend creates virtual devices and injects events
6. When the cursor returns, a `Leave` message closes the input stream

## Protocol

See `docs/protocol.md` for the full protocol specification.

## Platform Backends

| Platform | Capture | Emulation | Clipboard |
|----------|---------|-----------|-----------|
| Linux (evdev) | evdev device grab | uinput virtual device | arboard |
| Linux (Wayland) | wlr-layer-shell | wlr-virtual-pointer/keyboard | wl-clipboard-rs |
| Windows | Raw Input hooks | SendInput API | arboard |
