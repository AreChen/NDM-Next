#!/usr/bin/env bash
set -euo pipefail

# Native macOS build: shared, LGPL-only FFmpeg with networking disabled.
# Requires Xcode command-line tools, curl, gpg, xz, make, and nasm (on Intel).
if [[ $# != 1 || $(uname -s) != Darwin ]]; then
    echo "Usage (on macOS): $0 EMPTY_OUTPUT_DIRECTORY" >&2
    exit 2
fi
version=9.0.2
archive="ffmpeg-${version}.tar.xz"
expected_sha=8c3850283eb25fa026482078a04051e0be17347b09ef81a0849bec15a96e002e
expected_key=FCF986EA15E6E293A5644F10B4322F04D67658D8
script_dir=$(cd "$(dirname "$0")" && pwd)
mkdir -p "$1"
out=$(cd "$1" && pwd)
if [[ -n $(ls -A "$out") ]]; then
    echo "Output directory must be empty: $out" >&2
    exit 2
fi
scratch=$(mktemp -d "${TMPDIR:-/tmp}/ndm-ffmpeg-build.XXXXXXXX")
trap 'rm -rf "$scratch"' EXIT
mkdir -m 700 "$scratch/gnupg"
for name in "$archive" "$archive.asc" ffmpeg-devel.asc; do
    if [[ -n ${NDM_FFMPEG_ARCHIVE_DIR:-} ]]; then
        cp "$NDM_FFMPEG_ARCHIVE_DIR/$name" "$scratch/$name"
    else
        case "$name" in
            ffmpeg-devel.asc) url="https://ffmpeg.org/$name" ;;
            *) url="https://ffmpeg.org/releases/$name" ;;
        esac
        curl --fail --location --retry 3 --output "$scratch/$name" "$url"
    fi
done
printf '%s  %s\n' "$expected_sha" "$scratch/$archive" | shasum -a 256 --check --status
actual_key=$(gpg --batch --homedir "$scratch/gnupg" --with-colons --show-keys "$scratch/ffmpeg-devel.asc" | awk -F: '$1 == "fpr" { print $10; exit }')
[[ $actual_key == "$expected_key" ]] || { echo 'Unexpected FFmpeg signing key' >&2; exit 1; }
gpg --batch --homedir "$scratch/gnupg" --import "$scratch/ffmpeg-devel.asc"
gpg --batch --homedir "$scratch/gnupg" --verify "$scratch/$archive.asc" "$scratch/$archive"
tar -xf "$scratch/$archive" -C "$scratch"
mkdir "$scratch/build"
cd "$scratch/build"
export MACOSX_DEPLOYMENT_TARGET=${MACOSX_DEPLOYMENT_TARGET:-12.0}
"$scratch/ffmpeg-$version/configure" \
    --prefix="$scratch/install" --target-os=darwin --arch="$(uname -m)" --cc=clang \
    --install-name-dir=@rpath --extra-ldflags=-Wl,-rpath,@executable_path \
    --disable-static --enable-shared --disable-autodetect --disable-debug --disable-doc \
    --disable-gpl --disable-version3 --disable-nonfree --disable-network --disable-everything \
    --enable-ffmpeg --enable-ffprobe --enable-protocol=file,pipe,crypto,data \
    --enable-demuxer=hls,mpegts,mov,aac,mp3,ac3,eac3,matroska,webvtt,concat \
    --enable-muxer=mp4,matroska,webm \
    --enable-parser=aac,aac_latm,h264,hevc,av1,mpegaudio,ac3,opus,vorbis,flac,vp9,mpeg4video,mpegvideo \
    --enable-bsf=aac_adtstoasc,h264_mp4toannexb,hevc_mp4toannexb,av1_frame_merge,extract_extradata \
    --enable-decoder=h264,hevc,av1,aac,aac_latm,mp2,mp3,ac3,eac3,opus,vorbis,flac,vp9,mpeg4,mpeg2video
make -j "${NDM_BUILD_JOBS:-4}"
make install
mkdir -p "$out/bin" "$out/source"
cp "$scratch/install/bin/ffmpeg" "$scratch/install/bin/ffprobe" "$out/bin/"
cp -P "$scratch/install/lib/"*.dylib "$out/bin/"
# Sign inner libraries before executables. Signatures remain valid after relocation.
find "$out/bin" -type f -name '*.dylib' -exec codesign --force --sign - {} \;
codesign --force --sign - "$out/bin/ffmpeg" "$out/bin/ffprobe"
"$out/bin/ffmpeg" -hide_banner -version
cp "$scratch/$archive" "$scratch/$archive.asc" "$scratch/ffmpeg-devel.asc" "$out/source/"
cp "$scratch/ffmpeg-$version/COPYING.LGPLv2.1" "$scratch/ffmpeg-$version/LICENSE.md" "$out/source/"
cp "$script_dir/build-media.sh" "$out/source/build-media.sh"
{
    echo "FFmpeg $version; shared LGPL 2.1-or-later build; networking disabled"
    echo "Corresponding unmodified source and complete build script are in this directory."
    echo "Shared libraries can be replaced by compatible modified builds."
    echo "FFmpeg source SHA-256: $expected_sha"
    echo "MACOSX_DEPLOYMENT_TARGET=$MACOSX_DEPLOYMENT_TARGET"
    clang --version
} > "$out/source/BUILD-INFO.txt"
(cd "$out" && shasum -a 256 bin/* source/* > SHA256SUMS)
echo "Media tools: $out"
