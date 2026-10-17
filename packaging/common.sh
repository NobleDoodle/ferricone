# Sourced by the package-*.sh scripts: the version, the bundle and the app id.
set -euo pipefail
bundle=/src/${1:?usage: package-<format>.sh <bundle>}
version=$(sed -n 's/^version = "\(.*\)"/\1/p' /src/Cargo.toml | head -1)
app=io.github.nobledoodle.ferricone
summary="Media player on libVLC 4 with a redesigned interface and a library"
maintainer="NobleDoodle <nobledoodle@users.noreply.github.com>"
homepage=https://nobledoodle.github.io/ferricone/
out=/src/dist
# System libraries the bundle needs (collect-deps.py): `required` and
# `optional` (only plugins need them) sonames.
syslibs() { awk -v k="$1" '$1 == k { print $2 }' "$out/system-libs.txt"; }
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
