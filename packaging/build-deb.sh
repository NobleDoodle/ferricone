#!/usr/bin/env bash
# Builds dist/'s Debian package (.deb) from a clean tree; see packaging/build.sh.
exec "$(dirname "$0")/build.sh" "$@" deb
