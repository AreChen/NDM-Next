# Linux build and installation

The release archive targets Linux x86-64 with glibc 2.39 or newer. It is built
on Ubuntu 24.04 and supports Wayland and X11. Install these runtime packages on
Ubuntu or Debian, then extract the archive and run `./ndm-desktop` in the
extracted directory.

```sh
sudo apt-get install ffmpeg xdg-utils libglib2.0-bin libfontconfig1 libfreetype6 \
  libssl3t64 libxkbcommon0 libxkbcommon-x11-0 libx11-xcb1 libxcb1 \
  libwayland-client0 libwayland-cursor0 libvulkan1 mesa-vulkan-drivers
```

Downloads follow the XDG Downloads directory, with `~/Downloads` as the
fallback. Persistent state is in `${XDG_DATA_HOME:-~/.local/share}/ndm-next`
and caches are in `${XDG_CACHE_HOME:-~/.cache}/ndm-next`. The desktop app and
browser host share a randomly generated AES key with owner-only permissions.
The Settings toggle registers minimized startup through XDG Autostart.
Minimize the main window to keep downloading. Closing it checkpoints running
tasks and quits the app.
HLS and DASH use the distribution's `/usr/bin/ffmpeg` and `ffprobe` for
local media processing.

The app checks GitHub for Linux updates. **Upgrade** verifies the archive and
opens its containing folder. Quit the running app, extract the new archive
and run its `ndm-desktop`. The updated app uses the same XDG data directories.

## Browser extension

Start the desktop app once to register `ndm-host` for native installations
of Chrome, Chromium, Brave, and Microsoft Edge. Use the matching release
extension ZIP and select **OnesDM** as its backend. The **Repair connection**
action also accepts a custom unpacked extension's ID. Native host paths are
updated when you launch the app from a new directory. Browser packages with
separate sandbox profiles can require their own native messaging setup.

## Linux development

Use a Linux build environment with the runtime packages above and the
following build dependencies.

```sh
sudo apt-get install build-essential cmake pkg-config libssl-dev libfontconfig1-dev \
  libfreetype-dev libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev \
  libx11-xcb-dev libxcb1-dev libxcb-render0-dev libxcb-shape0-dev \
  libxcb-xfixes0-dev libxcb-xkb-dev xvfb xauth
rustup toolchain install 1.100.0 --profile minimal --component rustfmt,clippy
rustup target add --toolchain 1.100.0 x86_64-unknown-linux-gnu
cargo +1.100.0 install cargo-about --version 0.9.3 --features cli --locked
```

Run the workspace suite with an isolated XDG profile. Keep the scratch
directory until the tests and any manually launched processes exit.

```sh
ndm_profile=$(mktemp -d)
export XDG_CONFIG_HOME="$ndm_profile/config" XDG_DATA_HOME="$ndm_profile/data"
export XDG_CACHE_HOME="$ndm_profile/cache" XDG_RUNTIME_DIR="$ndm_profile/runtime"
install -d -m 700 "$XDG_RUNTIME_DIR"
cd desktop
xvfb-run -a cargo +1.100.0 test --locked --workspace
cargo +1.100.0 clippy --locked --workspace --all-targets -- -D warnings
bash packaging/linux/build.sh
```

The GitHub Actions Linux workflow runs the native workspace suite and builds
the downloadable archive. Local archives are for development and verification.
