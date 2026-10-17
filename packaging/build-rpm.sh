#!/usr/bin/env bash
# Builds dist/'s RPM package (.rpm) from a clean tree; see packaging/build.sh.
exec "$(dirname "$0")/build.sh" "$@" rpm
