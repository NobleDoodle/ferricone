#!/usr/bin/env bash
# Builds the pinned libVLC 4 into vendor/vlc. Nothing is installed system-wide.
# Re-run after installing new build dependencies: meson reconfigures and only
# the newly enabled modules are compiled.
set -euo pipefail

VLC_COMMIT=a5538c557d276378d5ba0f8f8010a8b87eaa6f78   # master, 2026-10-12

root=$(cd "$(dirname "$0")/.." && pwd)
# The packaging container builds elsewhere (packaging/bundle.sh).
src=${VLC_SRC_DIR:-$root/vendor/vlc-src}
build=${VLC_BUILD_DIR:-$root/vendor/vlc-build}
prefix=${VLC_PREFIX:-$root/vendor/vlc}

if [ ! -d "$src/.git" ]; then
  git clone -q --no-checkout --filter=blob:none https://code.videolan.org/videolan/vlc.git "$src"
fi
if [ "$(git -C "$src" rev-parse HEAD 2>/dev/null)" != "$VLC_COMMIT" ]; then
  git -C "$src" fetch -q --depth 1 origin "$VLC_COMMIT"
  git -C "$src" checkout -q --detach "$VLC_COMMIT"
fi

# Ferricone's libVLC patches, applied in order to clean sources whenever
# they change (a stamp in VLC's .git remembers the set applied last). 0004
# and 0005 touch the same file, so they can't be checked one by one.
stamp=$src/.git/ferricone-patches
want=$(cat "$root"/patches/*.patch | sha256sum | cut -d' ' -f1)
if [ "$(cat "$stamp" 2>/dev/null)" != "$want" ]; then
  git -C "$src" reset -q --hard
  for patch in "$root"/patches/*.patch; do
    git -C "$src" apply "$patch"
  done
  echo "$want" > "$stamp"
fi

opts=(
  --prefix="$prefix" --libdir=lib --buildtype=release
  -Dtests=disabled -Dnls=disabled
  # Ferricone is the interface
  -Dqt=disabled -Dqt_gtk=disabled -Dskins2=disabled -Dncurses=disabled
  # video reaches Ferricone through OpenGL output callbacks, not Vulkan
  -Dvulkan=disabled
  # GPL-only pieces stay out so the license decision stays open
  -Dx264=disabled -Dx265=disabled -Dx262=disabled -Dpostproc=disabled
  -Dfaad=disabled -Dmad=disabled -Dsid=disabled
)

if [ -f "$build/build.ninja" ]; then
  meson setup --reconfigure "$build" "$src" "${opts[@]}"
else
  meson setup "$build" "$src" "${opts[@]}"
fi
meson compile -C "$build"
meson install -C "$build" --quiet
echo "libVLC installed in $prefix"
