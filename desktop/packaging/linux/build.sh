#!/usr/bin/env bash
set -euo pipefail

desktop=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
repo=$(dirname "$desktop")
output=${1:-"$desktop/dist"}
mkdir -p -- "$output"
output=$(cd -- "$output" && pwd)

case $(uname -m) in
    x86_64) arch=x64; target=x86_64-unknown-linux-gnu ;;
    aarch64) arch=arm64; target=aarch64-unknown-linux-gnu ;;
    *) printf '%s\n' "Unsupported Linux architecture" >&2; exit 1 ;;
esac

cd -- "$desktop"
version=$(python3 -c 'import tomllib; print(tomllib.load(open("Cargo.toml", "rb"))["workspace"]["package"]["version"])')
package="NDM-Next-$version-linux-$arch"
scratch=$(mktemp -d)
trap 'rm -rf -- "$scratch"' EXIT
stage="$scratch/$package"
mkdir -p -- "$stage"

cargo +1.100.0 fetch --locked
cargo +1.100.0 build --locked --release --target "$target" -p ndm-desktop -p ndm-host
target_dir=$(cargo +1.100.0 metadata --locked --no-deps --format-version 1 |
    python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')
install -m 755 "$target_dir/$target/release/ndm-desktop" "$target_dir/$target/release/ndm-host" "$stage/"
install -m 644 "$repo/LICENSE" "$repo/PRIVACY.md" "$stage/"
install -m 644 "$desktop/packaging/linux/README.md" "$stage/README.md"
cargo +1.100.0 about generate --frozen --workspace --target "$target" --fail \
    --output-file "$stage/THIRD-PARTY-LICENSES.html" about.hbs

tar -C "$scratch" -czf "$output/$package.tar.gz" "$package"
(cd -- "$output" && sha256sum "$package.tar.gz" > "$package.tar.gz.sha256")
