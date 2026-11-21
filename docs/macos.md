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

Releases from 0.3.0 include macOS downloads for Apple silicon and Intel.
The install script picks the right one, checks it against the release's
checksums, and installs it to `~/.local/bin`:

```bash
curl -fsSL https://raw.githubusercontent.com/Adjoint-uk/cross-control/main/install.sh | bash
cross-control identity
```

Or build from source with `cargo install --path crates/cross-control-cli`.

Configuration lives in `~/Library/Application Support/cross-control/`, or
wherever `CROSS_CONTROL_CONFIG_DIR` points. Pairing, trust and layout work as
on the other platforms; see the [setup guide](setup-guide.md).

## Start at login

```bash
cross-control service install --enable   # write the agent and start it now
cross-control service unit               # print the agent without installing
cross-control service uninstall          # stop it and remove the agent
```

This writes a launchd agent to
`~/Library/LaunchAgents/uk.adjoint.cross-control.plist`. It runs in your
login session, where the event tap and event posting work. launchd restarts
the daemon if it fails, but not after `cross-control stop`. Its output goes to
`cross-control.log` in the configuration directory. Installing is idempotent:
an unchanged agent is left alone, and a changed one is reloaded.

Grant the two permissions to the same binary the agent runs (the path
`service unit` shows).

## Limits

- **Secure input:** event taps cannot see the login window, the lock screen
  or secure text fields (password prompts), and cannot inject into them.
  This macOS protection applies to every program.
- **Caps Lock** is captured as it toggles. Toggling it remotely by posted
  events is unreliable on macOS.
- **Unsigned binaries:** release binaries are ad-hoc signed, not notarised.
  The install script downloads with `curl`, which Gatekeeper does not
  quarantine. A binary downloaded in a browser needs
  `xattr -d com.apple.quarantine cross-control` before it runs.
