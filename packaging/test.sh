#!/usr/bin/env bash
# Installs each package from dist/ in a fresh container of the distributions
# it is for, then checks that `ferricone --version` runs and that no library
# of the binary, libVLC or any plugin is missing:
#
#   packaging/test.sh            # every package found in dist/
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
engine=${CONTAINER_ENGINE:-$(command -v podman || command -v docker)}
version=$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/Cargo.toml" | head -1)

pass=() fail=()
log=$(mktemp)
try() { # try <label> <image> <setup and install commands> [strict]
  echo "== $1 ($2)"
  if "$engine" run --rm -v "$root/dist:/dist:ro,z" -v "$root/packaging:/pkg:ro,z" -e STRICT="${4:-}" "$2" \
      sh -c "$3 && sh /pkg/check-install.sh" > "$log" 2>&1; then
    pass+=("$1")
  else
    fail+=("$1")
  fi
  grep -E "^Ferricone Player|^checked|missing|break|not found|undefined|^ok$" "$log" || tail -5 "$log"
}

deb=/dist/ferricone_${version}_amd64.deb
rpm=/dist/ferricone-$version-1.x86_64.rpm
pkg=/dist/ferricone-$version-1-x86_64.pkg.tar.zst
img=/dist/Ferricone_Player-$version-x86_64.AppImage
tgz=/dist/ferricone-$version-x86_64.tar.gz

# Container images skip recommended packages; the strict tests ask for them,
# as a desktop installs them.
if [ -f "$root$deb" ]; then
  for image in docker.io/library/debian:12 docker.io/library/ubuntu:22.04 docker.io/library/ubuntu:24.04; do
    try "deb on ${image##*/}" "$image" "apt-get update -qq >/dev/null && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq file $deb >/dev/null" 1
  done
fi
if [ -f "$root$rpm" ]; then
  try "rpm on fedora" docker.io/library/fedora:latest "dnf install -y -q --setopt=install_weak_deps=True file $rpm >/dev/null" 1
  try "rpm on almalinux 10" docker.io/library/almalinux:10 "dnf install -y -q --setopt=install_weak_deps=True file $rpm >/dev/null" 1
  try "rpm on opensuse" registry.opensuse.org/opensuse/tumbleweed:latest "zypper -q --non-interactive install --recommends --allow-unsigned-rpm file gawk $rpm >/dev/null" 1
fi
if [ -f "$root$pkg" ]; then
  try "pacman on arch" docker.io/library/archlinux:latest "pacman -Sy --noconfirm --needed file >/dev/null && pacman -U --noconfirm $pkg >/dev/null"
fi
if [ -f "$root$img" ]; then
  # A desktop's libraries: the .deb's dependencies. No FUSE in a container,
  # so it is unpacked, as --appimage-extract does.
  try "AppImage on debian" docker.io/library/debian:12 "apt-get update -qq >/dev/null && apt-get install -y -qq file \$(dpkg-deb -f $deb Depends | sed 's/([^)]*)//g; s/|[^,]*//g; s/,/ /g') >/dev/null && cd /tmp && $img --appimage-extract >/dev/null && ln -s /tmp/squashfs-root/usr/bin/ferricone /usr/local/bin/ferricone && ln -s /tmp/squashfs-root/usr /opt/ferricone"
fi
if [ -f "$root$tgz" ]; then
  # A desktop's libraries: the .rpm's dependencies.
  try "tar.gz on fedora" docker.io/library/fedora:latest "dnf install -y -q file util-linux \$(rpm -qpR $rpm | grep -v rpmlib | sed 's/ //g') >/dev/null && useradd -m u && su u -c 'cd && tar xzf $tgz && ferricone-$version-x86_64/install.sh' && ln -s /home/u/.local/bin/ferricone /usr/local/bin/ferricone && ln -s /home/u/.local/opt/ferricone /opt/ferricone"
fi

echo
echo "passed: ${#pass[@]}  ${pass[*]}"
echo "failed: ${#fail[@]}  ${fail[*]}"
[ ${#fail[@]} = 0 ]
