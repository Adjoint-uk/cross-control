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
