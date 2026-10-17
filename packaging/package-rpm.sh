#!/usr/bin/env bash
# package-rpm.sh <bundle>: dist/ferricone-<version>-1.x86_64.rpm, for
# Fedora, openSUSE and RHEL 10 (RHEL 9's glibc 2.34 is too old). It requires libraries by soname, which every
# RPM distribution provides under its own package names.
source /src/packaging/common.sh "$@"

top=$(mktemp -d)
mkdir -p "$top"/{BUILD,RPMS,SPECS,BUILDROOT}
cat > "$top/SPECS/ferricone.spec" <<SPEC
%global debug_package %{nil}
%global __os_install_post %{nil}
%global _build_id_links none
%global _binary_payload w19.zstdio

Name:           ferricone
Version:        $version
Release:        1
Summary:        $summary
License:        GPL-3.0-or-later
URL:            $homepage
BuildArch:      x86_64
AutoReqProv:    no
$(for l in $(syslibs required); do echo "Requires:       $l()(64bit)"; done)
Requires:       libc.so.6(GLIBC_2.35)(64bit)
$(for l in $(syslibs optional); do echo "Recommends:     $l()(64bit)"; done)
Recommends:     xdg-desktop-portal

%description
Ferricone plays videos, music and network streams on VLC's own engine,
libVLC 4 (bundled), with a new interface after Juxtopposed's "VLC Redesign":
a library beside the player, per-folder presets, a subtitle timeline,
OpenSubtitles, casting, and light, dark or system themes.

%install
mkdir -p %{buildroot}
cp -a $top/stage/. %{buildroot}/

%files
%license /usr/share/doc/ferricone/LICENSE
/opt/ferricone
/usr/bin/ferricone
/usr/share/applications/$app.desktop
/usr/share/metainfo/$app.metainfo.xml
/usr/share/icons/hicolor/scalable/apps/$app.svg
/usr/share/icons/hicolor/symbolic/apps/$app-symbolic.svg
/usr/share/doc/ferricone/README.md
SPEC
stage "$top/stage"
rpmbuild --quiet -bb --define "_topdir $top" "$top/SPECS/ferricone.spec"
rpm=$out/ferricone-$version-1.x86_64.rpm
cp "$top/RPMS/x86_64/$(basename "$rpm")" "$rpm"
finish "$rpm"
