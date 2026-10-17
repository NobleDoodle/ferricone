#!/usr/bin/env bash
# package-tarball.sh <bundle>: dist/ferricone-<version>-x86_64.tar.gz, the
# bundle as it is, with install.sh / uninstall.sh for ~/.local.
source /src/packaging/common.sh "$@"

# A copy, so the scripts below don't end up in the other packages.
name=$(basename "$bundle")
work=$(mktemp -d)
cp -a "$bundle" "$work/$name"
bundle=$work/$name
cat > "$bundle/install.sh" <<'INSTALL'
#!/bin/sh
# Installs this Ferricone for the current user: ~/.local/opt/ferricone, the
# command in ~/.local/bin, and the menu entry and icon.
set -e
here=$(cd "$(dirname "$0")" && pwd)
dest=${XDG_DATA_HOME:-$HOME/.local/share}
opt=$HOME/.local/opt/ferricone
app=io.github.nobledoodle.ferricone
rm -rf "$opt" && mkdir -p "$opt" "$HOME/.local/bin" "$dest/applications"
cp -a "$here"/bin "$here"/lib "$here"/libexec "$opt/"
mkdir -p "$opt/share" && cp -a "$here/share/vlc" "$opt/share/"
ln -sf "$opt/bin/ferricone" "$HOME/.local/bin/ferricone"
sed "s|^Exec=ferricone|Exec=$opt/bin/ferricone|" "$here/share/applications/$app.desktop" > "$dest/applications/$app.desktop"
mkdir -p "$dest/icons/hicolor/scalable/apps" "$dest/icons/hicolor/symbolic/apps"
cp "$here/share/icons/hicolor/scalable/apps/$app.svg" "$dest/icons/hicolor/scalable/apps/"
cp "$here/share/icons/hicolor/symbolic/apps/$app-symbolic.svg" "$dest/icons/hicolor/symbolic/apps/"
command -v update-desktop-database >/dev/null && update-desktop-database "$dest/applications" || true
echo "Installed. Run ferricone, or find Ferricone Player in your app menu."
INSTALL
cat > "$bundle/uninstall.sh" <<'UNINSTALL'
#!/bin/sh
# Removes what install.sh put in ~/.local. Settings and the library stay in
# ~/.config/ferricone, ~/.local/state/ferricone and ~/.local/share/ferricone.
dest=${XDG_DATA_HOME:-$HOME/.local/share}
app=io.github.nobledoodle.ferricone
rm -rf "$HOME/.local/opt/ferricone" "$HOME/.local/bin/ferricone" "$dest/applications/$app.desktop" \
  "$dest/icons/hicolor/scalable/apps/$app.svg" "$dest/icons/hicolor/symbolic/apps/$app-symbolic.svg"
echo "Removed."
UNINSTALL
chmod +x "$bundle/install.sh" "$bundle/uninstall.sh"

tarball=$out/$name.tar.gz
tar -C "$(dirname "$bundle")" -czf "$tarball" "$name"
finish "$tarball"
