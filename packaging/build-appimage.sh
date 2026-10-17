#!/usr/bin/env bash
# Builds dist/'s AppImage from a clean tree; see packaging/build.sh.
exec "$(dirname "$0")/build.sh" "$@" appimage
