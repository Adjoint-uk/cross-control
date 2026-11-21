#!/usr/bin/env bash
# cross-control installer for Linux.
#
#   curl -fsSL https://raw.githubusercontent.com/Adjoint-uk/cross-control/main/install.sh | bash
#
# Downloads a release archive, checks it against the release's SHA256SUMS,
# installs the binary, and runs `cross-control doctor` to show what (if
# anything) still needs setting up.
#
# Environment:
#   CROSS_CONTROL_VERSION      version to install, e.g. 0.3.0 (default: latest)
#   CROSS_CONTROL_INSTALL_DIR  where to put the binary (default: ~/.local/bin)
#   CROSS_CONTROL_SERVICE      "1" to also install and start the systemd user
#                              service without asking; "0" to skip it
#   CROSS_CONTROL_DOWNLOAD_BASE  URL holding the archives and SHA256SUMS, for
#                              an internal mirror (default: the GitHub release)
set -euo pipefail

REPO="Adjoint-uk/cross-control"
WORK=""
trap '[ -n "$WORK" ] && rm -rf "$WORK"' EXIT

info()  { printf '\033[1;34m[info]\033[0m  %s\n' "$*"; }
warn()  { printf '\033[1;33m[warn]\033[0m  %s\n' "$*"; }
error() { printf '\033[1;31m[error]\033[0m %s\n' "$*" >&2; exit 1; }

need() { command -v "$1" >/dev/null 2>&1 || error "$1 is required"; }

detect_target() {
    case "$(uname -m)" in
        x86_64 | amd64) echo "x86_64-unknown-linux-gnu" ;;
        aarch64 | arm64) echo "aarch64-unknown-linux-gnu" ;;
        *) error "unsupported architecture: $(uname -m) (x86_64 and aarch64 are supported)" ;;
    esac
}

latest_version() {
    # The newest release, including pre-releases.
    curl -fsSL "https://api.github.com/repos/${REPO}/releases?per_page=1" |
        sed -n -E 's/.*"tag_name": *"v([^"]+)".*/\1/p' | head -n 1
}

sha256() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | cut -d' ' -f1
    else
        shasum -a 256 "$1" | cut -d' ' -f1
    fi
}

main() {
    [ "$(uname -s)" = Linux ] || error "this installer is for Linux; on Windows use install.ps1"
    need curl
    need tar

    local target version install_dir
    target="$(detect_target)"
    version="${CROSS_CONTROL_VERSION:-}"
    [ -n "$version" ] || version="$(latest_version)"
    version="${version#v}"
    [ -n "$version" ] || error "could not find the latest release (set CROSS_CONTROL_VERSION)"
    install_dir="${CROSS_CONTROL_INSTALL_DIR:-$HOME/.local/bin}"

    local archive="cross-control-${version}-${target}.tar.gz"
    local base="${CROSS_CONTROL_DOWNLOAD_BASE:-https://github.com/${REPO}/releases/download/v${version}}"
    WORK="$(mktemp -d)"
    local work="$WORK"

    info "Downloading cross-control ${version} for ${target}"
    curl -fsSL -o "$work/$archive" "$base/$archive" || error "download failed: $base/$archive"
    curl -fsSL -o "$work/SHA256SUMS" "$base/SHA256SUMS" || error "download failed: $base/SHA256SUMS"

    local expected actual
    expected="$(awk -v f="$archive" '$2 == f { print $1 }' "$work/SHA256SUMS")"
    [ -n "$expected" ] || error "$archive is not listed in SHA256SUMS"
    actual="$(sha256 "$work/$archive")"
    [ "$expected" = "$actual" ] || error "checksum mismatch for $archive"
    info "Checksum verified"

    tar xzf "$work/$archive" -C "$work"
    mkdir -p "$install_dir"
    install -m 0755 "$work/cross-control-${version}-${target}/cross-control" "$install_dir/cross-control"
    local exe="$install_dir/cross-control"
    info "Installed $("$exe" --version) to $exe"

    local config_dir="${XDG_CONFIG_HOME:-$HOME/.config}/cross-control"
    if [ ! -e "$config_dir/config.toml" ]; then
        mkdir -p "$config_dir"
        install -m 0600 "$work/cross-control-${version}-${target}/config.example.toml" "$config_dir/config.example.toml"
        info "Example configuration: $config_dir/config.example.toml"
    fi

    case ":$PATH:" in
        *":$install_dir:"*) ;;
        *) warn "$install_dir is not in your PATH; add: export PATH=\"$install_dir:\$PATH\"" ;;
    esac

    echo
    info "Checking this machine (cross-control doctor):"
    "$exe" doctor || true

    local service="${CROSS_CONTROL_SERVICE:-}"
    if [ -z "$service" ] && [ -t 0 ] && command -v systemctl >/dev/null 2>&1; then
        read -rp "Start cross-control at login (systemd user service)? [y/N] " answer
        if [[ "$answer" =~ ^[Yy] ]]; then service=1; fi
    fi
    if [ "$service" = 1 ]; then
        "$exe" service install --enable
    else
        info "To start at login later: cross-control service install --enable"
    fi

    echo
    info "Next: cross-control identity   (this machine's fingerprint, for pairing)"
    info "      docs: https://github.com/${REPO}/blob/main/docs/setup-guide.md"
}

main "$@"
