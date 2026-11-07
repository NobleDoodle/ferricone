# The build environment for Ferricone's Linux packages: Ubuntu 22.04, whose
# glibc (2.35) is older than every distribution the packages target, so one
# build runs on all of them. libVLC 4 and Ferricone are compiled here, then
# bundled (packaging/bundle.sh) with the libraries a desktop doesn't
# already have.
FROM docker.io/library/ubuntu:22.04

ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential pkg-config git ca-certificates curl file python3-pip ninja-build \
    flex bison gettext nasm patchelf rpm zstd desktop-file-utils \
    # libVLC's modules (those whose libraries are missing are skipped)
    libavcodec-dev libavformat-dev libavutil-dev libswscale-dev libswresample-dev \
    libass-dev libfreetype-dev libfontconfig-dev libfribidi-dev libharfbuzz-dev \
    libva-dev libvdpau-dev libdrm-dev libegl-dev libgl-dev libgles-dev \
    libwayland-dev wayland-protocols libxkbcommon-dev libx11-dev libx11-xcb-dev \
    libxcb1-dev libxcb-shm0-dev libxcb-xv0-dev libxcb-keysyms1-dev libxcb-randr0-dev libxcb-composite0-dev \
    libpulse-dev libpipewire-0.3-dev libasound2-dev libdbus-1-dev \
    libmatroska-dev libebml-dev libdvbpsi-dev libmpg123-dev libvorbis-dev libogg-dev \
    libopus-dev libflac-dev libtheora-dev libvpx-dev libdav1d-dev libspeex-dev libspeexdsp-dev \
    libsoxr-dev libsamplerate0-dev libtag1-dev libxml2-dev libgnutls28-dev \
    libprotobuf-dev protobuf-compiler libarchive-dev \
    libbluray-dev libdvdnav-dev libdvdread-dev libsecret-1-dev libgcrypt20-dev libgbm-dev bzip2 \
    zlib1g-dev libpng-dev libjpeg-dev libudev-dev libmodplug-dev libopenmpt-dev \
    libmad0-dev liba52-0.7.4-dev libdca-dev libtwolame-dev libshout3-dev libnfs-dev \
    libsmbclient-dev libchromaprint-dev libaribb24-dev libzvbi-dev liblua5.4-dev lua5.4 \
    # Ferricone (Slint on winit, rfd's portal and MPRIS over D-Bus)
    libxcursor-dev libxrandr-dev libxi-dev libxkbcommon-x11-dev \
    && rm -rf /var/lib/apt/lists/*

# VLC 4 needs a newer Meson than 22.04's.
RUN pip3 install --no-cache-dir 'meson>=1.10'

# libmicrodns (mDNS: finding Chromecasts) isn't packaged for 22.04.
RUN git clone --depth 1 -b 0.2.0 https://github.com/videolabs/libmicrodns /tmp/microdns \
    && meson setup /tmp/microdns/build /tmp/microdns --prefix=/usr/local --libdir=lib -Dexamples=disabled -Dtests=disabled \
    && ninja -C /tmp/microdns/build install && ldconfig && rm -rf /tmp/microdns

# libupnp (UPnP media servers and renderers): VLC 4 wants 1.8.5 or newer,
# 22.04 has 1.8.4.
RUN curl -sSL --retry 5 --retry-all-errors -o /tmp/upnp.tar.bz2 https://github.com/pupnp/pupnp/releases/download/release-1.14.24/libupnp-1.14.24.tar.bz2 \
    && echo '8dfde05f63e91644e907dcfb7305955ad064b4cf6e5103cb37a7c57c50d1dd11  /tmp/upnp.tar.bz2' | sha256sum -c - \
    && tar -C /tmp -xjf /tmp/upnp.tar.bz2 && cd /tmp/libupnp-1.14.24 \
    && ./configure --prefix=/usr/local --disable-samples --disable-static >/dev/null \
    && make -j"$(nproc)" >/dev/null && make install >/dev/null && ldconfig \
    && cd / && rm -rf /tmp/libupnp-1.14.24 /tmp/upnp.tar.bz2

# Nettle 3.10.2, the newest with sonames libnettle.so.8 and libhogweed.so.6:
# bundled for libarchive and libsrt, it also serves the system's GnuTLS
# (which needs functions 22.04's 3.7 lacks) on distributions still on .so.8.
RUN apt-get update && apt-get install -y --no-install-recommends m4 libgmp-dev && rm -rf /var/lib/apt/lists/* \
    && curl -sSL --retry 5 --retry-all-errors -o /tmp/nettle.tar.gz https://ftp.gnu.org/gnu/nettle/nettle-3.10.2.tar.gz \
    && echo 'fe9ff51cb1f2abb5e65a6b8c10a92da0ab5ab6eaf26e7fc2b675c45f1fb519b5  /tmp/nettle.tar.gz' | sha256sum -c - \
    && tar -C /tmp -xzf /tmp/nettle.tar.gz && cd /tmp/nettle-3.10.2 \
    && ./configure --prefix=/usr/local --libdir=/usr/local/lib --disable-documentation --disable-openssl --disable-static >/dev/null \
    && make -j"$(nproc)" >/dev/null && make install >/dev/null && ldconfig \
    && cd / && rm -rf /tmp/nettle-3.10.2 /tmp/nettle.tar.gz

# Rust (edition 2024).
ENV RUSTUP_HOME=/opt/rustup CARGO_HOME=/opt/cargo PATH=/opt/cargo/bin:$PATH
RUN curl -sSf --retry 5 --retry-all-errors https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable

# AppImage's tool, run unpacked (no FUSE in containers). Its magic bytes
# ("AI\2" in the ELF header's padding) are zeroed: with them, binfmt_misc
# doesn't hand an arm64 AppImage to QEMU on an x86-64 machine.
RUN curl -sSL --retry 5 --retry-all-errors -o /usr/local/bin/appimagetool \
      https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-"$(uname -m)".AppImage \
    && printf '\0\0\0' | dd of=/usr/local/bin/appimagetool bs=1 seek=8 conv=notrunc status=none \
    && chmod +x /usr/local/bin/appimagetool
ENV APPIMAGE_EXTRACT_AND_RUN=1
