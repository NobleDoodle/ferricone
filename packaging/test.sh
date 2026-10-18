#!/usr/bin/env bash
# Installs each package from dist/ in a fresh container of the distributions
# it is for, then checks that `ferricone --version` runs and that no library
# of the binary, libVLC or any plugin is missing:
#
#   packaging/test.sh                    # this machine's architecture
#   packaging/test.sh --arch aarch64     # arm64, in emulated containers
#   packaging/test.sh --arch all
#   packaging/test.sh --format pacman    # only some formats (repeatable)
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
engine=${CONTAINER_ENGINE:-$(command -v podman || command -v docker)}
version=$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/Cargo.toml" | head -1)

arches=()
only=" "
while [ $# -gt 0 ]; do
  case $1 in
    --format) case ${2:-} in deb|rpm|pacman|appimage|tarball) only="$only$2 " ;; *) echo "unknown format: ${2:-}" >&2; exit 2 ;; esac; shift 2 ;;
    --arch) case ${2:-} in all) arches+=(x86_64 aarch64) ;; x86_64|aarch64) arches+=("$2") ;; *) echo "unknown architecture: ${2:-}" >&2; exit 2 ;; esac; shift 2 ;;
    *) echo "usage: $0 [--arch x86_64|aarch64|all] [--format deb|rpm|pacman|appimage|tarball]..." >&2; exit 2 ;;
  esac
done
[ ${#arches[@]} -gt 0 ] || arches=("$(uname -m)")
[ "$only" != " " ] || only=" deb rpm pacman appimage tarball "
wanted() { case $only in *" $1 "*) return 0 ;; esac; return 1; }

pass=() fail=()
log=$(mktemp)
try() { # try <label> <image> <setup and install commands> [strict]
  echo "== $1 ($2, $arch)"
  if "$engine" run --rm --platform "$platform" --ulimit core=0 -v "$root/dist:/dist:ro,z" -v "$root/packaging:/pkg:ro,z" -e STRICT="${4:-}" "$2" \
      sh -c "$3 && sh /pkg/check-install.sh" > "$log" 2>&1; then
    pass+=("$1/$arch")
  else
    fail+=("$1/$arch")
  fi
  grep -E "^Ferricone Player|^checked|missing|break|not found|undefined|^ok$" "$log" || tail -5 "$log"
}

for arch in "${arches[@]}"; do
  case $arch in
    x86_64) platform=linux/amd64 debarch=amd64
            pacman_images=(arch=docker.io/library/archlinux:latest) ;;
    # Arch Linux ARM (Omarchy's and Asahi's aarch64 base; no official image,
    # so menci/archlinuxarm, built daily from its rootfs) and Manjaro ARM.
    aarch64) platform=linux/arm64 debarch=arm64
             pacman_images=(archlinuxarm=docker.io/menci/archlinuxarm:base manjaro-arm=docker.io/manjarolinux/base:latest) ;;
  esac
  deb=/dist/ferricone_${version}_$debarch.deb
  rpm=/dist/ferricone-$version-1.$arch.rpm
  pkg=/dist/ferricone-$version-1-$arch.pkg.tar.zst
  img=/dist/Ferricone_Player-$version-$arch.AppImage
  tgz=/dist/ferricone-$version-$arch.tar.gz

  # Container images skip recommended packages; the strict tests ask for them,
  # as a desktop installs them.
  if wanted deb && [ -f "$root$deb" ]; then
    for image in docker.io/library/debian:12 docker.io/library/ubuntu:22.04 docker.io/library/ubuntu:24.04; do
      try "deb on ${image##*/}" "$image" "apt-get update -qq >/dev/null && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq file $deb >/dev/null" 1
    done
  fi
  if wanted rpm && [ -f "$root$rpm" ]; then
    try "rpm on fedora" docker.io/library/fedora:latest "dnf install -y -q --setopt=install_weak_deps=True file $rpm >/dev/null" 1
    try "rpm on almalinux 10" docker.io/library/almalinux:10 "dnf install -y -q --setopt=install_weak_deps=True file $rpm >/dev/null" 1
    try "rpm on opensuse" registry.opensuse.org/opensuse/tumbleweed:latest "zypper -q --non-interactive install --recommends --allow-unsigned-rpm file gawk $rpm >/dev/null" 1
  fi
  if wanted pacman && [ -f "$root$pkg" ]; then
    for entry in "${pacman_images[@]}"; do
      try "pacman on ${entry%%=*}" "${entry#*=}" "pacman-key --init >/dev/null 2>&1; pacman-key --populate >/dev/null 2>&1; sb=; pacman --disable-sandbox -V >/dev/null 2>&1 && sb=--disable-sandbox; pacman \$sb -Sy --noconfirm --needed file >/dev/null && pacman \$sb -U --noconfirm $pkg >/dev/null"
    done
  fi
  if wanted appimage && [ -f "$root$img" ]; then
    # A desktop's libraries: the .deb's dependencies. No FUSE in a container,
    # so it is unpacked, as --appimage-extract does, from a copy without the
    # AppImage magic bytes, which keep binfmt_misc from handing an arm64
    # AppImage to QEMU.
    try "AppImage on debian" docker.io/library/debian:12 "apt-get update -qq >/dev/null && apt-get install -y -qq file \$(dpkg-deb -f $deb Depends | sed 's/([^)]*)//g; s/|[^,]*//g; s/,/ /g') >/dev/null && cd /tmp && cp $img /tmp/f.AppImage && printf '\\0\\0\\0' | dd of=/tmp/f.AppImage bs=1 seek=8 conv=notrunc status=none && /tmp/f.AppImage --appimage-extract >/dev/null && ln -s /tmp/squashfs-root/usr/bin/ferricone /usr/local/bin/ferricone && ln -s /tmp/squashfs-root/usr /opt/ferricone"
  fi
  if wanted tarball && [ -f "$root$tgz" ]; then
    # A desktop's libraries: the .rpm's dependencies.
    try "tar.gz on fedora" docker.io/library/fedora:latest "dnf install -y -q file util-linux \$(rpm -qpR $rpm | grep -v rpmlib | sed 's/ //g') >/dev/null && useradd -m u && su u -c 'cd && tar xzf $tgz && ferricone-$version-$arch/install.sh' && ln -s /home/u/.local/bin/ferricone /usr/local/bin/ferricone && ln -s /home/u/.local/opt/ferricone /opt/ferricone"
  fi
done

echo
echo "passed: ${#pass[@]}  ${pass[*]}"
echo "failed: ${#fail[@]}  ${fail[*]}"
[ ${#fail[@]} = 0 ]
