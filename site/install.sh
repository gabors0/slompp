#!/bin/sh
# SLOPP installer for Linux and macOS.
#
#   curl -fsSL https://slopp.gs0.me/install.sh | sh
#
# It installs SLOPP from the latest release (the AppImage into ~/.local/bin
# on Linux, SLOPP.app into Applications on macOS), checked against the
# release's SHA-256 digests, then has sloppctl install the servers it drives
# that are missing, the same way the panel's Install button does: Apache,
# PHP and MariaDB with apt, dnf or pacman (through pkexec, which asks for
# your password) or Homebrew, and on Linux Mailpit from its GitHub release
# into the stack, since the distributions don't package it. Running it
# again updates SLOPP. Options go after `sh -s --`, as in
# `curl -fsSL https://slopp.gs0.me/install.sh | sh -s -- --no-deps`:
#
#   --no-deps        install SLOPP only, leave the servers alone
#   --yes, -y        don't ask before installing or uninstalling
#   --version X.Y.Z  install a specific release instead of the latest
#   --dir DIR        where the AppImage goes on Linux (default ~/.local/bin)
#   --uninstall      stop the servers and remove SLOPP; the stack (htdocs,
#                    databases) and the settings stay
#
# Also honoured: SLOPP_VERSION as the same as --version.

set -eu

REPO=gabors0/slopp
API="https://api.github.com/repos/$REPO/releases"
# The Linux builds need the glibc of the release workflow's runner (Ubuntu
# 24.04) or newer.
GLIBC_MIN=2.39

if [ -t 1 ]; then
    bold=$(printf '\033[1m')
    dim=$(printf '\033[2m')
    off=$(printf '\033[0m')
else
    bold='' dim='' off=''
fi

say() { printf '%s\n' "${bold}==> $*${off}"; }
note() { printf '%s\n' "${dim}$*${off}"; }
warn() { printf 'warning: %s\n' "$*" >&2; }
die() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}
have() { command -v "$1" >/dev/null 2>&1; }

usage() {
    cat <<'EOF'
Usage: install.sh [options]

  --no-deps        install SLOPP only, leave the servers alone
  --yes, -y        don't ask before installing or uninstalling
  --version X.Y.Z  install a specific release instead of the latest
  --dir DIR        where the AppImage goes on Linux (default ~/.local/bin)
  --uninstall      stop the servers and remove SLOPP (the stack and the
                   settings stay)
EOF
}

action=install
version=
dir=
deps=yes
assume_yes=no

while [ $# -gt 0 ]; do
    case "$1" in
        --no-deps) deps=no ;;
        --yes | -y) assume_yes=yes ;;
        --uninstall) action=uninstall ;;
        --dir | --version)
            [ $# -ge 2 ] || die "$1 needs a value"
            case "$1" in
                --dir) dir=$2 ;;
                *) version=$2 ;;
            esac
            shift
            ;;
        --dir=*) dir=${1#*=} ;;
        --version=*) version=${1#*=} ;;
        -h | --help)
            usage
            exit 0
            ;;
        *)
            printf 'unknown option: %s\n\n' "$1" >&2
            usage >&2
            exit 2
            ;;
    esac
    shift
done

if [ -z "$version" ] && [ -n "${SLOPP_VERSION:-}" ]; then
    version=$SLOPP_VERSION
fi
version=${version#v}

# Prompts on the user's terminal, since stdin is the script under `curl | sh`.
# Without a terminal it goes ahead.
confirm() {
    [ "$assume_yes" = yes ] && return 0
    if [ -t 0 ]; then
        printf '%s [Y/n] ' "$1"
        read -r reply
    elif [ -r /dev/tty ] && { : </dev/tty; } 2>/dev/null; then
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

sha256_of() {
    if have sha256sum; then
        sha256sum "$1" | cut -d ' ' -f 1
    elif have shasum; then
        shasum -a 256 "$1" | cut -d ' ' -f 1
    else
        die "sha256sum or shasum is required to check the download"
    fi
}

# Whether version $1 is at least $2 (dotted numbers).
version_ge() {
    awk -v a="$1" -v b="$2" 'BEGIN {
        n = split(a, x, "."); m = split(b, y, ".")
        for (i = 1; i <= (n > m ? n : m); i++) {
            if (x[i] + 0 > y[i] + 0) exit 0
            if (x[i] + 0 < y[i] + 0) exit 1
        }
        exit 0
    }'
}

os=$(uname -s)
arch=$(uname -m)
case "$os" in
    Linux) [ "$arch" = x86_64 ] || die "Linux builds are x86_64 only" ;;
    Darwin) [ "$arch" = arm64 ] || die "macOS builds are Apple silicon only" ;;
    *) die "unsupported system: $os" ;;
esac

# The release's JSON, one key per line whether or not GitHub pretty-prints
# it.
json=
find_release() {
    say "Finding SLOPP"
    if [ -n "$version" ]; then
        json=$(fetch_stdout "$API/tags/v$version") || die "release v$version not found"
    else
        json=$(fetch_stdout "$API/latest") || die "could not reach GitHub"
    fi
    # shellcheck disable=SC2020 # three characters to three newlines
    json=$(printf '%s\n' "$json" | tr ',{}' '\n\n\n')
    tag=$(printf '%s\n' "$json" | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -n 1)
    [ -n "$tag" ] || die "could not read the release"
    ver=${tag#v}
    note "release $tag"
}

# The release asset whose file name ends in $1, and its SHA-256 digest.
asset_url() {
    printf '%s\n' "$json" | grep -o "https://[^\"]*$1\"" | tr -d '"' | head -n 1
}
asset_digest() {
    printf '%s\n' "$json" | awk -v suffix="$1\"" '
        /"name":/ { digest = "" }
        /"digest": *"sha256:/ {
            digest = $0
            sub(/.*"sha256:/, "", digest)
            sub(/".*/, "", digest)
        }
        /"browser_download_url":/ && index($0, suffix) { print digest; exit }
    '
}

# Downloads the asset ending in $1 to $2 and checks it against the release's
# digest; a release without one is refused.
download() {
    url=$(asset_url "$1")
    [ -n "$url" ] || die "release $tag has no $1"
    digest=$(asset_digest "$1")
    [ -n "$digest" ] || die "release $tag has no checksum for $1"
    fetch "$url" "$2" || die "could not download $url"
    actual=$(sha256_of "$2")
    if [ "$actual" != "$digest" ]; then
        rm -f "$2"
        die "the download doesn't match the release's checksum (expected $digest, got $actual)"
    fi
}

# Is the panel running? Updating or removing it underneath works, but the
# running one stays the old one.
panel_running() {
    have pgrep && pgrep -x slopp >/dev/null 2>&1
}

# sloppctl from the installed SLOPP, without the rest of the script on its
# stdin. $ctl_bin is its path; $extract is set when the AppImage can only run by
# unpacking itself (no FUSE).
ctl_bin=
extract=
ctl() {
    if [ -n "$extract" ]; then
        APPIMAGE_EXTRACT_AND_RUN=1 "$ctl_bin" "$@" </dev/null
    else
        "$ctl_bin" "$@" </dev/null
    fi
}

# Whether sloppctl runs here; an AppImage that needs FUSE but has none
# gets to unpack itself instead.
ctl_works() {
    extract=
    ctl help >/dev/null 2>&1 && return 0
    [ "$os" = Linux ] || return 1
    extract=1
    if ctl help >/dev/null 2>&1; then
        warn "FUSE isn't available, so the AppImage unpacks itself on every start; install fuse3 (fusermount3) to run it normally"
        return 0
    fi
    extract=
    return 1
}

# A line of `sloppctl paths`: "Stack root", "Settings".
ctl_path() {
    ctl paths 2>/dev/null | sed -n "s/^$1  *//p"
}

# The servers sloppctl doesn't find, as "Apache, MariaDB and Mailpit".
missing_servers() {
    out=$(ctl paths) || return 1
    printf '%s\n' "$out" | awk '
        $NF != "found" || $(NF - 1) != "not" { next }
        $1 == "httpd" { m = m ", Apache" }
        $1 == "PHP" { m = m ", PHP" }
        $1 == "mariadbd" { m = m ", MariaDB" }
        $1 == "mailpit" { m = m ", Mailpit" }
        END {
            sub(/^, /, "", m)
            if (match(m, /, [^,]*$/)) m = substr(m, 1, RSTART - 1) " and " substr(m, RSTART + 2)
            print m
        }
    '
}

# sloppctl installs what's missing, as the panel's Install button does, so
# the package names, Debian's services, the advice for NixOS and
# image-based systems and Mailpit's download live in one place.
install_servers() {
    [ "$deps" = yes ] || return 0
    if ! missing=$(missing_servers); then
        warn "sloppctl doesn't run here, so the servers weren't checked"
        return 0
    fi
    if [ -z "$missing" ]; then
        note "the servers are already installed"
        return 0
    fi
    if ! confirm "Install $missing?"; then
        note "skipping the servers; SLOPP's Install button gets them later"
        return 0
    fi
    say "Installing $missing"
    ctl install-servers && return 0
    # Mailpit's download doesn't need the package manager, which may be
    # what failed (an image-based system, a dismissed password prompt).
    if [ "$os" = Linux ]; then
        case "$(missing_servers || true)" in
            *Mailpit*) ctl install-servers mailpit || true ;;
        esac
    fi
    warn "not everything was installed (see above); SLOPP's Install button can try again"
}

nixos_advice() {
    warn "the release builds don't run on NixOS by themselves"
    note "use the flake, which brings the servers along:"
    note "  nix profile install github:$REPO"
    note "or try it with: nix run github:$REPO"
}

install_linux() {
    if [ -e /etc/NIXOS ]; then
        nixos_advice
        exit 1
    fi
    glibc=$(getconf GNU_LIBC_VERSION 2>/dev/null || true)
    glibc=${glibc#glibc }
    [ -n "$glibc" ] || die "the Linux build needs glibc, and this system has another C library"
    version_ge "$glibc" "$GLIBC_MIN" ||
        die "the Linux build needs glibc $GLIBC_MIN or newer, and this system has $glibc"

    find_release
    dir=${dir:-$HOME/.local/bin}
    mkdir -p "$dir"
    appimage="$dir/slopp.AppImage"
    was_running=no
    panel_running && was_running=yes

    say "Downloading SLOPP $ver"
    # Downloaded beside the old one and moved over it, so a failed download
    # leaves it alone and a running SLOPP keeps its file.
    part=$(mktemp "$dir/.slopp.AppImage.XXXXXX")
    trap 'rm -f "$part"' EXIT
    download "linux-x86_64.AppImage" "$part"
    chmod 755 "$part"
    mv -f "$part" "$appimage"
    # A symlink to the AppImage named sloppctl runs sloppctl (see AppRun).
    ln -sf slopp.AppImage "$dir/slopp"
    ln -sf slopp.AppImage "$dir/sloppctl"
    say "Installed $appimage"

    ctl_bin="$dir/sloppctl"
    if ctl_works; then
        install_servers
    else
        warn "the AppImage doesn't run here; try $appimage from a terminal to see why"
    fi

    case ":$PATH:" in
        *":$dir:"*) ;;
        *)
            warn "$dir is not on your PATH"
            note "add it in your shell's profile: export PATH=\"$dir:\$PATH\""
            ;;
    esac
    if [ "$was_running" = yes ]; then
        note "SLOPP is running: quit it from its tray menu and start it again to use $ver"
    else
        note "run 'slopp' to open the panel, 'sloppctl status' from a terminal"
        if [ -n "${DISPLAY:-}${WAYLAND_DISPLAY:-}" ] && confirm "Open SLOPP now?"; then
            if [ -n "$extract" ]; then
                APPIMAGE_EXTRACT_AND_RUN=1 "$appimage" </dev/null >/dev/null 2>&1 &
            else
                "$appimage" </dev/null >/dev/null 2>&1 &
            fi
        fi
    fi
}

# The disk image's mount point and the half-copied app, for the exit trap.
tmp=
mnt=
staged=
cleanup_macos() {
    [ -n "$mnt" ] && hdiutil detach "$mnt" -quiet 2>/dev/null
    [ -n "$staged" ] && rm -rf "$staged"
    [ -n "$tmp" ] && rm -rf "$tmp"
    return 0
}

install_macos() {
    find_release
    was_running=no
    panel_running && was_running=yes

    say "Downloading SLOPP $ver"
    tmp=$(mktemp -d)
    trap cleanup_macos EXIT
    download "macos-arm64.dmg" "$tmp/SLOPP.dmg"
    mkdir -p "$tmp/mnt"
    hdiutil attach "$tmp/SLOPP.dmg" -mountpoint "$tmp/mnt" -nobrowse -quiet
    mnt="$tmp/mnt"
    dest=/Applications
    if [ ! -w "$dest" ]; then
        dest=$HOME/Applications
        mkdir -p "$dest"
        note "/Applications needs admin rights; using $dest"
    fi
    # Copied beside the old app and swapped in, so a failed copy leaves it.
    staged="$dest/.SLOPP.app.new"
    rm -rf "$staged"
    cp -R "$mnt/SLOPP.app" "$staged"
    hdiutil detach "$mnt" -quiet
    mnt=
    rm -rf "$dest/SLOPP.app"
    mv "$staged" "$dest/SLOPP.app"
    staged=
    # curl doesn't set the quarantine flag, but a browser download would.
    xattr -dr com.apple.quarantine "$dest/SLOPP.app" 2>/dev/null || true
    say "Installed $dest/SLOPP.app"

    ctl_bin="$dest/SLOPP.app/Contents/MacOS/sloppctl"
    if ctl_works; then
        install_servers
    else
        warn "sloppctl doesn't run; try $ctl_bin from a terminal to see why"
    fi

    if [ "$was_running" = yes ]; then
        note "SLOPP is running: quit it and open it again to use $ver"
    elif confirm "Open SLOPP now?"; then
        open "$dest/SLOPP.app"
    fi
}

# Stops the servers (they outlive the panel) and says where the stack and
# the settings stay.
stop_and_report() {
    if ! ctl_works; then
        warn "sloppctl doesn't run, so the servers weren't stopped"
        return 0
    fi
    root=$(ctl_path "Stack root")
    settings=$(ctl_path "Settings")
    say "Stopping the servers"
    ctl stop || warn "not every server stopped; see 'sloppctl status' above"
    kept="${root:+the stack in $root}"
    kept="$kept${settings:+${kept:+ and }the settings in $settings}"
    [ -z "$kept" ] || note "kept $kept; delete them to remove everything"
}

uninstall_linux() {
    dir=${dir:-$HOME/.local/bin}
    appimage="$dir/slopp.AppImage"
    [ -e "$appimage" ] || die "SLOPP isn't installed in $dir (--dir points at another folder)"
    confirm "Stop the servers and remove SLOPP from $dir?" || exit 0
    ctl_bin="$dir/sloppctl"
    [ -e "$ctl_bin" ] || ctl_bin=$appimage
    stop_and_report

    rm -f "$appimage"
    for name in slopp sloppctl; do
        if [ -L "$dir/$name" ] && [ "$(readlink "$dir/$name")" = slopp.AppImage ]; then
            rm -f "$dir/$name"
        fi
    done
    # The desktop file and icon the AppImage adds on launch, if it wrote
    # them (its desktop files carry this line).
    data=${XDG_DATA_HOME:-$HOME/.local/share}
    desktop="$data/applications/org.slopp.Slopp.desktop"
    if [ -f "$desktop" ] && grep -qx 'X-SLOPP-AppImage=true' "$desktop"; then
        rm -f "$desktop" "$data/icons/hicolor/scalable/apps/org.slopp.Slopp.svg"
    fi
    say "Removed SLOPP from $dir"
    if panel_running; then
        note "SLOPP is still running: quit it from its tray menu"
    fi
}

uninstall_macos() {
    app=
    for d in /Applications "$HOME/Applications"; do
        if [ -d "$d/SLOPP.app" ]; then
            app="$d/SLOPP.app"
            break
        fi
    done
    [ -n "$app" ] || die "SLOPP.app isn't in /Applications or ~/Applications"
    confirm "Stop the servers and remove $app?" || exit 0
    ctl_bin="$app/Contents/MacOS/sloppctl"
    stop_and_report
    if panel_running; then
        note "SLOPP is running: quit it"
    fi
    rm -rf "$app"
    say "Removed $app"
}

case "$action:$os" in
    install:Linux) install_linux ;;
    install:Darwin) install_macos ;;
    uninstall:Linux) uninstall_linux ;;
    uninstall:Darwin) uninstall_macos ;;
esac

say "Done"
