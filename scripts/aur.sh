#!/usr/bin/env bash
# Builds and checks the AUR package in packaging/aur: makepkg (with the
# protocol tests), namcap, and that .SRCINFO is current. It runs inside Arch
# Linux as root, as in CI's archlinux container; locally, use
# `mise run packaging:aur`, which runs it in Docker.
set -euo pipefail

src=${1:-packaging/aur}
[[ -f /etc/arch-release ]] || { echo "aur.sh runs inside Arch Linux; use mise run packaging:aur" >&2; exit 1; }

# Containers (Docker here, CI's job container) can't always apply pacman's
# seccomp filter for its download sandbox, notably under emulation.
if [[ -f /.dockerenv || -n "${container:-}" ]]; then
  sed -i 's/^#DisableSandboxSyscalls/DisableSandboxSyscalls/' /etc/pacman.conf
fi
pacman -Syu --noconfirm --needed base-devel rust namcap >/dev/null
id builder >/dev/null 2>&1 || useradd -m builder
work=$(mktemp -d)
cp "$src/PKGBUILD" "$src/.SRCINFO" "$work/"
chown -R builder "$work"
cd "$work"

echo "==> .SRCINFO is current"
su builder -c 'makepkg --printsrcinfo' | diff -u .SRCINFO - || {
  echo "packaging/aur/.SRCINFO is stale: run makepkg --printsrcinfo > .SRCINFO" >&2
  exit 1
}

echo "==> namcap PKGBUILD"
namcap PKGBUILD | tee namcap.txt
if grep -q ' E: ' namcap.txt; then
  echo "namcap found errors in the PKGBUILD" >&2
  exit 1
fi

echo "==> makepkg"
su builder -c 'makepkg --noconfirm --cleanbuild'

# Only the package itself: namcap always flags the -debug package's links
# into it. namcap exits 0 even on errors, so look for them.
pkg=$(find . -maxdepth 1 -name "irsty-[0-9]*.pkg.tar.zst" ! -name "*-debug-*" | head -1)
echo "==> namcap $pkg"
namcap "$pkg" | tee namcap.txt
if grep -q ' E: ' namcap.txt; then
  echo "namcap found errors" >&2
  exit 1
fi
pacman -Qlp "$pkg" | sed 's/^/  /'
