# Setup Guide

## Requirements

- **Linux** x86_64 or aarch64 (Ubuntu 22.04+, Fedora 38+, Arch, or similar)
- User in the `input` group (for keyboard/mouse access)
- `/dev/uinput` accessible (for virtual input devices)
- Network connectivity between machines (port 24800/UDP by default)
- Rust 1.75+ (if building from source)

## Installation

### Install script

```bash
curl -fsSL https://raw.githubusercontent.com/Adjoint-uk/cross-control/main/install.sh | bash
```

The script will download the latest binary, check permissions, and optionally set up a systemd service.

### Prebuilt binaries

Download from [GitHub Releases](https://github.com/Adjoint-uk/cross-control/releases):

```bash
# Download (replace with your architecture)
curl -fsSL -o cross-control https://github.com/Adjoint-uk/cross-control/releases/latest/download/cross-control-x86_64-unknown-linux-gnu
chmod +x cross-control
mv cross-control ~/.local/bin/
```

### Build from source

```bash
git clone https://github.com/Adjoint-uk/cross-control.git
cd cross-control
cargo install --path crates/cross-control-cli
```

## Linux Permissions Setup

cross-control needs access to input devices and the ability to create virtual devices.

### Add user to input group

```bash
sudo usermod -aG input $USER
# Log out and back in for the change to take effect
```

Verify with: `groups | grep input`

### Enable uinput

```bash
sudo modprobe uinput

# Make it persistent across reboots
echo uinput | sudo tee /etc/modules-load.d/uinput.conf

# Set permissions
echo 'KERNEL=="uinput", MODE="0660", GROUP="input"' | sudo tee /etc/udev/rules.d/99-uinput.rules
sudo udevadm control --reload-rules && sudo udevadm trigger
```

### Firewall

cross-control uses **UDP 24800** for the QUIC transport and **UDP 5353** for mDNS discovery. Open both on every machine:

```bash
# UFW (Ubuntu)
sudo ufw allow 24800/udp
sudo ufw allow 5353/udp

# firewalld (Fedora)
sudo firewall-cmd --add-port=24800/udp --permanent
sudo firewall-cmd --add-port=5353/udp --permanent
sudo firewall-cmd --reload

# iptables
sudo iptables -A INPUT -p udp --dport 24800 -j ACCEPT
sudo iptables -A INPUT -p udp --dport 5353 -j ACCEPT
```

On networks where mDNS is blocked (some enterprise WiFi, captive portals) set `[daemon] discovery = false` in `config.toml` and fall back to static `[[screens]]` entries with `address = "..."`.

## Zero-Config Quick Start (mDNS)

The fastest way to get two machines talking. Skip if your network blocks mDNS — use the static-config flow below instead.

### 1. Install on both machines and generate certs

```bash
cargo install --path crates/cross-control-cli
cross-control generate-cert --output ~/.config/cross-control/
```

Write down the SHA-256 fingerprint each machine prints. You will compare them in step 3 below.

### 2. Minimal config on each machine

```toml
# ~/.config/cross-control/config.toml
[identity]
name = "workstation"   # or "laptop" on the other machine

[daemon]
discovery = true        # mDNS on (this is the default)
screen_width = 1920
screen_height = 1080

# Optional: declare which side the other machine is on. Without this,
# cross-control discovers the peer but doesn't know where to hand off the cursor.
[[screens]]
name = "laptop"
position = "Right"
# no `address =` — discovery fills it in
# `fingerprint =` is optional; if set, refuses connections that don't match
```

### 3. Start both daemons

```bash
cross-control start
```

Within a few seconds each daemon should report finding the other. Confirm:

```bash
cross-control status
```

### 4. First-contact fingerprint check (TOFU)

Each daemon's mDNS advertisement carries the local cert fingerprint in the `fp` TXT record. When two daemons first see each other, compare the discovered fingerprints to the ones you wrote down in step 1. If they match, pin them by adding to the config:

```toml
[[screens]]
name = "laptop"
position = "Right"
fingerprint = "ab12cd34..."   # what you verified
```

Once pinned, **any future certificate mismatch refuses the connection** with an SSH-style identity-changed error. This is the trust-on-first-use model — see [ADR 0002](adr/0002-tofu-pairing.md) for the full threat model and what TOFU does *not* protect against.

### 5. Use it

Move the cursor to the configured edge. Press **Ctrl+Shift+Escape** to release.

## Static Config: Two Linux Machines

Use this when mDNS is blocked, or when you want the network address fixed. This example sets up a workstation (left) and laptop (right).

### 1. Generate certificates on both machines

```bash
cross-control generate-cert --output ~/.config/cross-control/
```

Note the fingerprint printed on each machine. You'll use the *other* machine's fingerprint in your config.

### 2. Create configuration

**Workstation** (192.168.1.10):

```toml
# ~/.config/cross-control/config.toml

[daemon]
port = 24800

[identity]
name = "workstation"

[daemon]
screen_width = 1920
screen_height = 1080

[[screens]]
name = "laptop"
address = "192.168.1.20:24800"
position = "Right"
fingerprint = "SHA256:ab:cd:ef:..."  # laptop's fingerprint from step 1
```

**Laptop** (192.168.1.20):

```toml
# ~/.config/cross-control/config.toml

[daemon]
port = 24800

[identity]
name = "laptop"

[daemon]
screen_width = 1920
screen_height = 1080

[[screens]]
name = "workstation"
address = "192.168.1.10:24800"
position = "Left"
fingerprint = "SHA256:12:34:56:..."  # workstation's fingerprint from step 1
```

### 3. Start the daemon on both machines

```bash
cross-control start
```

### 4. Use it

- Move your cursor to the **right edge** of the workstation screen — it appears on the laptop
- Move the cursor to the **left edge** of the laptop screen — it returns to the workstation
- Press **Ctrl+Shift+Escape** to immediately release input back to the local machine

### 5. Check status

```bash
cross-control status
```

## Three-Machine Setup

For three or more machines, use `screen_adjacency` to define the full layout:

```toml
# On the workstation (center machine)
[identity]
name = "workstation"

[[screens]]
name = "laptop"
address = "192.168.1.20:24800"
position = "Right"
fingerprint = "SHA256:..."

[[screens]]
name = "desktop"
address = "192.168.1.30:24800"
position = "Left"
fingerprint = "SHA256:..."

# Define that laptop's right neighbor is desktop (cursor wraps around)
[[screen_adjacency]]
screen = "laptop"
neighbor = "desktop"
position = "Right"
```

## systemd User Service

Run cross-control as a background service that starts on login:

```bash
# Create service directory
mkdir -p ~/.config/systemd/user/

# Create service file
cat > ~/.config/systemd/user/cross-control.service <<EOF
[Unit]
Description=cross-control virtual KVM daemon
After=network.target

[Service]
Type=simple
ExecStart=$(which cross-control) start
Restart=on-failure
RestartSec=5

[Install]
WantedBy=default.target
EOF

# Enable and start
systemctl --user daemon-reload
systemctl --user enable --now cross-control

# Check logs
journalctl --user -u cross-control -f
```

## Troubleshooting

### "no keyboard or mouse devices found"

Your user cannot read `/dev/input/event*` devices.

**Fix**: Add your user to the `input` group:
```bash
sudo usermod -aG input $USER
# Log out and back in
```

### "permission denied reading /dev/input/"

Same as above — the `input` group is required.

### "failed to create virtual device"

Cannot write to `/dev/uinput`.

**Fix**: Load the uinput module and set permissions:
```bash
sudo modprobe uinput
echo 'KERNEL=="uinput", MODE="0660", GROUP="input"' | sudo tee /etc/udev/rules.d/99-uinput.rules
sudo udevadm control --reload-rules && sudo udevadm trigger
```

### Connection refused or timeout

- Check that the daemon is running on both machines: `cross-control status`
- Check firewall allows UDP port 24800
- Verify the IP addresses in config are correct and reachable: `ping 192.168.1.20`
- Check that both machines are on the same network

### Cursor doesn't switch

- Ensure screen positions match: if machine A has machine B on the "Right", machine B should have machine A on the "Left"
- Check that `screen_width` and `screen_height` in config match your actual display resolution
- Move the cursor firmly to the screen edge

### High latency

- cross-control uses QUIC (UDP) for low latency. If you're on WiFi, try a wired connection
- Check network latency: `ping -c 10 <other-machine>` should be < 5ms on LAN

### Daemon crashes on start

Check logs with:
```bash
RUST_LOG=debug cross-control start
```

Or if using systemd:
```bash
journalctl --user -u cross-control --no-pager -n 50
```
