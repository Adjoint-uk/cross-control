# macOS (preview)

cross-control has a native macOS backend. Like the [Windows backend](windows.md)
it is a preview: CI builds and tests it on macOS, but it has not yet been
validated on physical machines (see [hardware-test.md](hardware-test.md)).

## How it works

- **Capture:** a Quartz event tap on a dedicated thread with its own run
  loop. While you control another machine, local events are swallowed and
  the cursor is detached from the mouse
  (`CGAssociateMouseAndMouseCursorPosition`). Motion keeps arriving as deltas
  and never pins against a screen edge. When you control this Mac, the
  backend reports the cursor's real position, so crossing at an edge is exact.
- **Emulation:** `CGEventPost`.
  - Keys are sent by virtual key code, which names the physical key, so it
    works with any keyboard layout.
  - The modifier flags of held keys go on every event, so Cmd+C arrives as
    Cmd+C.
  - While a button is held, motion is sent as "dragged" events, which drag
    and drop needs.
  - Double clicks are counted as for a physical mouse.
  - Smooth scrolling keeps its fractional deltas.
- **No echo:** events this process posts carry a marker that its own tap
  ignores, so being controlled never sends input back.
- PC keys map to their Mac equivalents: Insert is Help, Num Lock is Clear,
  and Print Screen / Scroll Lock / Pause are F13 / F14 / F15.

## Permissions

macOS requires two privacy permissions, granted per program in
**System Settings → Privacy & Security**:

| Permission | Needed for |
|---|---|
| **Input Monitoring** | capturing your keyboard and mouse (sharing them) |
| **Accessibility** | injecting input (being controlled) and swallowing local input while you control another machine |

The first `cross-control start` triggers the system prompts. Add the
`cross-control` binary in both lists and enable it, then restart the daemon.
`cross-control doctor` reports what is granted and what is missing:

```
[ok  ] input      Input Monitoring granted (capture)
[FAIL] input      no Accessibility permission: input cannot be injected or grabbed
                  System Settings → Privacy & Security → Accessibility: add and enable /usr/local/bin/cross-control
```

macOS ties the permission to the binary. Replacing it, for example after an
upgrade, can require granting the permission again.

## Setup

Build from source for now:

```bash
cargo install --path crates/cross-control-cli
cross-control doctor
cross-control identity
```

Configuration lives in `~/Library/Application Support/cross-control/`, or
wherever `CROSS_CONTROL_CONFIG_DIR` points. Pairing, trust and layout work as
on the other platforms; see the [setup guide](setup-guide.md).

## Limits

- **Secure input:** event taps cannot see the login window, the lock screen
  or secure text fields (password prompts), and cannot inject into them.
  This macOS protection applies to every program.
- **Caps Lock** is captured as it toggles. Toggling it remotely by posted
  events is unreliable on macOS.
- **Not yet:** start at login (a launchd agent), release downloads and an
  installer for macOS. Until then, start it from a terminal.
