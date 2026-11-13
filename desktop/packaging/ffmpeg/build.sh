#!/usr/bin/env bash
set -euo pipefail

# Build shared, LGPL-only, network-disabled Windows x64 FFmpeg tools on Linux.
# Dependencies: curl, gpg, sha256sum, xz, make, nasm, and the MinGW-w64 win32 GCC toolchain.
# NDM_GCC_COPYRIGHT can override Debian's GCC 14 copyright file on other hosts.
if [[ $# != 1 ]]; then
    echo "Usage: $0 EMPTY_OUTPUT_DIRECTORY" >&2
    exit 2
fi
version=9.0.2
archive="ffmpeg-${version}.tar.xz"
expected_sha=8c3850283eb25fa026482078a04051e0be17347b09ef81a0849bec15a96e002e
expected_key=FCF986EA15E6E293A5644F10B4322F04D67658D8
script=$(realpath "$0")
out=$(realpath -m "$1")
mkdir -p "$out"
if [[ -n $(ls -A "$out") ]]; then
    echo "Output directory must be empty: $out" >&2
    exit 2
fi
scratch=$(mktemp -d "${TMPDIR:-/tmp}/ndm-ffmpeg-build.XXXXXXXX")
trap 'rm -rf -- "$scratch"' EXIT
mkdir -m 700 "$scratch/gnupg"

for name in "$archive" "$archive.asc" ffmpeg-devel.asc; do
    if [[ -n ${NDM_FFMPEG_ARCHIVE_DIR:-} ]]; then
        cp -- "$NDM_FFMPEG_ARCHIVE_DIR/$name" "$scratch/$name"
    else
        case "$name" in
            ffmpeg-devel.asc) url="https://ffmpeg.org/$name" ;;
            *) url="https://ffmpeg.org/releases/$name" ;;
        esac
        curl --fail --location --retry 3 --output "$scratch/$name" "$url"
    fi
done
echo "$expected_sha  $scratch/$archive" | sha256sum --check --status
actual_key=$(gpg --batch --homedir "$scratch/gnupg" --with-colons --show-keys "$scratch/ffmpeg-devel.asc" | awk -F: '$1 == "fpr" { print $10; exit }')
[[ $actual_key == "$expected_key" ]] || { echo 'Unexpected FFmpeg signing key' >&2; exit 1; }
gpg --batch --homedir "$scratch/gnupg" --import "$scratch/ffmpeg-devel.asc"
gpg --batch --homedir "$scratch/gnupg" --verify "$scratch/$archive.asc" "$scratch/$archive"
tar -xf "$scratch/$archive" -C "$scratch"
mkdir "$scratch/build"
cd "$scratch/build"
"$scratch/ffmpeg-$version/configure" \
    --prefix="$scratch/install" --target-os=mingw32 --arch=x86_64 \
    --cross-prefix=x86_64-w64-mingw32- --cc=x86_64-w64-mingw32-gcc-win32 \
    --enable-cross-compile --extra-ldflags=-static-libgcc \
    --disable-static --enable-shared --disable-autodetect --disable-debug --disable-doc \
    --disable-gpl --disable-version3 --disable-nonfree --disable-network --disable-everything \
    --enable-ffmpeg --enable-ffprobe --enable-protocol=file,pipe,crypto,data \
    --enable-demuxer=hls,mpegts,mov,aac,mp3,ac3,eac3,matroska,webvtt,concat \
    --enable-muxer=mp4,matroska \
    --enable-parser=aac,aac_latm,h264,hevc,av1,mpegaudio,ac3,opus,vorbis,flac,vp9,mpeg4video,mpegvideo \
    --enable-bsf=aac_adtstoasc,h264_mp4toannexb,hevc_mp4toannexb,av1_frame_merge,extract_extradata \
    --enable-decoder=h264,hevc,av1,aac,aac_latm,mp2,mp3,ac3,eac3,opus,vorbis,flac,vp9,mpeg4,mpeg2video
make -j "${NDM_BUILD_JOBS:-4}"
make install

mkdir -p "$out/bin" "$out/source"
cp -- "$scratch/install/bin/"*.exe "$scratch/install/bin/"*.dll "$out/bin/"
cp -- "$scratch/$archive" "$scratch/$archive.asc" "$scratch/ffmpeg-devel.asc" "$out/source/"
cp -- "$scratch/ffmpeg-$version/COPYING.LGPLv2.1" "$scratch/ffmpeg-$version/LICENSE.md" "$out/source/"
cp -- "${NDM_GCC_COPYRIGHT:-/usr/share/doc/gcc-14-base/copyright}" "$out/source/COPYING.GCC-RUNTIME"
cp -- /usr/share/common-licenses/GPL-3 "$out/source/COPYING.GPL-3"
cp -- "$script" "$out/source/build.sh"
{
    echo "FFmpeg $version; shared LGPL 2.1-or-later build; networking disabled"
    echo "Corresponding unmodified source and the complete build script are in this directory."
    echo "The shared libraries can be replaced by compatible modified builds."
    echo "GCC runtime components are covered by the GCC Runtime Library Exception; see COPYING.GCC-RUNTIME."
    echo "FFmpeg source SHA-256: $expected_sha"
    x86_64-w64-mingw32-gcc-win32 --version | head -n 1
    nasm -v
} > "$out/source/BUILD-INFO.txt"
(cd "$out" && sha256sum bin/* source/* > SHA256SUMS)
echo "Media tools: $out"
