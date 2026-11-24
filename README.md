<div align="center">
  <h1><img src="assets/readme/hero.webp" width="1200" alt="NDM Next"></h1>
  <p><strong>Fast, multithreaded downloads in a modern native app.</strong></p>
  <p>Built with Rust and GPUI for Windows, macOS, and Linux, with a memory-safe core, pluggable download engines, and optional AI integration.</p>
  <p>Download torrents, magnet links, and streaming media with optional engines from the <a href="#plugins-and-ai">desktop plugin market</a>.</p>
  <p>
    <a href="https://github.com/AreChen/NDM-Next/releases/latest"><img src="https://img.shields.io/github/v/release/AreChen/NDM-Next?label=download&amp;color=7c61ff" alt="Latest release"></a>
    <img src="https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-168aad" alt="Windows, macOS, and Linux">
    <a href="https://github.com/AreChen/NDM-Next/actions/workflows/release.yml"><img src="https://github.com/AreChen/NDM-Next/actions/workflows/release.yml/badge.svg" alt="Desktop release build"></a>
    <a href="LICENSE"><img src="https://img.shields.io/github/license/AreChen/NDM-Next?color=34b8a3" alt="MIT license"></a>
  </p>
  <p><a href="https://github.com/AreChen/NDM-Next/releases/latest"><strong>Download</strong></a> · <a href="#features">Features</a> · <a href="#screenshots">Screenshots</a> · <a href="#browser-extension">Browser extension</a> · <a href="README.zh-CN.md">简体中文</a></p>
</div>

## Features

| Downloads & media | Everyday controls |
| :--- | :--- |
| **Fast transfers** — HTTP(S), FTP(S) and SFTP, with multi-connection HTTP(S) downloads. | **Browser integration** — Capture downloads with file type, size and domain filters. |
| **Media downloads** — HLS and static DASH, plus a searchable resource library and batch downloads. | **Organized tasks** — Multilingual search, category filters, Shift-click range selection, and quick file actions. |
| **Storage & network** — Custom download/cache folders, cache cleanup and per-task proxy controls. | **Native interface** — Smooth motion, compact progress windows, light/dark themes, and English/Chinese support. |

## Download

Get your installer from [**GitHub Releases →**](https://github.com/AreChen/NDM-Next/releases/latest).

| Platform | Package | Install |
| :--- | :--- | :--- |
| Windows · x64 | `NDM-Next-Setup-*-x64.exe` | Run the installer. |
| macOS 12+ · Apple Silicon | `NDM-Next-*-macos-arm64.dmg` | Drag **NDM Next** to **Applications**. |
| macOS 12+ · Intel | `NDM-Next-*-macos-x64.dmg` | Drag **NDM Next** to **Applications**. |
| Linux · x86-64 | `NDM-Next-*-linux-x64.tar.gz` | Install the [Linux dependencies](desktop/packaging/linux/README.md), extract the archive, and run `ndm-desktop`. |

The app checks GitHub for updates. Click **Upgrade** to download a verified package. Windows installs when idle, macOS opens the DMG, and Linux opens the folder containing the archive.

New profiles allow up to 16 concurrent HTTP ranges per download and start at login. The downloader chooses range sizes using the file size and the time the server took to answer the initial probe. Existing settings are preserved.

## Screenshots

| Dark theme | Light theme |
| :---: | :---: |
| [![Download list in dark mode](assets/readme/downloads-dark.webp)](assets/readme/downloads-dark.webp) | [![Download list in light mode](assets/readme/downloads-light.webp)](assets/readme/downloads-light.webp) |

## Plugins and AI

<img src="assets/readme/plugins-ai.webp" width="1200" alt="NDM Next plugin market and AI integration settings">

Install **BitTorrent** for `.torrent` files and magnet links, or **Media Downloader** for streaming video, audio, and playlists from supported sites. Both engines are available in the [desktop plugin market](assets/readme/plugin-market.webp).

Use AI integration to queue and manage downloads from OpenAI Codex (CLI and desktop), Claude Code, Pi, OpenCode, and DeepSeek Harness through MCP and the companion Skill. [Plugin and AI setup guide](docs/plugins-and-ai.md)

## Browser extension

Connect **Chrome, Edge or Brave** to capture downloads and discover webpage media. Optional download buttons appear beside media; the separate media library lets you search, select and download resources in batches.

Each release includes its matching versioned extension ZIP. Follow the [extension installation guide](docs/extension-install.md).

**Platform details:** [Windows](desktop/packaging/windows/README.md#browser-extension) · [macOS](desktop/packaging/macos/README.md#browser-extension) · [Linux](desktop/packaging/linux/README.md#browser-extension). Select **NDM Next** as the extension backend. The extension also supports Neat Download Manager.

<details>
<summary><strong>Build & release</strong></summary>

GitHub Actions builds the Windows installer, both macOS DMGs, and the Linux archive. Pushing a `desktop-vX.Y.Z` tag runs the [release workflow](.github/workflows/release.yml), which publishes the packages and matching companion extension after all builds and tests succeed.

Local build guides: [Windows](desktop/packaging/windows/README.md) · [macOS](desktop/packaging/macos/README.md) · [Linux](desktop/packaging/linux/README.md).

</details>

---

[MIT License](LICENSE) · [Privacy](PRIVACY.md) · Independent project, not affiliated with Neat Download Manager.
