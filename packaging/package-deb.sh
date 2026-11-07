#!/usr/bin/env bash
# package-deb.sh <bundle>: dist/ferricone_<version>_<amd64|arm64>.deb, for Debian,
# Ubuntu, Mint and their kin. Built with dpkg-deb in the packaging image.
source /src/packaging/common.sh "$@"
case $arch in x86_64) debarch=amd64 ;; aarch64) debarch=arm64 ;; esac

# Debian package names for sonames, from the build image's own packages
# (22.04's names; later releases' t64 packages provide them). dpkg is asked
# directly: ldconfig -p crashes under QEMU's arm64.
debpkgs() {
  local soname owner
  for soname in "$@"; do
    owner=$(dpkg -S "*/$soname" 2>/dev/null \
      | awk -F': ' -v s="/$soname" '$2 ~ /^\/(usr\/)?lib\// && substr($2, length($2) - length(s) + 1) == s { print $1; exit }')
    [ -n "$owner" ] || { echo "no package owns $soname" >&2; exit 1; }
    owner=${owner%%,*}
    echo "${owner%%:*}"
  done | sort -u | grep -vx libc6 | paste -sd, | sed 's/,/, /g'
}
depends="libc6 (>= 2.35), $(debpkgs $(syslibs required))"
recommends=$(debpkgs $(syslibs optional))

root=$(mktemp -d)
stage "$root"
mkdir -p "$root/DEBIAN"
cat > "$root/DEBIAN/control" <<CONTROL
Package: ferricone
Version: $tilde_version
Architecture: $debarch
Maintainer: $maintainer
Installed-Size: $(du -sk "$root" | cut -f1)
Section: video
Priority: optional
Homepage: $homepage
Depends: $depends
Recommends: $recommends, xdg-desktop-portal
Description: $summary
 Ferricone plays videos, music and network streams on VLC's own engine,
 libVLC 4 (bundled), with a new interface after Juxtopposed's "VLC Redesign":
 a library beside the player, per-folder presets, a subtitle timeline,
 OpenSubtitles, casting, and light, dark or system themes.
CONTROL
cat > "$root/usr/share/doc/ferricone/copyright" <<COPYRIGHT
Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/
Upstream-Name: Ferricone Player
Source: https://github.com/NobleDoodle/ferricone

Files: *
Copyright: 2026 NobleDoodle
License: GPL-3.0-or-later
 On Debian systems, the full text is in /usr/share/common-licenses/GPL-3.
 It is also in /usr/share/doc/ferricone/LICENSE.
COPYRIGHT

deb=$out/ferricone_${version}_$debarch.deb
dpkg-deb --build --root-owner-group -Zxz "$root" "$deb"
finish "$deb"
