#!/usr/bin/env bash
# Builds dist/'s Arch package (.pkg.tar.zst) from a clean tree; see packaging/build.sh.
exec "$(dirname "$0")/build.sh" "$@" pacman
