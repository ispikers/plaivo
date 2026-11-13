#!/usr/bin/env bash
set -euo pipefail

deps=$(cd "$(dirname "$0")" && pwd)
source "$deps/libmpv.env"
mkdir -p "$1" "${2:-libmpv-build}"
out=$(cd "$1" && pwd)
work=$(cd "${2:-libmpv-build}" && pwd)
prefix=$work/prefix
jobs=$(getconf _NPROCESSORS_ONLN)
export PKG_CONFIG_PATH=$prefix/lib/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}

case "$(uname -s)" in
  MINGW*)
    ffmpeg_os=(--enable-d3d11va --enable-dxva2 --enable-openssl)
    placebo_os=(-Dd3d11=enabled)
    mpv_os=(-Dd3d11=enabled -Dd3d-hwaccel=enabled -Dshaderc=enabled -Dspirv-cross=enabled -Dvulkan=enabled)
    ;;
  Darwin)
    ffmpeg_os=(--enable-videotoolbox --enable-audiotoolbox --enable-securetransport --enable-iconv)
    placebo_os=(-Dd3d11=disabled)
    mpv_os=(-Dcocoa=enabled -Dswift-build=enabled -Dvulkan=enabled)
    ;;
  *) echo "unsupported host: $(uname -s)" >&2; exit 1 ;;
esac

cd "$work"
git clone -q --depth 1 --branch "$FFMPEG" https://github.com/FFmpeg/FFmpeg ffmpeg
git -C ffmpeg apply "$deps/ffmpeg-http-backport.patch" "$deps/ffmpeg-http-keepalive.patch"
git clone -q --depth 1 --branch "$MPV" https://github.com/mpv-player/mpv mpv
git -C mpv apply "$deps/mpv-macos-embed.patch"
git clone -q --depth 1 --branch "$LIBPLACEBO" --recurse-submodules=3rdparty --shallow-submodules \
  https://code.videolan.org/videolan/libplacebo.git libplacebo

(
  cd ffmpeg
  ./configure --prefix="$prefix" --enable-shared --disable-static --disable-programs --disable-doc \
    --disable-debug --disable-indevs --disable-outdevs --disable-encoders --disable-muxers --enable-muxer=spdif \
    --enable-zlib --enable-libdav1d "${ffmpeg_os[@]}"
  grep -q 'FFMPEG_LICENSE "LGPL version 2.1 or later"' config.h
  make -j"$jobs" install
)

# Installed rather than an mpv subproject: mpv's Swift build can't see the subproject's generated config.h.
meson setup libplacebo/build libplacebo --prefix="$prefix" --buildtype=release \
  -Ddemos=false -Dtests=false -Dvulkan=enabled -Dshaderc=enabled -Dlcms=enabled "${placebo_os[@]}"
meson install -C libplacebo/build

meson setup mpv/build mpv --prefix="$prefix" --buildtype=release \
  -Dgpl=false -Dlibmpv=true -Dcplayer=false -Dtests=false \
  -Dlua=disabled -Djavascript=disabled -Dlibarchive=disabled -Dlibbluray=disabled -Ddvdnav=disabled \
  -Dcdda=disabled -Dmanpage-build=disabled -Djpeg=disabled -Duchardet=enabled -Dlcms2=enabled "${mpv_os[@]}"
meson install -C mpv/build

case "$(uname -s)" in
  MINGW*)
    cp "$prefix/bin/libmpv-2.dll" "$out/"
    libs=$(PATH="$prefix/bin:$PATH" ldd "$out/libmpv-2.dll")
    if grep -qE 'not found|\?\?\?' <<<"$libs"; then echo "$libs" >&2; exit 1; fi
    grep -vi '=> /c/windows/' <<<"$libs" | awk '{print $3}' | grep -v "^$out/" | xargs -r cp -t "$out"
    strip --strip-unneeded "$out"/*.dll
    ;;
  Darwin)
    cp -L "$prefix/lib/libmpv.2.dylib" "$out/"
    install_name_tool -id @loader_path/libmpv.2.dylib "$out/libmpv.2.dylib"
    dylibbundler -of -b -x "$out/libmpv.2.dylib" -d "$out" -p @loader_path/ -s "$prefix/lib" -s "$(brew --prefix)/lib"
    # The Vulkan loader finds MoltenVK only through ICD manifests outside an app bundle, so link MoltenVK directly instead.
    cp -L "$(brew --prefix molten-vk)/lib/libMoltenVK.dylib" "$out/"
    chmod u+w "$out"/*.dylib
    install_name_tool -id @loader_path/libMoltenVK.dylib "$out/libMoltenVK.dylib"
    for lib in "$out"/*.dylib; do
      otool -L "$lib" | awk 'NR > 1 && /libvulkan/ {print $1}' | while read -r dep; do
        install_name_tool -change "$dep" @loader_path/libMoltenVK.dylib "$lib"
      done
    done
    rm -f "$out"/libvulkan*.dylib
    codesign --force -s - "$out"/*.dylib
    if otool -L "$out"/*.dylib | grep -E "$prefix|/opt/homebrew|/usr/local|@rpath|libvulkan"; then exit 1; fi
    ;;
esac
