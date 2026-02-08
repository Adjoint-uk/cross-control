# Setup Guide

## Installation

### From Source

```bash
git clone https://github.com/Adjoint-uk/cross-control.git
cd cross-control
cargo install --path crates/cross-control-cli
```

## Quick Start

### 1. Generate certificates on both machines

```bash
cross-control generate-cert --output ~/.config/cross-control/
```

### 2. Create configuration

Create `~/.config/cross-control/config.toml` on each machine. See `examples/config.toml` for a complete example.

**Machine A** (left workstation):

```toml
[identity]
name = "workstation"

[[screens]]
name = "laptop"
position = "Right"
```

**Machine B** (right laptop):

```toml
[identity]
name = "laptop"

[[screens]]
name = "workstation"
position = "Left"
```

### 3. Start the daemon

On both machines:

```bash
cross-control start
```

### 4. Pair the machines

On either machine:

```bash
cross-control pair <other-machine-ip>:24800
```

Verify the certificate fingerprint when prompted.

### 5. Use it

Move your cursor to the right edge of Machine A's screen. It will appear on Machine B. Move it back to the left edge to return.

Press **Ctrl+Shift+Escape** to immediately release input and return control to the local machine.

## systemd User Service (Linux)

```bash
cp systemd/cross-control.service ~/.config/systemd/user/
systemctl --user enable --now cross-control
```
