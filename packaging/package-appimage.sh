#!/usr/bin/env bash
# package-appimage.sh <bundle>: dist/Ferricone_Player-<version>-<arch>.AppImage,
# for any distribution: one file, made executable and run.
source /src/packaging/common.sh "$@"

appdir=$(mktemp -d)/Ferricone.AppDir
mkdir -p "$appdir"
cp -a "$bundle"/. "$appdir/usr/"
cat > "$appdir/AppRun" <<'APPRUN'
#!/bin/sh
here=$(dirname "$(readlink -f "$0")")
exec "$here/usr/bin/ferricone" "$@"
APPRUN
chmod +x "$appdir/AppRun"
cp "$bundle/share/applications/$app.desktop" "$appdir/$app.desktop"
cp "$bundle/share/icons/hicolor/scalable/apps/$app.svg" "$appdir/$app.svg"
ln -s "$app.svg" "$appdir/.DirIcon"

image=$out/Ferricone_Player-$version-$arch.AppImage
ARCH=$arch appimagetool --no-appstream "$appdir" "$image" >/dev/null
finish "$image"
