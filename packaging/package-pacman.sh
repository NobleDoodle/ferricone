#!/usr/bin/env bash
# package-pacman.sh <bundle>: dist/ferricone-<version>-1-<arch>.pkg.tar.zst,
# for Arch and the systems built on it: Omarchy, EndeavourOS, Manjaro, and on
# aarch64 Arch Linux ARM and those built on it (Omarchy's ARM build, Asahi,
# Manjaro ARM). Built with makepkg in an Arch (Linux ARM) container
# (packaging/build.sh), as an unprivileged user, as makepkg requires.
source /src/packaging/common.sh "$@"

# Arch Linux ARM's image ships without an initialized keyring.
pacman-key --init >/dev/null 2>&1
pacman-key --populate >/dev/null 2>&1
# pacman 7 downloads in a Landlock sandbox, which QEMU's user-mode emulation
# (arm64 builds on x86-64) lacks; this container is throwaway anyway.
sandbox=()
pacman --disable-sandbox -V >/dev/null 2>&1 && sandbox=(--disable-sandbox)
pacman "${sandbox[@]}" -Sy --noconfirm --needed fakeroot binutils >/dev/null
pacman "${sandbox[@]}" -Fy >/dev/null
# Arch package names for sonames, from pacman's file lists: one query for
# all of them (each query reads the whole list, slow under emulation).
archpkgs() {
  [ $# -gt 0 ] || return 0
  local soname paths=()
  for soname in "$@"; do paths+=("usr/lib/$soname"); done
  # repo, package, version and path, NUL-separated; the first owner wins.
  pacman -F --machinereadable "${paths[@]}" | tr '\0' '\t' \
    | awk -F'\t' '!seen[$4]++ { print $4 "\t" $2 }' > /tmp/owners || true
  for soname in "$@"; do
    awk -F'\t' -v p="usr/lib/$soname" '$1 == p { print $2; found = 1 } END { exit !found }' /tmp/owners \
      || { echo "no package has $soname" >&2; exit 1; }
    # Arch split gcc-libs into libgcc, libstdc++ and others in 2025 and kept
    # gcc-libs as the package depending on them; systems on older snapshots
    # (Manjaro ARM) only have gcc-libs.
  done | sed -E 's/^(libgcc|libstdc\+\+)$/gcc-libs/' | sort -u
}
depends=$(archpkgs $(syslibs required) | paste -sd' ')
optdepends=$(archpkgs $(syslibs optional) | sed "s/.*/'&: some libVLC plugins'/" | paste -sd' ')
useradd -m builder 2>/dev/null || true
work=$(mktemp -d) && chown builder "$work"
stage "$work/stage"
cat > "$work/PKGBUILD" <<PKGBUILD
pkgname=ferricone
pkgver=$pacman_version
pkgrel=1
pkgdesc="$summary"
arch=($arch)
url="$homepage"
license=(GPL-3.0-or-later)
depends=($depends)
optdepends=($optdepends 'xdg-desktop-portal: file dialogs')
options=(!strip !debug)

package() {
  cp -a "$work/stage/." "\$pkgdir/"
  install -Dm644 "\$pkgdir/usr/share/doc/ferricone/LICENSE" "\$pkgdir/usr/share/licenses/ferricone/LICENSE"
}
PKGBUILD
chown -R builder "$work"
# Arch Linux ARM's makepkg.conf defaults to .pkg.tar.xz.
su builder -c "cd '$work' && PKGDEST='$work' PKGEXT=.pkg.tar.zst makepkg -d -f --noconfirm >/dev/null"
pkg=$out/ferricone-$version-1-$arch.pkg.tar.zst
cp "$work/ferricone-$pacman_version-1-$arch.pkg.tar.zst" "$pkg"
finish "$pkg"
