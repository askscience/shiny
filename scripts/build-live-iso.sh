#!/usr/bin/env bash
# Build the PEAK'D! live installer ISO without root.
#
# The base is the official Debian 13 XFCE live image. We remaster its squashfs,
# add PEAK'D!, Qt6, the installer helper and the T2 kernel/userspace setup, then
# rebuild the bootable image with GRUB. Nothing on the host is installed and no
# running service is touched.
set -euo pipefail

REPO=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
BUILD=${PEAKD_ISO_BUILD:-$HOME/peakd-iso}
BASE_ISO=${PEAKD_BASE_ISO:-$BUILD/base.iso}
OUTDIR=${PEAKD_ISO_OUT:-$BUILD/out}
ISO_NAME=peakd-live-t2-amd64.iso
LIVE_URL=${PEAKD_LIVE_URL:-https://cdimage.debian.org/debian-cd/current-live/amd64/iso-hybrid/debian-live-13.7.0-amd64-xfce.iso}
T2_KVER=${T2_KVER:-$(uname -r)}
PKG_VERSION=${PEAKD_VERSION:-0.1.1}
DEB_BUILD=${PEAKD_DEB_BUILD:-$BUILD/deb}
APT_KEYRING=${PEAKD_KEYRING:-$REPO/packaging/peakd-archive.gpg}
APT_URL=https://askscience.github.io/shiny/apt

TOOLS="$BUILD/tools"
DEBS="$BUILD/debs"
ISO_BASE="$BUILD/iso-base"
ROOTFS="$BUILD/rootfs"
ISO_STAGE="$BUILD/iso-stage"
OUT="$OUTDIR/$ISO_NAME"

log() { printf '\n== %s ==\n' "$*"; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

[ "$(id -u)" -eq 0 ] && die "run this as your normal user; it is designed to build rootless"
mkdir -p "$BUILD" "$DEBS" "$TOOLS" "$OUTDIR"

install_tools() {
    local pkg
    for pkg in squashfs-tools xorriso libisoburn1t64 libisofs6t64 libburn4t64 libjte2 mtools grub-pc-bin grub-efi-amd64-bin; do
        compgen -G "$DEBS/${pkg}_*.deb" >/dev/null && continue
        (cd "$DEBS" && apt-get download "$pkg" >/dev/null 2>&1) || true
    done
    local deb
    for deb in "$DEBS"/*.deb; do
        [ -f "$deb" ] || continue
        dpkg-deb -x "$deb" "$TOOLS"
    done
    export PATH="$TOOLS/usr/bin:/usr/sbin:/sbin:$PATH"
    export LD_LIBRARY_PATH="$TOOLS/usr/lib/x86_64-linux-gnu${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
    command -v mksquashfs >/dev/null || die "mksquashfs not available"
    command -v unsquashfs >/dev/null || die "unsquashfs not available"
    command -v xorriso >/dev/null || die "xorriso not available"
    command -v mformat >/dev/null || die "mformat not available"
    command -v grub-mkrescue >/dev/null || die "grub-mkrescue not available"
}

download_base() {
    if [ -s "$BASE_ISO" ]; then
        log "using existing base ISO: $BASE_ISO"
        return
    fi
    log "downloading Debian live XFCE ISO"
    curl -fL --retry 3 --retry-delay 5 -o "$BASE_ISO.part" "$LIVE_URL"
    mv "$BASE_ISO.part" "$BASE_ISO"
}

extract_base() {
    [ -s "$BASE_ISO" ] || die "missing base ISO: $BASE_ISO"
    log "extracting base ISO"
    chmod -R u+w "$ISO_BASE" 2>/dev/null || true
    rm -rf "$ISO_BASE"
    xorriso -osirrox on -indev "$BASE_ISO" -extract / "$ISO_BASE" >/dev/null
    local squash
    squash=$(find "$ISO_BASE/live" -maxdepth 1 -name 'filesystem.squashfs' -print -quit)
    [ -n "$squash" ] || die "base ISO has no live/filesystem.squashfs"
    log "unsquashing base rootfs"
    chmod -R u+w "$ROOTFS" 2>/dev/null || true
    rm -rf "$ROOTFS"
    unsquashfs -no-xattrs -d "$ROOTFS" "$squash" >/dev/null
    chmod 0755 "$ROOTFS"
}

copy_missing_libs() {
    local binary=$1 lib src dest real real_dest
    # Shared modules do not carry the executable bit, but ldd still works on
    # them and their dependencies are exactly what we need to stage.
    [ -f "$binary" ] || return 0
    while IFS= read -r lib; do
        [ -n "$lib" ] || continue
        [ -e "$lib" ] || continue
        case "$lib" in
            /lib64/*|/lib/x86_64-linux-gnu/ld-*|/usr/lib/x86_64-linux-gnu/ld-*) continue ;;
        esac
        src=$lib
        case "$src" in
            "$TOOLS"/*) dest=${src#"$TOOLS"} ;;
            *) dest=$src ;;
        esac
        if [ ! -e "$ROOTFS$dest" ]; then
            mkdir -p "$ROOTFS$(dirname "$dest")"
            cp -a "$src" "$ROOTFS$dest"
        fi
        real=$(readlink -f "$src" 2>/dev/null || true)
        [ -n "$real" ] || continue
        case "$real" in
            "$TOOLS"/*) real_dest=${real#"$TOOLS"} ;;
            *) real_dest=$real ;;
        esac
        if [ -e "$real" ] && [ ! -e "$ROOTFS$real_dest" ]; then
            mkdir -p "$ROOTFS$(dirname "$real_dest")"
            cp -a "$real" "$ROOTFS$real_dest"
        fi
    done < <(ldd "$binary" 2>/dev/null | awk '/=> \// {print $3} /^[[:space:]]*\/.*ld-linux/ {print $1}')
}

stage_app() {
    log "staging PEAK'D! application"
    install -d "$ROOTFS/usr/bin" "$ROOTFS/usr/libexec/shiny" "$ROOTFS/usr/lib/shiny/plugins" \
        "$ROOTFS/usr/share/shiny/web" "$ROOTFS/usr/share/shiny/backgrounds" \
        "$ROOTFS/usr/share/shiny/iconsets" "$ROOTFS/var/lib/shiny" "$ROOTFS/var/log/shiny"
    install -m 0755 "$REPO/target/release/shiny" "$ROOTFS/usr/bin/shiny"
    install -m 0755 "$REPO/target/release/peakd" "$ROOTFS/usr/bin/peakd"
    install -m 0755 "$REPO/target/release/shiny-installer-helper" "$ROOTFS/usr/libexec/shiny/shiny-installer-helper"
    if [ -x /usr/bin/matchbox-window-manager ]; then
        install -m 0755 /usr/bin/matchbox-window-manager "$ROOTFS/usr/bin/matchbox-window-manager"
        [ -d /etc/matchbox ] && cp -a /etc/matchbox "$ROOTFS/etc/"
        [ -d /usr/share/matchbox ] && cp -a /usr/share/matchbox "$ROOTFS/usr/share/"
        copy_missing_libs "$ROOTFS/usr/bin/matchbox-window-manager"
    fi

    cp -a "$REPO/web/." "$ROOTFS/usr/share/shiny/web/"
    [ -d "$REPO/web/backgrounds" ] && cp -a "$REPO/web/backgrounds/." "$ROOTFS/usr/share/shiny/backgrounds/"
    [ -d "$REPO/assets/iconsets" ] && cp -a "$REPO/assets/iconsets/." "$ROOTFS/usr/share/shiny/iconsets/"

    local dir name
    for dir in "$REPO"/data/plugins/*/; do
        name=$(basename "$dir")
        case "$name" in
            browser.bak|hello) continue ;;
        esac
        [ -f "$dir/plugin.toml" ] || continue
        cp -a "$dir" "$ROOTFS/usr/lib/shiny/plugins/$name"
    done

    if [ -d "$HOME/.local/share/shiny/plugins/installer" ]; then
        cp -a "$HOME/.local/share/shiny/plugins/installer" "$ROOTFS/usr/lib/shiny/plugins/installer"
    else
        install -d "$ROOTFS/usr/lib/shiny/plugins/installer"
        cp -a "$REPO/plugins/installer/plugin.toml" "$ROOTFS/usr/lib/shiny/plugins/installer/"
        cp -a "$REPO/plugins/installer/web" "$ROOTFS/usr/lib/shiny/plugins/installer/"
        cp -a "$REPO/target/release/libshiny_installer_plugin.so" "$ROOTFS/usr/lib/shiny/plugins/installer/"
    fi

    copy_missing_libs "$ROOTFS/usr/bin/shiny"
    copy_missing_libs "$ROOTFS/usr/bin/peakd"
    copy_missing_libs "$ROOTFS/usr/libexec/shiny/shiny-installer-helper"
    local so
    for so in "$ROOTFS"/usr/lib/shiny/plugins/*/*.so; do
        copy_missing_libs "$so"
    done
}

seed_live_account() {
    log "seeding the live PEAK'D! account database"
    local seed="$BUILD/seed" db="$BUILD/seed/db/shiny.db" pid i
    rm -rf "$seed"
    mkdir -p "$seed/home" "$seed/run" "$seed/plugins" "$seed/db"
    HOME="$seed/home" XDG_RUNTIME_DIR="$seed/run" \
        DATABASE_URL="sqlite://$db" SERVER_HOST=127.0.0.1 SERVER_PORT=18183 \
        LOG_FILE="$seed/shiny.log" WEB_DIR="$REPO/web" \
        PLUGINS_DIR="$seed/plugins" SYSTEM_PLUGINS_DIR="$REPO/data/plugins" \
        SHINY_AUTH_ENABLED=false SHINY_LINUX_USERS=false SHINY_HOME_MODE=virtual \
        SUPERTONIC_AUTO_INSTALL=0 WHISPER_AUTO_INSTALL=0 \
        "$REPO/target/release/shiny" >"$seed/server.out" 2>&1 &
    pid=$!
    for i in $(seq 1 80); do
        curl -sf http://127.0.0.1:18183/api/health/live >/dev/null 2>&1 && break
        sleep 0.25
    done
    curl -fsS -X POST http://127.0.0.1:18183/api/auth/register \
        -H 'Content-Type: application/json' \
        -d '{"username":"user","password":"live-installer"}' >/dev/null \
        || { kill "$pid" 2>/dev/null || true; die "could not seed the live account"; }
    kill "$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true
    [ -s "$db" ] || die "seeded live database was not created"
    install -d "$ROOTFS/usr/share/shiny/live"
    install -m 0600 "$db" "$ROOTFS/usr/share/shiny/live/seed.db"
    rm -rf "$seed"
}

build_packages() {
    log "building the peakd deb"
    (cd "$REPO" && PEAKD_DEB_BUILD="$DEB_BUILD" PEAKD_KEYRING="$APT_KEYRING" \
        PEAKD_VERSION="$PKG_VERSION" ./scripts/build-deb.sh)
}

stage_package() {
    log "installing the peakd package payload into the live rootfs"
    build_packages
    local deb="$DEB_BUILD/out/peakd_${PKG_VERSION}_amd64.deb"
    local keydeb="$DEB_BUILD/out/peakd-archive-keyring_${PKG_VERSION}_all.deb"
    [ -f "$deb" ] || die "missing $deb"
    [ -f "$keydeb" ] || die "missing $keydeb"

    rm -rf "$ROOTFS/usr/bin/shiny" "$ROOTFS/usr/bin/peakd" \
        "$ROOTFS/usr/bin/shiny-auth" "$ROOTFS/usr/bin/shiny-server-mode" \
        "$ROOTFS/usr/libexec/shiny" "$ROOTFS/usr/lib/shiny/plugins" \
        "$ROOTFS/usr/share/shiny/web" "$ROOTFS/usr/share/shiny/iconsets" \
        "$ROOTFS/usr/share/shiny/backgrounds"
    dpkg-deb -x "$deb" "$ROOTFS"
    dpkg-deb -x "$keydeb" "$ROOTFS"

    install -d "$ROOTFS/etc/apt/sources.list.d" "$ROOTFS/var/lib/shiny" "$ROOTFS/var/log/shiny" \
        "$ROOTFS/usr/share/fonts/truetype"
    rm -rf "$ROOTFS/opt/peakd/repo" "$ROOTFS/etc/apt/preferences.d/90-peakd.pref"
    install -m 0644 "$APT_KEYRING" "$ROOTFS/usr/share/keyrings/peakd-archive.gpg"
    cat > "$ROOTFS/etc/apt/sources.list.d/peakd.sources" <<EOF
Types: deb
URIs: $APT_URL/
Suites: peakd
Components: main
Signed-By: /usr/share/keyrings/peakd-archive.gpg
EOF
    if [ -d /usr/share/fonts/truetype/roboto ]; then
        cp -a /usr/share/fonts/truetype/roboto "$ROOTFS/usr/share/fonts/truetype/"
    fi
}

dpkg_in_root() {
    unshare -Ur /usr/sbin/chroot "$ROOTFS" /usr/bin/env \
        PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
        /bin/sh -c "$1"
}

register_payload() {
    log "registering peakd in the image dpkg database"
    local reg="$ROOTFS/tmp/peakd-register"
    local dlog="$BUILD/dpkg-register.log"
    local pkg
    rm -rf "$reg"
    mkdir -p "$reg/first" "$reg/deps" "$reg/main"
    for pkg in libqt6serialport6 libqt6webenginequick6 libqt6webchannelquick6 \
        libqt6positioning6-plugins fonts-roboto-unhinted; do
        local target="$reg/deps"
        [ "$pkg" = libqt6serialport6 ] && target="$reg/first"
        if ! compgen -G "$DEBS/${pkg}_*.deb" >/dev/null; then
            (cd "$DEBS" && apt-get download "$pkg" >/dev/null 2>&1) \
                || die "could not download $pkg"
        fi
        cp "$DEBS/${pkg}_"*.deb "$target/"
    done
    cp "$DEB_BUILD/out/peakd_${PKG_VERSION}_amd64.deb" \
        "$DEB_BUILD/out/peakd-archive-keyring_${PKG_VERSION}_all.deb" "$reg/main/"

    : >"$dlog"
    dpkg_in_root 'dpkg -i /tmp/peakd-register/first/*.deb' >>"$dlog" 2>&1 || {
        tail -30 "$dlog" >&2
        die "could not register the peakd dependencies"
    }
    dpkg_in_root 'dpkg -i /tmp/peakd-register/deps/*.deb' >>"$dlog" 2>&1 || {
        tail -30 "$dlog" >&2
        die "could not register the peakd dependencies"
    }
    dpkg_in_root 'dpkg -i /tmp/peakd-register/main/*.deb' >>"$dlog" 2>&1 || {
        tail -30 "$dlog" >&2
        die "could not register peakd"
    }
    rm -rf "$reg"

    local status
    status=$(unshare -Ur /usr/sbin/chroot "$ROOTFS" /usr/bin/env \
        PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
        dpkg-query -W -f='${Status}' peakd 2>/dev/null || true)
    [ "$status" = "install ok installed" ] \
        || die "peakd is not registered in the image (status: ${status:-missing})"
}

stage_qt() {
    log "staging Qt 6 WebEngine runtime"
    install -d "$ROOTFS/usr/lib/x86_64-linux-gnu" "$ROOTFS/usr/lib" "$ROOTFS/usr/share"
    cp -a /usr/lib/x86_64-linux-gnu/libQt6*.so* "$ROOTFS/usr/lib/x86_64-linux-gnu/"
    cp -a /usr/lib/x86_64-linux-gnu/qt6 "$ROOTFS/usr/lib/x86_64-linux-gnu/"
    cp -a /usr/lib/qt6 "$ROOTFS/usr/lib/"
    install -d "$ROOTFS/usr/share/qt6/resources" "$ROOTFS/usr/share/qt6/translations"
    find /usr/share/qt6/resources -maxdepth 1 -type f -exec cp -a {} "$ROOTFS/usr/share/qt6/resources/" \;
    cp -a /usr/share/qt6/translations/. "$ROOTFS/usr/share/qt6/translations/"
    local plugin
    for plugin in "$ROOTFS"/usr/lib/x86_64-linux-gnu/qt6/plugins/*/*.so; do
        copy_missing_libs "$plugin"
    done
    copy_missing_libs "$ROOTFS/usr/lib/qt6/libexec/QtWebEngineProcess"
}

stage_ollama() {
    log "staging Ollama runtime"
    if [ ! -x /usr/local/bin/ollama ]; then
        log "Ollama is not installed on the build host; skipping runtime"
        return
    fi
    install -Dm0755 /usr/local/bin/ollama "$ROOTFS/usr/local/bin/ollama"
    install -d "$ROOTFS/usr/local/lib/ollama" "$ROOTFS/var/lib/ollama/models"
    if [ -d /usr/local/lib/ollama ]; then
        local item name
        for item in /usr/local/lib/ollama/*; do
            name=$(basename "$item")
            case "$name" in
                cuda_v12|cuda_v13|rocm_v7_2) continue ;;
            esac
            cp -a "$item" "$ROOTFS/usr/local/lib/ollama/"
        done
    fi
    copy_missing_libs "$ROOTFS/usr/local/bin/ollama"
    local so
    for so in "$ROOTFS"/usr/local/lib/ollama/*.so* "$ROOTFS"/usr/local/lib/ollama/vulkan/*.so*; do
        [ -e "$so" ] && copy_missing_libs "$so"
    done
    log "Ollama runtime staged (CPU + Vulkan); models are downloaded on demand"
}

stage_audio_runtime() {
    log "staging PipeWire/WirePlumber runtime"
    local pkg deb
    for pkg in pipewire pipewire-bin pipewire-pulse wireplumber \
        libpipewire-0.3-0t64 libpipewire-0.3-modules libwireplumber-0.5-0 \
        libspa-0.2-modules libspa-0.2-bluetooth liblua5.4-0; do
        (cd "$DEBS" && apt-get download "$pkg" >/dev/null 2>&1) || true
        for deb in "$DEBS/${pkg}_"*.deb; do
            [ -f "$deb" ] || continue
            dpkg-deb -x "$deb" "$ROOTFS"
        done
    done
    local bin
    for bin in "$ROOTFS/usr/bin/pipewire" "$ROOTFS/usr/bin/wireplumber" \
        "$ROOTFS/usr/bin/wpctl" "$ROOTFS/usr/bin/pactl"; do
        [ -e "$bin" ] && copy_missing_libs "$bin"
    done
    local so
    for so in "$ROOTFS"/usr/lib/x86_64-linux-gnu/pipewire-0.3/*.so \
        "$ROOTFS"/usr/lib/x86_64-linux-gnu/spa-0.2/*/*.so \
        "$ROOTFS"/usr/lib/x86_64-linux-gnu/wireplumber-0.5/*.so; do
        [ -e "$so" ] && copy_missing_libs "$so"
    done
}

verify_audio_runtime() {
    log "verifying PipeWire/WirePlumber runtime dependencies"
    local missing
    missing=$(unshare -Ur /usr/sbin/chroot "$ROOTFS" /bin/sh -c '
        for f in /usr/bin/pipewire /usr/bin/pipewire-pulse /usr/bin/wireplumber \
            /usr/bin/wpctl /usr/bin/filter-chain \
            /usr/lib/x86_64-linux-gnu/wireplumber-0.5/*.so \
            /usr/lib/x86_64-linux-gnu/pipewire-0.3/*.so \
            /usr/lib/x86_64-linux-gnu/spa-0.2/*/*.so; do
            [ -e "$f" ] || continue
            /usr/bin/ldd "$f" 2>/dev/null
        done
    ' | grep 'not found' | sort -u || true)
    if [ -n "$missing" ]; then
        printf '%s\n' "$missing" >&2
        die "audio runtime has unresolved shared-library dependencies"
    fi
}

stage_extra_packages() {
    log "staging parted and filesystem tools"
    local pkg
    for pkg in parted libparted2t64 e2fsprogs dosfstools matchbox-window-manager; do
        (cd "$DEBS" && apt-get download "$pkg" >/dev/null 2>&1) || true
        local deb
        for deb in "$DEBS/${pkg}_"*.deb; do
            [ -f "$deb" ] || continue
            dpkg-deb -x "$deb" "$ROOTFS"
        done
    done
    local p
    for p in "$ROOTFS"/usr/sbin/parted "$ROOTFS"/usr/bin/parted \
        "$ROOTFS"/usr/sbin/mke2fs "$ROOTFS"/usr/sbin/mkfs.fat \
        "$ROOTFS"/usr/bin/matchbox-window-manager; do
        if [ -e "$p" ]; then
            copy_missing_libs "$p"
        fi
    done
    if [ -e "$ROOTFS/usr/sbin/mke2fs" ] && [ ! -e "$ROOTFS/usr/sbin/mkfs.ext4" ]; then
        ln -sf mke2fs "$ROOTFS/usr/sbin/mkfs.ext4"
    fi
    if [ -e "$ROOTFS/usr/sbin/mkfs.fat" ] && [ ! -e "$ROOTFS/usr/sbin/mkfs.vfat" ]; then
        ln -sf mkfs.fat "$ROOTFS/usr/sbin/mkfs.vfat"
    fi
    cp -a "$TOOLS"/usr/lib/x86_64-linux-gnu/libparted.so* "$ROOTFS/usr/lib/x86_64-linux-gnu/" 2>/dev/null || true

    # grub-mkstandalone is in the base image, but its platform modules are not.
    local grubdir=/usr/lib/grub/x86_64-efi
    [ -d "$grubdir" ] || grubdir="$TOOLS/usr/lib/grub/x86_64-efi"
    if [ -d "$grubdir" ]; then
        mkdir -p "$ROOTFS/usr/lib/grub"
        cp -a "$grubdir" "$ROOTFS/usr/lib/grub/"
    else
        die "GRUB x86_64-efi modules not found for the installer"
    fi
}

stage_t2() {
    log "staging T2 kernel, packages and userspace setup"
    [ -f "/boot/vmlinuz-$T2_KVER" ] || die "T2 kernel /boot/vmlinuz-$T2_KVER not found"
    [ -d "/lib/modules/$T2_KVER" ] || die "T2 modules /lib/modules/$T2_KVER not found"
    install -d "$ROOTFS/lib/modules"
    cp -a "/lib/modules/$T2_KVER" "$ROOTFS/lib/modules/"

    # T2 Macs use Apple-specific Broadcom firmware (BCM4364/4377/4387).
    # The Debian live firmware package does not carry all of these, so copy the
    # host's linux-firmware set; otherwise brcmfmac loads but never creates wlan0.
    if [ -d /lib/firmware/brcm ]; then
        install -d "$ROOTFS/lib/firmware/brcm"
        cp -a /lib/firmware/brcm/. "$ROOTFS/lib/firmware/brcm/"
    else
        log "warning: /lib/firmware/brcm not found; T2 Wi-Fi firmware missing"
    fi

    local pkg p
    for pkg in tiny-dfr t2fanrd t2-touchid t2-ncm apple-t2-audio-config; do
        dpkg -s "$pkg" >/dev/null 2>&1 || { log "T2 package not installed: $pkg (skipped)"; continue; }
        while IFS= read -r p; do
            [ -e "$p" ] || continue
            if [ -d "$p" ]; then
                mkdir -p "$ROOTFS$p"
            else
                mkdir -p "$ROOTFS$(dirname "$p")"
                cp -a "$p" "$ROOTFS$p"
            fi
        done < <(dpkg -L "$pkg")
        case "$pkg" in
            tiny-dfr|t2fanrd|t2-touchid|t2-ncm) ;;
        esac
    done

    install -d "$ROOTFS/usr/local/lib/shiny/t2/scripts"
    cp -a "$REPO"/scripts/install-t2-*.sh "$ROOTFS/usr/local/lib/shiny/t2/scripts/"
    cp -a "$REPO/scripts/install-touchpad-gestures.sh" "$ROOTFS/usr/local/lib/shiny/t2/scripts/"
    cp -a "$REPO/scripts/shiny-wait-drm.sh" "$ROOTFS/usr/local/lib/shiny/t2/scripts/"
    cp -a "$REPO/scripts/t2-audio" "$ROOTFS/usr/local/lib/shiny/t2/scripts/"
    cp -a "$REPO/scripts/touchbar" "$ROOTFS/usr/local/lib/shiny/t2/scripts/"
    install -m 0755 "$REPO/scripts/t2-audio/shiny-t2-audio-watchdog" "$ROOTFS/usr/local/bin/shiny-t2-audio-watchdog"
    install -d "$ROOTFS/etc/systemd/user"
    install -m 0644 "$REPO/scripts/live/overlay/etc/systemd/user/shiny-t2-audio-watchdog.service" \
        "$ROOTFS/etc/systemd/user/shiny-t2-audio-watchdog.service"
    chmod 0755 "$ROOTFS/usr/local/lib/shiny/t2/scripts/"*.sh
    chmod 0755 "$ROOTFS/usr/local/lib/shiny/t2/scripts/touchbar/"*.sh 2>/dev/null || true

    # Match the working T2 host initrd: only the BCE bridge/controller is
    # required before the rootfs is mounted. Audio, Touch Bar, SPI and the GPU
    # mux autoload from the rootfs once udev is up; forcing them into the
    # initrd made T2 boots depend on unrelated hardware probes.
    install -d "$ROOTFS/etc/initramfs-tools"
    cat >> "$ROOTFS/etc/initramfs-tools/modules" <<'EOF'
t2bce_dma
t2bce_core
t2bce_vhci
EOF
}

stage_overlay() {
    log "staging live session and installer helper"
    cp -a "$REPO/scripts/live/overlay/." "$ROOTFS/"
    chmod 0755 "$ROOTFS/usr/local/bin/peakd-live-session" \
        "$ROOTFS/usr/local/bin/peakd-collect-logs" \
        "$ROOTFS/usr/local/lib/shiny/t2-live-setup.sh" \
        "$ROOTFS/usr/local/lib/shiny/seed-live-account.sh"
    install -d "$ROOTFS/etc/systemd/system/multi-user.target.wants"
    ln -sf /etc/systemd/system/shiny-installer-helper.service \
        "$ROOTFS/etc/systemd/system/multi-user.target.wants/shiny-installer-helper.service"
    ln -sf /etc/systemd/system/shiny-t2-live-setup.service \
        "$ROOTFS/etc/systemd/system/multi-user.target.wants/shiny-t2-live-setup.service"
    ln -sf /etc/systemd/system/peakd-collect-logs.service \
        "$ROOTFS/etc/systemd/system/multi-user.target.wants/peakd-collect-logs.service"
    ln -sf /etc/systemd/system/shiny-live-seed.service \
        "$ROOTFS/etc/systemd/system/multi-user.target.wants/shiny-live-seed.service"
    # peakd-live-session starts Ollama as the live user with a valid HOME.
    # The system unit has no HOME and would restart forever in the live image.
    rm -f "$ROOTFS/etc/systemd/system/multi-user.target.wants/ollama.service" \
        "$ROOTFS/etc/systemd/system/graphical.target.wants/ollama.service"

    # The Debian live base enables PulseAudio, which is what the generic
    # profile used before. Keep it as the default; the T2 session switches to
    # PipeWire/WirePlumber itself, because that is the stack the T2 DSP needs.
    local user_units="$ROOTFS/etc/systemd/user"
    install -d "$user_units/default.target.wants" "$user_units/sockets.target.wants"
    ln -sf /usr/lib/systemd/user/pulseaudio.service "$user_units/default.target.wants/pulseaudio.service"
    ln -sf /usr/lib/systemd/user/pulseaudio.socket "$user_units/sockets.target.wants/pulseaudio.socket"

    # T2 DSP: the host setup uses WirePlumber's software DSP, the measured 16_1
    # FIR graphs and the LV2 plugins. The raw UCM card alone can still present
    # as Dummy Output in the live session, so transpose the working host files.
    local wp_dir="$ROOTFS/etc/xdg/wireplumber/wireplumber.conf.d"
    local t2_assets="$ROOTFS/usr/share/t2linux-audio/16_1"
    install -d "$wp_dir" "$t2_assets" "$ROOTFS/etc/udev/rules.d" "$ROOTFS/usr/lib/lv2"
    cp -a "$REPO/scripts/t2-audio/wireplumber.conf" "$wp_dir/50-t2-audio.conf"
    cp -a "$REPO/scripts/t2-audio/16_1/." "$t2_assets/"
    # The host's WirePlumber picks the T2 HiFi profile/route through this state;
    # without it a fresh live user can bind the card without a usable sink.
    install -d "$ROOTFS/usr/share/shiny/t2-wireplumber-state"
    for state in default-nodes default-routes default-profile; do
        [ -f "$HOME/.local/state/wireplumber/$state" ] && \
            cp -a "$HOME/.local/state/wireplumber/$state" \
                "$ROOTFS/usr/share/shiny/t2-wireplumber-state/"
    done
    sed 's/@MODEL_DIR@/16_1/g' "$REPO/scripts/t2-audio/99-t2-audio-rename.rules" \
        > "$ROOTFS/etc/udev/rules.d/99-t2-audio-rename.rules"
    chmod 0644 "$ROOTFS/etc/udev/rules.d/99-t2-audio-rename.rules"
    local lv2
    for lv2 in bankstown.lv2 lsp-plugins.lv2 triforce.lv2; do
        if [ -d "/usr/lib/lv2/$lv2" ]; then
            cp -a "/usr/lib/lv2/$lv2" "$ROOTFS/usr/lib/lv2/"
        fi
    done
    local so
    for so in "$ROOTFS"/usr/lib/lv2/*/*.so; do
        [ -e "$so" ] && copy_missing_libs "$so"
    done

    # The live session is the PEAK'D! kiosk, not an XFCE desktop session.
    rm -f "$ROOTFS/etc/xdg/autostart/peakd-live.desktop"
    find "$ROOTFS/usr/share/xsessions" -maxdepth 1 -type f ! -name peakd.desktop -delete 2>/dev/null || true

    # The live image must not expose the stock Calamares/d-i installer.
    find "$ROOTFS/usr" "$ROOTFS/etc" "$ROOTFS/var" -iname '*calamares*' \
        -exec rm -rf {} + 2>/dev/null || true
    rm -rf "$ROOTFS/home/eev" "$ROOTFS/tmp/livetest"
    rm -f "$ROOTFS/data/shiny.log"

    local live_user
    live_user=$(awk -F: '$3 == 1000 {print $1; exit}' "$ROOTFS/etc/passwd")
    [ -n "$live_user" ] || live_user=user
    if ! grep -q '^shiny:' "$ROOTFS/etc/group"; then
        printf 'shiny:x:990:\n' >> "$ROOTFS/etc/group"
    fi
    add_group_member shiny "$live_user"
    add_group_member video "$live_user"
    add_group_member audio "$live_user"
    add_group_member bluetooth "$live_user"
    log "live account: $live_user"
}

remove_xfce() {
    log "removing XFCE desktop payload"
    "$REPO/scripts/live/remove-xfce.sh" "$ROOTFS"
}

add_group_member() {
    local group=$1 user=$2 file="$ROOTFS/etc/group"
    awk -F: -v g="$group" -v u="$user" 'BEGIN { OFS=":" }
        $1 == g {
            found = 0
            n = split($4, a, ",")
            for (i = 1; i <= n; i++) if (a[i] == u) found = 1
            if (!found) {
                if ($4 == "") $4 = u
                else $4 = $4 "," u
            }
        }
        { print }
    ' "$file" > "$file.tmp"
    mv "$file.tmp" "$file"
}

build_t2_initrd() {
    log "building live-boot initrd for the T2 kernel"
    [ -x "$ROOTFS/usr/sbin/mkinitramfs" ] || die "mkinitramfs absent from the live rootfs"
    rm -f "$ROOTFS/tmp/initrd-t2.img"
    unshare -Ur chroot "$ROOTFS" /usr/sbin/mkinitramfs -o /tmp/initrd-t2.img "$T2_KVER" \
        >"$BUILD/mkinitramfs-t2.log" 2>&1 || {
            tail -40 "$BUILD/mkinitramfs-t2.log" >&2
            die "could not build the T2 live initrd"
        }
    [ -s "$ROOTFS/tmp/initrd-t2.img" ] || die "T2 initrd was not created"
    cp "$ROOTFS/tmp/initrd-t2.img" "$BUILD/initrd-t2.img"
    rm -f "$ROOTFS/tmp/initrd-t2.img"
}

assemble_boot() {
    log "assembling live boot tree"
    rm -rf "$ISO_STAGE"
    install -d "$ISO_STAGE/live" "$ISO_STAGE/boot/grub" "$ISO_STAGE/.disk"
    local base_kernel base_initrd
    base_kernel=$(find "$ISO_BASE/live" -maxdepth 1 -name 'vmlinuz*' -print -quit)
    base_initrd=$(find "$ISO_BASE/live" -maxdepth 1 -name 'initrd*' -print -quit)
    [ -n "$base_kernel" ] && [ -n "$base_initrd" ] || die "base live kernel/initrd not found"
    install -m 0644 "$base_kernel" "$ISO_STAGE/live/vmlinuz"
    install -m 0644 "$base_initrd" "$ISO_STAGE/live/initrd.img"
    [ -s "$BUILD/initrd-t2.img" ] || die "missing generated T2 initrd"
    install -m 0644 "$BUILD/initrd-t2.img" "$ISO_STAGE/live/initrd-t2.img"
    install -m 0644 "/boot/vmlinuz-$T2_KVER" "$ISO_STAGE/live/vmlinuz-t2"
    install -m 0644 "$REPO/scripts/live/grub.cfg" "$ISO_STAGE/boot/grub/grub.cfg"
    [ -f "$ISO_BASE/.disk/info" ] && install -m 0644 "$ISO_BASE/.disk/info" "$ISO_STAGE/.disk/info" || true

    log "building squashfs (root-owned entries)"
    mksquashfs "$ROOTFS" "$ISO_STAGE/live/filesystem.squashfs" \
        -comp zstd -noappend -all-root -no-progress >/dev/null
}

build_iso() {
    log "building UEFI ISO with GRUB"
    local grubdir=/usr/lib/grub/x86_64-efi
    [ -d "$grubdir" ] || grubdir="$TOOLS/usr/lib/grub/x86_64-efi"
    [ -f "$grubdir/modinfo.sh" ] || die "GRUB x86_64-efi modules not found"
    rm -f "$OUT"
    grub-mkrescue --directory="$grubdir" -o "$OUT" "$ISO_STAGE" \
        --compress=xz >"$BUILD/grub-mkrescue.log" 2>&1 || {
            tail -40 "$BUILD/grub-mkrescue.log" >&2
            die "grub-mkrescue failed"
        }
    sha256sum "$OUT" > "$OUT.sha256"
    cat > "$OUTDIR/$ISO_NAME.build-manifest.json" <<EOF
{
  "iso": "$ISO_NAME",
  "base_iso": "$BASE_ISO",
  "base_url": "$LIVE_URL",
  "t2_kernel": "$T2_KVER",
  "package": "peakd $PKG_VERSION amd64",
  "apt_repo": "$APT_URL",
  "built_at": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "note": "rootless UEFI remaster; package payload; real install engine enabled"
}
EOF
    log "done"
    ls -lh "$OUT" "$OUT.sha256"
    cat "$OUT.sha256"
}

install_tools
download_base
extract_base
stage_package
seed_live_account
stage_qt
stage_ollama
stage_audio_runtime
stage_extra_packages
stage_t2
stage_overlay
verify_audio_runtime
remove_xfce
register_payload
build_t2_initrd
assemble_boot
build_iso
