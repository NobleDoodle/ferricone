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
# A PNG, so that file managers and the AppImage catalog can show it as a thumbnail.
cp /src/assets/ferricone-256.png "$appdir/.DirIcon"

image=$out/Ferricone_Player-$version-$arch.AppImage
# Update information and a .zsync file for AppImageUpdate and similar tools,
# beside Ferricone's own updater (src/update.rs). Releases only: "latest"
# would take a development build back to the last release.
update=()
case $version in
  *-*) ;;
  *) update=(-u "gh-releases-zsync|NobleDoodle|ferricone|latest|Ferricone_Player-*-$arch.AppImage.zsync") ;;
esac
(cd "$out" && ARCH=$arch appimagetool --no-appstream "${update[@]}" "$appdir" "$image" >/dev/null)
finish "$image"
if [ ${#update[@]} -gt 0 ]; then
  [ -s "$image.zsync" ] || { echo "no $image.zsync" >&2; exit 1; }
  finish "$image.zsync"
fi
