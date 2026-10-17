#!/usr/bin/env bash
# package-deb.sh <bundle>: dist/ferricone_<version>_amd64.deb, for Debian,
# Ubuntu, Mint and their kin. Built with dpkg-deb in the packaging image.
source /src/packaging/common.sh "$@"

# Debian package names for sonames, from the build image's own packages
# (22.04's names; later releases' t64 packages provide them).
debpkgs() {
  local soname path owner
  for soname in "$@"; do
    path=$(ldconfig -p | awk -v s="$soname" '$1 == s && /x86-64/ { print $NF; exit }')
    owner=$(dpkg -S "$path" 2>/dev/null || dpkg -S "/usr$path" 2>/dev/null || dpkg -S "${path#/usr}" 2>/dev/null) \
      || { echo "no package owns $soname" >&2; exit 1; }
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
Version: $version
Architecture: amd64
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

deb=$out/ferricone_${version}_amd64.deb
dpkg-deb --build --root-owner-group -Zxz "$root" "$deb"
finish "$deb"
