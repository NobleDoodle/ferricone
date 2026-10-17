#!/bin/sh
# check-install.sh [dir]: run inside a test container (packaging/test.sh)
# after installing a package: `ferricone --version` runs, and no program,
# libVLC or bundled library under dir (/opt/ferricone) is missing a library.
# Plugins' missing libraries fail the check only with STRICT=1 (where the
# package manager installs recommended packages); otherwise they're listed.
set -e
dir=${1:-/opt/ferricone}
ferricone --version
# Programs must find their libraries through their own RPATH. Libraries and
# plugins are loaded by a program whose DT_RPATH covers lib, lib/vlc and
# lib/deps, so they are checked with those paths.
libs="$dir/lib:$dir/lib/vlc:$dir/lib/deps"
find -L "$dir" -type f | while read -r f; do
  case $(file -b "$f") in
    *ELF*executable*) ldd "$f" 2>/dev/null ;;
    *ELF*) LD_LIBRARY_PATH=$libs ldd "$f" 2>/dev/null ;;
  esac | grep "not found" | sed "s|^|$f: |"
done | sort -u > /tmp/missing
grep -v /plugins/ /tmp/missing > /tmp/core || true
grep /plugins/ /tmp/missing > /tmp/plugins || true
echo "checked $(find -L "$dir" -name '*.so*' | wc -l) libraries"

# Shadowing: a bundled library must not break a system library loaded into
# the same process (the system's GnuTLS needs a newer Nettle than 22.04's).
# Each system library the bundle loads is resolved with and without the
# bundle's paths; errors that only the bundle's paths cause are failures.
find -L "$dir" -type f -name '*.so*' | while read -r f; do
  LD_LIBRARY_PATH=$libs ldd "$f" 2>/dev/null
done | awk '$2 == "=>" && $3 ~ /^\// && $3 !~ /^\/opt\/ferricone|squashfs-root|\.local\/opt/ { print $3 }' | sort -u > /tmp/system
: > /tmp/shadow
while read -r lib; do
  ldd -r "$lib" 2>&1 | grep -E "undefined symbol|not found" | sort > /tmp/plain || true
  LD_LIBRARY_PATH=$libs ldd -r "$lib" 2>&1 | grep -E "undefined symbol|not found" | sort > /tmp/with || true
  comm -13 /tmp/plain /tmp/with | sed "s|^|$lib: |" >> /tmp/shadow
done < /tmp/system
echo "checked $(wc -l < /tmp/system) system libraries for shadowing"
if [ -s /tmp/shadow ]; then echo "bundled libraries break system ones:"; sort -u /tmp/shadow | head -20; exit 1; fi
if [ -s /tmp/plugins ]; then
  echo "plugins missing system libraries:"
  sed 's|^[^:]*: *||' /tmp/plugins | sort -u
fi
if [ -s /tmp/core ]; then cat /tmp/core; exit 1; fi
if [ -s /tmp/plugins ] && [ "${STRICT:-}" = 1 ]; then exit 1; fi
echo "ok"
