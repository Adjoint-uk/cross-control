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

cross-control uses UDP port 24800 by default. Open it on both machines:

```bash
# UFW (Ubuntu)
sudo ufw allow 24800/udp

# firewalld (Fedora)
sudo firewall-cmd --add-port=24800/udp --permanent
sudo firewall-cmd --reload

# iptables
sudo iptables -A INPUT -p udp --dport 24800 -j ACCEPT
```

## Quick Start: Two Linux Machines

This example sets up a workstation (left) and laptop (right).

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
