#!/usr/bin/env bash
# Builds Ferricone's Linux packages in containers, so the host needs only
# Docker (or Podman):
#
#   packaging/build.sh deb rpm pacman appimage tarball   # or: all
#
# The first run builds the image (packaging/Dockerfile) and the bundle every
# package wraps (packaging/bundle.sh: libVLC 4 with patches/, Ferricone, and
# their libraries); later runs reuse both unless --rebuild is given. Packages
# land in dist/.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
engine=${CONTAINER_ENGINE:-$(command -v podman || command -v docker)}
image=ferricone-build
version=$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/Cargo.toml" | head -1)
bundle=dist/ferricone-$version-x86_64

rebuild=false
formats=()
for arg in "$@"; do
  case $arg in
    --rebuild) rebuild=true ;;
    all) formats+=(deb rpm pacman appimage tarball) ;;
    deb|rpm|pacman|appimage|tarball) formats+=("$arg") ;;
    *) echo "unknown format: $arg" >&2; exit 2 ;;
  esac
done
[ ${#formats[@]} -gt 0 ] || { echo "usage: $0 [--rebuild] deb|rpm|pacman|appimage|tarball|all..." >&2; exit 2; }

# Docker's root is the host's root: hand the files back. Rootless Podman's
# root already is this user.
owner=()
case $(basename "$engine") in docker) owner=(-e HOST_UID="$(id -u)" -e HOST_GID="$(id -g)") ;; esac

run() { # run <image> <script> [args]: a script from packaging/, in a container
  local img=$1; shift
  "$engine" run --rm -v "$root:/src:z" -w /src "${owner[@]}" "$img" bash "/src/packaging/$@"
}

if $rebuild || ! "$engine" image inspect "$image" >/dev/null 2>&1; then
  "$engine" build -t "$image" "$root/packaging"
fi
if $rebuild || [ ! -x "$root/$bundle/bin/ferricone" ]; then
  run "$image" bundle.sh
fi

for format in "${formats[@]}"; do
  case $format in
    # makepkg is Arch's own tool: run it on Arch.
    pacman) run docker.io/library/archlinux:latest package-pacman.sh "$bundle" ;;
    *) run "$image" "package-$format.sh" "$bundle" ;;
  esac
done
ls -lh "$root"/dist/*.{deb,rpm,zst,AppImage,tar.gz} 2>/dev/null || true
