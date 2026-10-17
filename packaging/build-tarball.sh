#!/usr/bin/env bash
# Builds dist/'s archive (.tar.gz) from a clean tree; see packaging/build.sh.
exec "$(dirname "$0")/build.sh" "$@" tarball
