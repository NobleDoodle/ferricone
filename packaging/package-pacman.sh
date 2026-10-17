#!/usr/bin/env bash
# package-pacman.sh <bundle>: dist/ferricone-<version>-1-x86_64.pkg.tar.zst,
# for Arch, Manjaro and EndeavourOS. Built with makepkg in an Arch container
# (packaging/build.sh), as an unprivileged user, as makepkg requires.
source /src/packaging/common.sh "$@"

pacman -Sy --noconfirm --needed fakeroot binutils >/dev/null
pacman -Fy >/dev/null
# Arch package names for sonames, from pacman's file lists.
archpkgs() {
  local soname
  for soname in "$@"; do
    pacman -Fq "usr/lib/$soname" | head -1 | sed 's|.*/||' | grep . \
      || { echo "no package has $soname" >&2; exit 1; }
  done | sort -u
}
depends=$(archpkgs $(syslibs required) | paste -sd' ')
optdepends=$(archpkgs $(syslibs optional) | sed "s/.*/'&: some libVLC plugins'/" | paste -sd' ')
useradd -m builder 2>/dev/null || true
work=$(mktemp -d) && chown builder "$work"
stage "$work/stage"
cat > "$work/PKGBUILD" <<PKGBUILD
pkgname=ferricone
pkgver=$version
pkgrel=1
pkgdesc="$summary"
arch=(x86_64)
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
su builder -c "cd '$work' && PKGDEST='$work' makepkg -d -f --noconfirm >/dev/null"
pkg=$out/ferricone-$version-1-x86_64.pkg.tar.zst
cp "$work/$(basename "$pkg")" "$pkg"
finish "$pkg"
