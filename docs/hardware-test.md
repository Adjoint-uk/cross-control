# Two-machine hardware test

Everything between the two physical ends is tested automatically (see the
`loopback` example and the integration tests). What CI cannot exercise is
real keyboards and mice, a real desktop session and a real network. This is
the checklist for that, on two Linux machines. It takes about 30 minutes.

## 0. Prepare both machines

```bash
cargo install --path crates/cross-control-cli     # or copy a release binary
cross-control doctor
```

`doctor` checks input-device permissions, `/dev/uinput`, detected keyboards
and mice, config, identity, trust, the UDP port and clipboard access, and
prints the exact fix for anything that fails. Re-run it until it says
**Ready** (warnings about config and trust are expected at this point).

Typical first-time fixes: add yourself to the `input` group (then log out and
back in) and load `uinput` with the udev rule `doctor` prints.

## 1. Configure and pair

Here `left` is at 192.0.2.10 and `right` at 192.0.2.20; use your real names
and addresses.

On **left**, `~/.config/cross-control/config.toml`:

```toml
[identity]
name = "left"

[[screens]]
name = "right"
position = "Right"
address = "192.0.2.20:24800"
```

On **right**, the mirror image (`name = "right"`, a `left` screen at
`position = "Left"`, `address = "192.0.2.10:24800"`).

Then exchange fingerprints:

```bash
# on each machine
cross-control identity                      # note the fingerprint
# on left
cross-control trust add right SHA256:…      # right's fingerprint
# on right
cross-control trust add left SHA256:…       # left's fingerprint
```

(`cross-control pair` does the same interactively once one side runs.)

`cross-control doctor` on both should now report every check `ok` except
possibly `session`/`clipboard` details.

## 2. Start with debug logging

In a terminal on each machine, inside the graphical session:

```bash
RUST_LOG=cross_control=debug,cross_control_daemon=debug cross-control start 2>&1 | tee ~/cc-$(hostname).log
```

On either machine, `cross-control status` should show the other as
connected within a few seconds.

## 3. Checklist

Mark each one; note anything odd (lag, stuck keys, wrong position).

**Crossing**
- [ ] Move the cursor off left's right edge: it appears on right, near the same height.
- [ ] Move it back off right's left edge: it returns to left.
- [ ] Cross at the top, middle and bottom of the edge: the height carries over.
- [ ] Holding a key or mouse button at the edge does **not** cross; releasing it lets you cross.

**Input while controlling right (from left's keyboard and mouse)**
- [ ] Typing lands on right; nothing is typed on left (input is grabbed).
- [ ] Shift, Ctrl, Alt and Super combinations work (e.g. Ctrl+C / Ctrl+V, Alt+Tab).
- [ ] Left, right and middle click; drag-select text.
- [ ] Scroll wheel, vertical and horizontal.
- [ ] No key stays pressed after crossing back (e.g. hold Shift, cross back, release).

**Release hotkey**
- [ ] Ctrl+Shift+Esc while controlling right returns control to left immediately.

**Clipboard**
- [ ] Copy text on left, cross, paste on right.
- [ ] Copy text on right, cross back, paste on left.
- [ ] Copy an image (screenshot) and paste it on the other side.

**Resilience**
- [ ] Stop the daemon on right (Ctrl+C): left takes control back and keeps working locally.
- [ ] Start it again: they reconnect within about a minute without touching left.
- [ ] Unplug the network or switch Wi-Fi off and on: they reconnect.
- [ ] While controlling right, stop right's daemon: left's keyboard and mouse work locally again.

**Optional**
- [ ] Multiple monitors on one side (add `[[monitors]]`): crossing happens only at the outer edge.
- [ ] Both Wayland and X11 sessions.
- [ ] `cross-control service install --enable` and log out/in: it starts on its own.

## 4. Reporting results

For anything that fails, send:

- which checklist item, and what happened instead;
- `cross-control doctor --json` and `cross-control status --json` from both machines;
- the two log files from step 2 (`~/cc-<hostname>.log`).

With the log filter from step 2, logs contain machine names, addresses and
certificate fingerprints (which are public), but no keystrokes or clipboard
contents. Do not raise the input crate to `trace` for a report: at that
level it records every injected key.
