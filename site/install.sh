#!/bin/sh
# SLOPP installer for Linux and macOS.
#
#   curl -fsSL https://slopp.gs0.me/install.sh | sh
#
# It installs the servers SLOPP drives (Apache, PHP, MariaDB and Mailpit)
# with your system's package manager, then installs SLOPP itself from the
# latest release. The Linux distributions don't package Mailpit, so there
# SLOPP downloads it from GitHub into its stack once it's installed. Options go after `sh -s --`, as in
# `curl -fsSL https://slopp.gs0.me/install.sh | sh -s -- --no-deps`:
#
#   --no-deps        install SLOPP only, leave the servers alone
#   --yes, -y        don't ask before installing packages
#   --version X.Y.Z  install a specific release instead of the latest
#   --dir DIR        where the AppImage goes on Linux (default ~/.local/bin)
#
# Also honoured: SLOPP_VERSION as the same as --version.

set -eu

REPO=gabors0/slopp
API="https://api.github.com/repos/$REPO/releases"

version=
dir=
deps=yes
assume_yes=no

while [ $# -gt 0 ]; do
    case "$1" in
        --no-deps) deps=no ;;
        --yes | -y) assume_yes=yes ;;
        --dir)
            dir=$2
            shift
            ;;
        --dir=*) dir=${1#*=} ;;
        --version)
            version=$2
            shift
            ;;
        --version=*) version=${1#*=} ;;
        -h | --help)
            printf '%s\n' "Usage: install.sh [--no-deps] [--yes] [--version X.Y.Z] [--dir DIR]"
            exit 0
            ;;
        *)
            printf 'unknown option: %s\n' "$1" >&2
            exit 2
            ;;
    esac
    shift
done

if [ -z "$version" ] && [ -n "${SLOPP_VERSION:-}" ]; then
    version=$SLOPP_VERSION
fi

if [ -t 1 ]; then
    bold=$(printf '\033[1m')
    dim=$(printf '\033[2m')
    off=$(printf '\033[0m')
else
    bold= dim= off=
fi

say() { printf '%s\n' "${bold}==> $*${off}"; }
note() { printf '%s\n' "${dim}$*${off}"; }
warn() { printf 'warning: %s\n' "$*" >&2; }
die() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}
have() { command -v "$1" >/dev/null 2>&1; }

# Prompts on the user's terminal, since stdin is the script under `curl | sh`.
confirm() {
    [ "$assume_yes" = yes ] && return 0
    if [ -t 0 ]; then
        printf '%s [Y/n] ' "$1"
        read -r reply
    elif [ -r /dev/tty ]; then
        printf '%s [Y/n] ' "$1" >/dev/tty
        read -r reply </dev/tty
    else
        return 0
    fi
    case "$reply" in [Nn]*) return 1 ;; *) return 0 ;; esac
}

fetch_stdout() {
    if have curl; then
        curl -fsSL --retry 3 "$1"
    elif have wget; then
        wget -q -O - "$1"
    else
        die "curl or wget is required"
    fi
}

fetch() {
    if have curl; then
        curl -fL --retry 3 --progress-bar -o "$2" "$1"
    else
        wget -q --show-progress -O "$2" "$1"
    fi
}

os=$(uname -s)
arch=$(uname -m)
case "$os" in
    Linux) [ "$arch" = x86_64 ] || die "Linux builds are x86_64 only" ;;
    Darwin) [ "$arch" = arm64 ] || die "macOS builds are Apple silicon only" ;;
    *) die "unsupported system: $os" ;;
esac

# The latest (or requested) release.
say "Finding SLOPP"
if [ -n "$version" ]; then
    json=$(fetch_stdout "$API/tags/v$version") || die "release v$version not found"
else
    json=$(fetch_stdout "$API/latest") || die "could not reach GitHub"
fi
tag=$(printf '%s' "$json" | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -n1)
[ -n "$tag" ] || die "could not read the release"
ver=${tag#v}
note "release $tag"

asset_url() {
    printf '%s' "$json" | grep -o "https://[^\"]*$1" | head -n1 || true
}

# Like have(), but also looks in the usual sbin dirs, which a normal PATH
# leaves out (mariadbd lives there on most distributions).
found() {
    for name in "$@"; do
        have "$name" && return 0
        for d in /usr/sbin /usr/local/sbin /opt/homebrew/bin /opt/homebrew/sbin; do
            [ -x "$d/$name" ] && return 0
        done
    done
    return 1
}

# Is a set of servers already there?
servers_present() {
    found httpd apache2 httpd2 || return 1
    found php || return 1
    found mariadbd mysqld mysqld_safe mariadbd-safe || return 1
    return 0
}

as_root=
if [ "$(id -u)" != 0 ]; then
    if have sudo; then
        as_root=sudo
    else
        warn "not root and no sudo: skipping the package installs"
    fi
fi

install_linux_deps() {
    [ "$deps" = yes ] || return 0
    [ -n "$as_root" ] || return 0
    if servers_present; then
        note "Apache, PHP and MariaDB are already installed"
        return 0
    fi
    if [ -e /etc/NIXOS ]; then
        warn "NixOS: use 'nix profile install github:gabors0/slopp' instead"
        return 0
    fi
    if have apt-get; then
        packages="apache2 php-fpm php-mysql mariadb-server"
        set -- apt-get install -y
    elif have dnf; then
        packages="httpd php-fpm php-mysqlnd mariadb-server"
        set -- dnf install -y
    elif have pacman; then
        packages="apache php-fpm mariadb"
        set -- pacman -S --needed --noconfirm
    elif have zypper; then
        packages="apache2 php8 php8-fpm php8-mysql mariadb"
        set -- zypper install -y
    else
        warn "unknown package manager: install Apache, PHP and MariaDB yourself"
        return 0
    fi
    if confirm "Install the servers ($packages)?"; then
        say "Installing $packages"
        $as_root "$@" $packages || warn "the package install failed; install Apache, PHP and MariaDB yourself"
    else
        note "skipping the servers"
    fi
}

install_linux() {
    install_linux_deps

    say "Downloading SLOPP $ver"
    dir=${dir:-$HOME/.local/bin}
    mkdir -p "$dir"
    url=$(asset_url "linux-x86_64.AppImage")
    [ -n "$url" ] || die "release $tag has no AppImage"
    appimage="$dir/slopp.AppImage"
    fetch "$url" "$appimage"
    chmod +x "$appimage"
    # A symlink to the AppImage named sloppctl runs sloppctl (see AppRun).
    ln -sf slopp.AppImage "$dir/slopp"
    ln -sf slopp.AppImage "$dir/sloppctl"
    say "Installed $appimage"
    install_linux_mailpit
    case ":$PATH:" in
        *":$dir:"*) ;;
        *)
            warn "$dir is not on your PATH"
            note "add: export PATH=\"$dir:\$PATH\""
            ;;
    esac
    note "run 'slopp' to open the panel, 'sloppctl status' from a terminal"
    if [ -n "${DISPLAY:-}${WAYLAND_DISPLAY:-}" ] && confirm "Open SLOPP now?"; then
        "$dir/slopp" >/dev/null 2>&1 &
    fi
}

# sloppctl downloads Mailpit's static binary from its GitHub release into the
# stack, where SLOPP finds it; no root needed. The stack is in its default
# place unless the panel's settings moved it.
install_linux_mailpit() {
    [ "$deps" = yes ] || return 0
    stack=${SLOPP_ROOT:-${XDG_DATA_HOME:-$HOME/.local/share}/slopp}
    if found mailpit || [ -x "$stack/mailpit/bin/mailpit" ]; then
        note "Mailpit is already installed"
        return 0
    fi
    if confirm "Download Mailpit from GitHub?"; then
        say "Downloading Mailpit"
        "$dir/sloppctl" install-servers mailpit ||
            warn "could not get Mailpit; SLOPP's Install button can try again"
    else
        note "skipping Mailpit"
    fi
}

install_macos_deps() {
    [ "$deps" = yes ] || return 0
    formulae=
    servers_present || formulae="php mariadb"
    found mailpit || formulae="${formulae:+$formulae }mailpit"
    if [ -z "$formulae" ]; then
        note "PHP, MariaDB and Mailpit are already installed"
        return 0
    fi
    if ! have brew; then
        warn "Homebrew is not installed; get it from https://brew.sh"
        note "then run: brew install $formulae"
        return 0
    fi
    if confirm "Install $formulae with Homebrew?"; then
        say "Installing $formulae"
        brew install $formulae || warn "brew install failed; install $formulae yourself"
    else
        note "skipping the servers"
    fi
}

install_macos() {
    install_macos_deps

    say "Downloading SLOPP $ver"
    tmp=$(mktemp -d)
    trap 'rm -rf "$tmp"' EXIT
    url=$(asset_url "macos-arm64.dmg")
    [ -n "$url" ] || die "release $tag has no disk image"
    fetch "$url" "$tmp/SLOPP.dmg"
    mnt="$tmp/mnt"
    mkdir -p "$mnt"
    hdiutil attach "$tmp/SLOPP.dmg" -mountpoint "$mnt" -nobrowse -quiet
    dest=/Applications
    if [ ! -w "$dest" ]; then
        dest=$HOME/Applications
        mkdir -p "$dest"
        note "/Applications needs admin rights; using $dest"
    fi
    rm -rf "$dest/SLOPP.app"
    cp -R "$mnt/SLOPP.app" "$dest/"
    hdiutil detach "$mnt" -quiet
    # curl doesn't set the quarantine flag, but a browser download would.
    xattr -dr com.apple.quarantine "$dest/SLOPP.app" 2>/dev/null || true
    say "Installed $dest/SLOPP.app"
    if confirm "Open SLOPP now?"; then
        open "$dest/SLOPP.app"
    fi
}

case "$os" in
    Linux) install_linux ;;
    Darwin) install_macos ;;
esac

say "Done"
