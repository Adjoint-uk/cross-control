#!/usr/bin/env bash
set -euo pipefail

# cross-control installer
# Downloads the latest release binary and sets up permissions.

REPO="Adjoint-uk/cross-control"
BIN_NAME="cross-control"

info()  { printf '\033[1;34m[info]\033[0m  %s\n' "$*"; }
warn()  { printf '\033[1;33m[warn]\033[0m  %s\n' "$*"; }
error() { printf '\033[1;31m[error]\033[0m %s\n' "$*"; exit 1; }

detect_arch() {
    local arch
    arch="$(uname -m)"
    case "$arch" in
        x86_64|amd64) echo "x86_64-unknown-linux-gnu" ;;
        aarch64|arm64) echo "aarch64-unknown-linux-gnu" ;;
        *) error "Unsupported architecture: $arch. Only x86_64 and aarch64 are supported." ;;
    esac
}

detect_install_dir() {
    if [ -d "$HOME/.cargo/bin" ]; then
        echo "$HOME/.cargo/bin"
    elif [ -d "$HOME/.local/bin" ]; then
        echo "$HOME/.local/bin"
    else
        mkdir -p "$HOME/.local/bin"
        echo "$HOME/.local/bin"
    fi
}

get_latest_version() {
    curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" \
        | grep '"tag_name"' \
        | sed -E 's/.*"tag_name": *"([^"]+)".*/\1/'
}

download_binary() {
    local version="$1"
    local target="$2"
    local install_dir="$3"
    local url="https://github.com/${REPO}/releases/download/${version}/${BIN_NAME}-${target}"

    info "Downloading cross-control ${version} for ${target}..."
    curl -fsSL -o "${install_dir}/${BIN_NAME}" "$url"
    chmod +x "${install_dir}/${BIN_NAME}"
    info "Installed to ${install_dir}/${BIN_NAME}"
}

check_permissions() {
    local issues=0

    # Check input group membership
    if ! groups | grep -qw input; then
        warn "Your user is not in the 'input' group."
        warn "  Fix: sudo usermod -aG input \$USER"
        warn "  Then log out and back in."
        issues=$((issues + 1))
    fi

    # Check /dev/uinput
    if [ -e /dev/uinput ]; then
        if ! [ -r /dev/uinput ] || ! [ -w /dev/uinput ]; then
            warn "/dev/uinput is not readable/writable by your user."
            warn "  Fix: sudo modprobe uinput"
            warn "  echo 'KERNEL==\"uinput\", MODE=\"0660\", GROUP=\"input\"' | sudo tee /etc/udev/rules.d/99-uinput.rules"
            warn "  sudo udevadm control --reload-rules && sudo udevadm trigger"
            issues=$((issues + 1))
        fi
    else
        warn "/dev/uinput not found."
        warn "  Fix: sudo modprobe uinput"
        issues=$((issues + 1))
    fi

    # Check /dev/input readability
    if ! ls /dev/input/event* &>/dev/null; then
        warn "Cannot access /dev/input/event* devices."
        warn "  This may be a permissions issue (see input group fix above)."
        issues=$((issues + 1))
    fi

    if [ "$issues" -eq 0 ]; then
        info "Permissions look good."
    else
        echo ""
        warn "$issues permission issue(s) found. cross-control may not work correctly until fixed."
    fi
}

setup_systemd() {
    local service_dir="$HOME/.config/systemd/user"
    local install_dir="$1"

    read -rp "Set up systemd user service? [y/N] " answer
    if [[ ! "$answer" =~ ^[Yy] ]]; then
        return
    fi

    mkdir -p "$service_dir"
    cat > "${service_dir}/cross-control.service" <<UNIT
[Unit]
Description=cross-control virtual KVM daemon
After=network.target

[Service]
Type=simple
ExecStart=${install_dir}/${BIN_NAME} start
Restart=on-failure
RestartSec=5

[Install]
WantedBy=default.target
UNIT

    systemctl --user daemon-reload
    info "Systemd service installed at ${service_dir}/cross-control.service"
    info "  Enable: systemctl --user enable --now cross-control"
}

main() {
    info "cross-control installer"
    echo ""

    # Check OS
    if [ "$(uname -s)" != "Linux" ]; then
        error "cross-control currently only supports Linux."
    fi

    local target
    target="$(detect_arch)"
    info "Detected target: ${target}"

    local install_dir
    install_dir="$(detect_install_dir)"
    info "Install directory: ${install_dir}"

    local version
    version="$(get_latest_version)"
    if [ -z "$version" ]; then
        error "Could not determine latest release version. Check your internet connection."
    fi
    info "Latest version: ${version}"

    echo ""
    download_binary "$version" "$target" "$install_dir"

    echo ""
    check_permissions

    echo ""
    setup_systemd "$install_dir"

    echo ""
    info "Installation complete!"

    # Check PATH
    if ! echo "$PATH" | tr ':' '\n' | grep -qx "$install_dir"; then
        warn "${install_dir} is not in your PATH."
        warn "  Add to your shell profile: export PATH=\"${install_dir}:\$PATH\""
    fi

    echo ""
    info "Next steps:"
    info "  1. cross-control generate-cert --output ~/.config/cross-control/"
    info "  2. Create ~/.config/cross-control/config.toml"
    info "  3. cross-control start"
}

main "$@"
