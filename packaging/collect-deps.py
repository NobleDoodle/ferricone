#!/usr/bin/env python3
"""collect-deps.py <bundle>: copies into <bundle>/lib/deps every shared
library the binary, libVLC and its plugins need that a Linux desktop
doesn't already provide (glibc, the GL/Vulkan/VA drivers' libraries, X11 and
Wayland clients, audio servers, fontconfig/freetype, D-Bus, glib — those must
match the running system), then strips everything. The binary's DT_RPATH
($ORIGIN/../lib/deps) also serves the libraries the plugins load, since none
of them carry a RUNPATH of their own."""

import os
import re
import shutil
import subprocess
import sys

bundle = sys.argv[1]
deps = os.path.join(bundle, "lib", "deps")
os.makedirs(deps, exist_ok=True)

# Sonames left to the system, by prefix.
SYSTEM = [
    "ld-linux", "linux-vdso", "libc.so", "libm.so", "libdl.so", "libpthread.so", "librt.so",
    "libresolv.so", "libutil.so", "libanl.so", "libnsl.so", "libmvec.so",
    "libstdc++.so", "libgcc_s.so",
    "libGL.so", "libGLX", "libGLdispatch", "libEGL", "libGLES", "libOpenGL", "libgbm",
    "libdrm", "libvulkan", "libva", "libvdpau",
    "libwayland-", "libX11", "libxcb", "libXext", "libXrender", "libXau", "libXdmcp",
    "libXfixes", "libXi", "libXrandr", "libXcursor", "libXinerama", "libxkbcommon",
    "libasound", "libpulse", "libpipewire", "libjack",
    "libdbus-1", "libsystemd", "libudev", "libcap.so",
    "libfontconfig", "libfreetype", "libharfbuzz", "libexpat", "libz.so",
    "libglib-2.0", "libgobject-2.0", "libgio-2.0", "libgmodule-2.0", "libgthread-2.0",
    "libsecret-1", "libselinux", "libmount", "libblkid", "libuuid", "libpcre",
    "libgcrypt", "libgpg-error",
    # Libraries the system's own libraries load too (libsecret needs GnuTLS,
    # PulseAudio libsndfile and its codecs, Mesa zstd): one bundled from
    # 22.04 would shadow the system's newer copy for them. Same soname on
    # every target distribution. (Nettle stays bundled, as 3.10.2: see the
    # Dockerfile.)
    "libgnutls.so", "libp11-kit.so", "libgmp.so",
    "libtasn1.so", "libidn2.so", "libffi.so", "libpcre2-8.so",
    "libzstd.so", "liblzma.so", "liblz4.so", "libbrotli", "libpng16.so",
    "libsndfile.so", "libopus.so", "libvorbis.so", "libvorbisenc.so", "libogg.so",
    "libmpg123.so", "libmp3lame.so",
]


def is_system(name: str) -> bool:
    return any(name.startswith(p) for p in SYSTEM)


def elf_files():
    for root, _, files in os.walk(bundle):
        for f in files:
            path = os.path.join(root, f)
            if os.path.islink(path) or not os.path.isfile(path):
                continue
            with open(path, "rb") as h:
                if h.read(4) == b"\x7fELF":
                    yield path


# /usr/local/lib first: the libraries the Dockerfile builds from source
# (Nettle 3.10.2, libupnp, libmicrodns) must win over Ubuntu's, whatever
# order ld.so.cache ended up in (ldconfig crashes under QEMU's arm64).
env = dict(os.environ, LD_LIBRARY_PATH=":".join(
    [os.path.join(bundle, d) for d in ("lib", "lib/vlc", "lib/deps")] + ["/usr/local/lib"]))
line = re.compile(r"^\s*(\S+) => (\S+) \(0x")

copied = set()
while True:
    new = 0
    for path in list(elf_files()):
        out = subprocess.run(["ldd", path], capture_output=True, text=True, env=env).stdout
        for m in map(line.match, out.splitlines()):
            if not m:
                continue
            soname, real = m.groups()
            if is_system(soname) or real.startswith(bundle) or soname in copied:
                continue
            shutil.copy2(os.path.realpath(real), os.path.join(deps, soname))
            copied.add(soname)
            new += 1
    if not new:
        break

# The libraries left to the system, for the packages' dependencies: each
# file's direct NEEDED entries (what those libraries need in turn is the
# system's business: libpulse's private libpulsecommon-<version> differs
# per release). "required" when the binary, libVLC or a bundled library
# needs it, "optional" when only plugins do (VLC skips a plugin it can't
# load).
needed = {}
for path in elf_files():
    plugin = "/lib/vlc/plugins/" in path
    out = subprocess.run(["readelf", "-d", path], capture_output=True, text=True).stdout
    for soname in re.findall(r"\(NEEDED\)\s+Shared library: \[(.+?)\]", out):
        if is_system(soname) and not soname.startswith(("ld-linux", "linux-vdso")):
            needed[soname] = needed.get(soname, True) and plugin
# What the binary dlopens instead of linking (winit, glutin, fontique,
# rfd's portal over D-Bus): no NEEDED entry, so listed by hand.
for soname in ["libEGL.so.1", "libGL.so.1", "libwayland-client.so.0", "libwayland-egl.so.1",
               "libX11.so.6", "libX11-xcb.so.1", "libxcb.so.1", "libXcursor.so.1", "libXi.so.6",
               "libXrender.so.1", "libxkbcommon.so.0", "libxkbcommon-x11.so.0",
               "libfontconfig.so.1", "libdbus-1.so.3"]:
    needed[soname] = False
with open(bundle.rstrip("/") + ".system-libs.txt", "w") as f:
    for soname in sorted(needed):
        f.write(f"{'optional' if needed[soname] else 'required'} {soname}\n")

missing = []
for path in elf_files():
    out = subprocess.run(["ldd", path], capture_output=True, text=True, env=env).stdout
    missing += [f"{os.path.relpath(path, bundle)}: {l.strip()}" for l in out.splitlines() if "not found" in l]
if missing:
    sys.exit("unresolved libraries:\n" + "\n".join(missing))

# No build-machine paths, and no debugging symbols. VLC's helper programs
# (vlc-preparser makes thumbnails and reads durations) get a DT_RPATH like
# the binary's, which also serves the plugins they load.
helper_rpath = "$ORIGIN/../../lib:$ORIGIN/../../lib/vlc:$ORIGIN/../../lib/deps"
for path in elf_files():
    rel = os.path.relpath(path, bundle)
    if rel.startswith("libexec/"):
        subprocess.run(["patchelf", "--force-rpath", "--set-rpath", helper_rpath, path], check=True)
    elif rel != "bin/ferricone":
        subprocess.run(["patchelf", "--remove-rpath", path], check=False, capture_output=True)
    subprocess.run(["strip", "--strip-unneeded", path], check=False, capture_output=True)
print(f"bundled {len(copied)} libraries into lib/deps")
