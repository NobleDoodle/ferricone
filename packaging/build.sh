#!/usr/bin/env bash
# Builds Ferricone's Linux packages in containers, so the host needs only
# Docker (or Podman):
#
#   packaging/build.sh deb rpm pacman appimage tarball   # or: all
#   packaging/build.sh --arch aarch64 all                # arm64 (emulated)
#   packaging/build.sh --arch all all                    # both
#
# --arch defaults to this machine's. Another architecture's containers run
# under QEMU (qemu-user-static with binfmt), so the same scripts build it
# natively, only slower.
#
# The first run builds the image (packaging/Dockerfile) and the bundle every
# package wraps (packaging/bundle.sh: libVLC 4 with patches/, Ferricone, and
# their libraries); later runs reuse both unless --rebuild is given. Packages
# land in dist/.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
engine=${CONTAINER_ENGINE:-$(command -v podman || command -v docker)}
version=$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/Cargo.toml" | head -1)

rebuild=false
formats=()
arches=()
want_arch=false
for arg in "$@"; do
  if $want_arch; then
    case $arg in
      all) arches+=(x86_64 aarch64) ;;
      x86_64|aarch64) arches+=("$arg") ;;
      *) echo "unknown architecture: $arg" >&2; exit 2 ;;
    esac
    want_arch=false
    continue
  fi
  case $arg in
    --rebuild) rebuild=true ;;
    --arch) want_arch=true ;;
    all) formats+=(deb rpm pacman appimage tarball) ;;
    deb|rpm|pacman|appimage|tarball) formats+=("$arg") ;;
    *) echo "unknown format: $arg" >&2; exit 2 ;;
  esac
done
[ ${#formats[@]} -gt 0 ] || { echo "usage: $0 [--rebuild] [--arch x86_64|aarch64|all] deb|rpm|pacman|appimage|tarball|all..." >&2; exit 2; }
[ ${#arches[@]} -gt 0 ] || arches=("$(uname -m)")

# Docker's root is the host's root: hand the files back. Rootless Podman's
# root already is this user.
owner=()
case $(basename "$engine") in docker) owner=(-e HOST_UID="$(id -u)" -e HOST_GID="$(id -g)") ;; esac

run() { # run <image> <script> [args]: a script from packaging/, in a container
  local img=$1; shift
  # No core files: QEMU writes them into the working directory, /src.
  "$engine" run --rm --platform "$platform" --ulimit core=0 -v "$root:/src:z" -w /src "${owner[@]}" "$img" bash "/src/packaging/$@"
}

for arch in "${arches[@]}"; do
  case $arch in
    x86_64) platform=linux/amd64 pacman_image=docker.io/library/archlinux:latest ;;
    # Arch Linux ARM (the base of Omarchy's and Asahi's aarch64 systems):
    # no official image, so menci/archlinuxarm, built daily from its rootfs.
    aarch64) platform=linux/arm64 pacman_image=docker.io/menci/archlinuxarm:base ;;
  esac
  image=ferricone-build:$arch
  bundle=dist/ferricone-$version-$arch
  echo "== $arch"

  if $rebuild || ! "$engine" image inspect "$image" >/dev/null 2>&1; then
    "$engine" build --platform "$platform" -t "$image" "$root/packaging"
  fi
  if $rebuild || [ ! -x "$root/$bundle/bin/ferricone" ]; then
    run "$image" bundle.sh
  fi

  for format in "${formats[@]}"; do
    case $format in
      # makepkg is Arch's own tool: run it on Arch (Linux ARM).
      pacman) run "$pacman_image" package-pacman.sh "$bundle" ;;
      *) run "$image" "package-$format.sh" "$bundle" ;;
    esac
  done
done
ls -lh "$root"/dist/*.{deb,rpm,zst,AppImage,tar.gz} 2>/dev/null || true
