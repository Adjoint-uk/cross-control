# Windows (preview)

cross-control builds and runs on Windows 10 and 11 with a native input
backend. It is a **preview**: the backend compiles and is unit-tested in CI,
but has not yet been validated on real hardware. Reports are welcome; use
the checklist in [hardware-test.md](hardware-test.md).

## How it works

- **Capture** uses low-level keyboard and mouse hooks on a dedicated thread.
  While you control another machine, local input is swallowed (so local
  programs do not see it) and the cursor is parked in the middle of the
  primary screen, so movement in every direction keeps registering.
- **Emulation** uses `SendInput`. Keys are sent by hardware scan code, so the
  receiving machine's keyboard layout decides the character, exactly as for a
  physical keyboard.
- Input the daemon injects itself is ignored by its hooks, so being
  controlled never echoes back to the controller.
- No device permissions or drivers are needed.

## Setup

```powershell
cargo install --path crates/cross-control-cli
cross-control doctor
cross-control identity
```

Configuration lives in `%APPDATA%\cross-control\` (or wherever
`CROSS_CONTROL_CONFIG_DIR` points). Pairing, trust and layout work exactly as
on Linux; see the [setup guide](setup-guide.md). Allow `cross-control.exe`
through Windows Defender Firewall (UDP 24800) when prompted.

Run `cross-control start` in a terminal; `cross-control status` and
`cross-control stop` work from another terminal.

## Automated testing on real Windows

Every pull request runs an end-to-end test on CI's Windows runner (a fresh
Windows VM with an interactive desktop, discarded afterwards):
`crates/cross-control-daemon/tests/windows_e2e.rs`. Two daemons on the same
desktop, connected over QUIC with mutual TLS:

1. Synthetic "hardware" mouse motion goes through daemon A's **real
   low-level hooks**; the real cursor reaches the screen edge and A crosses
   to B.
2. Further motion is captured and swallowed by A, sent to B, and replayed by
   B's **real `SendInput`**: the real cursor moves.
3. A key captured by A is pressed system-wide by B (and only by B).
4. The release hotkey, captured by A, takes control back; nothing is left
   pressed.

Synthetic input is tagged so A's hooks accept it, which only builds with the
`test-support` feature (tests only); B's own injections are untagged and
ignored, exactly as in production. On CI the test fails rather than skips if
the desktop is unavailable (`CROSS_CONTROL_REQUIRE_DESKTOP=1`).

For hands-on testing without a spare PC, run Windows in a VM on a machine with
hardware virtualisation (KVM / Hyper-V / VirtualBox), using Microsoft's free
evaluation images; for example the
[dockur/windows](https://github.com/dockur/windows) container on a Linux host
with `/dev/kvm`, or a Vagrant Windows box. Bridge the VM's network so the
daemons can reach each other.

## Limits

These are Windows rules for normal (non-elevated) programs:

- **Elevated windows:** input cannot be injected into programs running as
  administrator (User Interface Privilege Isolation). Typing into such a
  window from another machine does nothing.
- **Secure desktop:** UAC prompts, Ctrl+Alt+Del and the lock screen cannot be
  reached.
- **`stop` terminates the process.** Windows cannot deliver Ctrl+C to another
  console, so `cross-control stop` ends the daemon instead of asking it to
  shut down; peers see the connection drop and reconnect when it restarts.
- No Windows service yet: start it from a terminal or Task Scheduler.

Lifting the first two needs a signed binary with UI access, or a service
with a helper in the user's session — planned.
