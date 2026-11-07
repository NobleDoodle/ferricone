#!/usr/bin/env bash
# Builds the bundle every package wraps, inside the packaging container
# (packaging/build.sh runs it): libVLC 4 with Ferricone's patches, and
# Ferricone, laid out as they install under /opt/ferricone, with the
# libraries a desktop doesn't already have in lib/deps.
#
#   dist/ferricone-<version>-<arch>/
#     bin/ferricone        rpath $ORIGIN/../lib:…/lib/vlc:…/lib/deps
#     lib/                 libvlc, libvlccore, vlc/plugins, deps/
#     libexec/vlc/         vlc-preparser (thumbnails, durations)
#     share/               vlc/lua, applications, icons, metainfo, doc
set -euo pipefail

src=/src
arch=$(uname -m)
cache=$src/.package-cache
version=$(sed -n 's/^version = "\(.*\)"/\1/p' "$src/Cargo.toml" | head -1)
bundle=$src/dist/ferricone-$version-$arch
prefix=/opt/ferricone

git config --global --add safe.directory '*'

# libVLC, installed where the packages put it (VLC finds lib/vlc, libexec
# and share relative to libvlccore).
VLC_SRC_DIR=$cache/vlc-src VLC_BUILD_DIR=$cache/$arch/vlc-build VLC_PREFIX=$prefix \
  "$src/scripts/build-libvlc.sh"

# Ferricone, linked against it, finding its libraries next to itself.
export FERRICONE_LIBVLC_DIR=$prefix
export FERRICONE_RPATH='$ORIGIN/../lib:$ORIGIN/../lib/vlc:$ORIGIN/../lib/deps'
export CARGO_HOME=$cache/cargo
cargo build --release --locked --manifest-path "$src/Cargo.toml" --target-dir "$cache/$arch/target"

rm -rf "$bundle"
mkdir -p "$bundle"/{bin,lib,libexec,share}
install -m755 "$cache/$arch/target/release/ferricone" "$bundle/bin/ferricone"
cp -a "$prefix"/lib/libvlc.so* "$prefix"/lib/libvlccore.so* "$prefix"/lib/libvlc_*.so* "$prefix/lib/vlc" "$bundle/lib/"
cp -a "$prefix/libexec/vlc" "$bundle/libexec/"
mkdir -p "$bundle/share/vlc" && cp -a "$prefix/share/vlc/lua" "$bundle/share/vlc/"

# Desktop integration and documents.
app=io.github.nobledoodle.ferricone
install -Dm644 "$src/packaging/$app.desktop" "$bundle/share/applications/$app.desktop"
install -Dm644 "$src/packaging/$app.metainfo.xml" "$bundle/share/metainfo/$app.metainfo.xml"
install -Dm644 "$src/assets/ferricone.svg" "$bundle/share/icons/hicolor/scalable/apps/$app.svg"
install -Dm644 "$src/assets/ferricone-symbolic.svg" "$bundle/share/icons/hicolor/symbolic/apps/$app-symbolic.svg"
for doc in README.md LICENSE; do install -Dm644 "$src/$doc" "$bundle/share/doc/ferricone/$doc"; done
desktop-file-validate "$bundle/share/applications/$app.desktop"

python3 "$src/packaging/collect-deps.py" "$bundle"

# Smoke test: the binary starts and loads the bundled libVLC.
"$bundle/bin/ferricone" --version

[ -n "${HOST_UID:-}" ] && chown -R "$HOST_UID:${HOST_GID:-$HOST_UID}" "$src/dist" "$cache"
echo "bundle: $bundle ($(du -sh "$bundle" | cut -f1))"
