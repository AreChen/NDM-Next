#!/usr/bin/env bash
set -euo pipefail

# Build a relocatable .app and drag-to-install DMG on the native architecture.
if [[ $# -lt 1 || $# -gt 2 || $(uname -s) != Darwin ]]; then
    echo "Usage (on macOS): $0 MEDIA_TOOL_DIRECTORY [OUTPUT_DIRECTORY]" >&2
    exit 2
fi
script_dir=$(cd "$(dirname "$0")" && pwd)
workspace=$(cd "$script_dir/../.." && pwd)
repo=$(cd "$workspace/.." && pwd)
media=$(cd "$1" && pwd)
mkdir -p "${2:-$workspace/dist}"
out=$(cd "${2:-$workspace/dist}" && pwd)
case $(uname -m) in
    arm64) target=aarch64-apple-darwin; arch=arm64 ;;
    x86_64) target=x86_64-apple-darwin; arch=x64 ;;
    *) echo 'Unsupported macOS architecture' >&2; exit 2 ;;
esac
export MACOSX_DEPLOYMENT_TARGET=${MACOSX_DEPLOYMENT_TARGET:-12.0}
(cd "$media" && shasum -a 256 --check --status SHA256SUMS)
configuration=$("$media/bin/ffmpeg" -hide_banner -buildconf 2>&1)
for flag in --disable-gpl --disable-version3 --disable-nonfree --disable-network --disable-autodetect --enable-shared; do
    [[ $configuration == *"$flag"* ]] || { echo "Media tools missing $flag" >&2; exit 1; }
done
for name in ffmpeg ffprobe; do
    lipo "$media/bin/$name" -verify_arch "$(uname -m)"
done
scratch=$(mktemp -d "${TMPDIR:-/tmp}/ndm-macos-package.XXXXXXXX")
trap 'rm -rf "$scratch"' EXIT
cd "$workspace"
cargo +1.100.0 fetch --locked
metadata=$(cargo +1.100.0 metadata --locked --no-deps --format-version 1)
version=$(printf '%s' "$metadata" | python3 -c 'import json,sys; print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"] == "ndm-desktop"))')
[[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo 'Unsupported version' >&2; exit 1; }
target_dir=$(printf '%s' "$metadata" | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')
cargo +1.100.0 build --locked --release --workspace --target "$target"
app="$scratch/image/OnesDM.app"
macos="$app/Contents/MacOS"
resources="$app/Contents/Resources"
mkdir -p "$macos/ffmpeg" "$resources/media-source"
cp "$target_dir/$target/release/ndm-desktop" "$target_dir/$target/release/ndm-host" "$macos/"
ln -s ndm-host "$macos/onesdm"
cp -RP "$media/bin/." "$macos/ffmpeg/"
cp -R "$media/source/." "$resources/media-source/"
cp "$media/SHA256SUMS" "$resources/media-source/SHA256SUMS.original"
cp "$repo/LICENSE" "$repo/PRIVACY.md" "$script_dir/README.md" "$resources/"
cp -R "$repo/extension" "$resources/extension"
cargo-about generate --frozen --workspace --target "$target" --fail \
    --output-file "$resources/THIRD-PARTY-LICENSES.html" about.hbs
python3 - "$app/Contents/Info.plist" "$version" "$MACOSX_DEPLOYMENT_TARGET" <<'PY'
import plistlib, sys
with open(sys.argv[1], 'wb') as file:
    plistlib.dump({
        'CFBundleName': 'OnesDM', 'CFBundleDisplayName': 'OnesDM',
        'CFBundleIdentifier': 'com.ndmnext.desktop', 'CFBundleExecutable': 'ndm-desktop',
        'CFBundlePackageType': 'APPL', 'CFBundleIconFile': 'ndm-next',
        'CFBundleVersion': sys.argv[2], 'CFBundleShortVersionString': sys.argv[2],
        'LSMinimumSystemVersion': sys.argv[3], 'NSHighResolutionCapable': True,
        'NSPrincipalClass': 'NSApplication',
        'NSDownloadsFolderUsageDescription': 'OnesDM saves your downloads in this folder.',
    }, file)
PY
# Generate all standard icon sizes from the repository's original artwork.
iconset="$scratch/ndm-next.iconset"
mkdir "$iconset"
for size in 16 32 128 256 512; do
    sips -s format png -z "$size" "$size" "$repo/assets/onesdm-icon.png" --out "$iconset/icon_${size}x${size}.png" >/dev/null
    double=$((size * 2))
    sips -s format png -z "$double" "$double" "$repo/assets/onesdm-icon.png" --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$iconset" -o "$resources/ndm-next.icns"
# Ad-hoc signing supports local/CI verification without Apple credentials.
find "$macos/ffmpeg" -type f -name '*.dylib' -exec codesign --force --sign - {} \;
for binary in "$macos/ffmpeg/ffmpeg" "$macos/ffmpeg/ffprobe" "$macos/ndm-host" "$macos/ndm-desktop"; do
    codesign --force --sign - "$binary"
done
codesign --force --sign - "$app"
codesign --verify --deep --strict "$app"
plutil -lint "$app/Contents/Info.plist"
"$macos/ffmpeg/ffmpeg" -hide_banner -version
"$macos/ffmpeg/ffprobe" -hide_banner -version
ln -s /Applications "$scratch/image/Applications"
name="OnesDM-$version-macos-$arch"
[[ ! -e "$out/$name.dmg" && ! -e "$out/OnesDM.app" ]] || { echo 'Output already exists; use a fresh directory' >&2; exit 2; }
hdiutil create -quiet -volname 'OnesDM' -srcfolder "$scratch/image" -format UDZO -ov "$out/$name.dmg"
ditto "$app" "$out/OnesDM.app"
(cd "$out" && shasum -a 256 "$name.dmg" > "$name.dmg.sha256")
echo "Application: $out/OnesDM.app"
echo "Disk image: $out/$name.dmg"
