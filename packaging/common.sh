# Sourced by the package-*.sh scripts: the version, the architecture, the
# bundle and the app id.
set -euo pipefail
bundle=/src/${1:?usage: package-<format>.sh <bundle>}
version=$(sed -n 's/^version = "\(.*\)"/\1/p' /src/Cargo.toml | head -1)
# x86_64 or aarch64, from the bundle's name (dist/ferricone-<version>-<arch>).
arch=${bundle##*-}
# A prerelease ("1.1.0-dev.1") must sort before its release in every package
# manager: dpkg and rpm read "~" so ("1.1.0~dev.1"), pacman a letter right
# after the number ("1.1.0dev1"). File names keep the plain version.
case $version in
  *-*) tilde_version=${version/-/\~}
       pacman_version=${version%%-*}$(echo "${version#*-}" | tr -d .) ;;
  *) tilde_version=$version pacman_version=$version ;;
esac
app=io.github.nobledoodle.ferricone
summary="Media player on libVLC 4 with a redesigned interface and a library"
maintainer="NobleDoodle <nobledoodle@users.noreply.github.com>"
homepage=https://nobledoodle.github.io/ferricone/
out=/src/dist
# System libraries the bundle needs (collect-deps.py): `required` and
# `optional` (only plugins need them) sonames.
syslibs() { awk -v k="$1" '$1 == k { print $2 }' "$bundle.system-libs.txt"; }
finish() { # finish <file>: hands the package to the host user
  [ -n "${HOST_UID:-}" ] && chown "$HOST_UID:${HOST_GID:-$HOST_UID}" "$1"
  echo "built $1 ($(du -h "$1" | cut -f1))"
}

# stage <dir>: the system layout every package installs: the bundle under
# /opt/ferricone, a command in /usr/bin, and the desktop files in /usr/share.
stage() {
  local root=$1
  mkdir -p "$root/opt/ferricone" "$root/usr/bin" "$root/usr/share"
  cp -a "$bundle"/{bin,lib,libexec} "$root/opt/ferricone/"
  mkdir -p "$root/opt/ferricone/share" && cp -a "$bundle/share/vlc" "$root/opt/ferricone/share/"
  cp -a "$bundle"/share/{applications,icons,metainfo,doc} "$root/usr/share/"
  ln -s /opt/ferricone/bin/ferricone "$root/usr/bin/ferricone"
}
