<div align="center">
  <img src="extension/icons/icon-128.png" width="96" height="96" alt="NDM Next logo">
  <h1>NDM Next</h1>
  <p><strong>High performance · Memory safety · Modern design</strong></p>
  <p>A download manager for Windows and macOS, built with Rust and a polished native GPUI interface.</p>
  <p>
    <a href="https://github.com/AreChen/NDM-Next/releases/latest"><img src="https://img.shields.io/github/v/release/AreChen/NDM-Next?label=download&amp;color=7c61ff" alt="Latest release"></a>
    <img src="https://img.shields.io/badge/platform-Windows%20%7C%20macOS-168aad" alt="Windows and macOS">
    <a href="https://github.com/AreChen/NDM-Next/actions/workflows/release.yml"><img src="https://github.com/AreChen/NDM-Next/actions/workflows/release.yml/badge.svg" alt="Desktop release build"></a>
    <a href="LICENSE"><img src="https://img.shields.io/github/license/AreChen/NDM-Next?color=34b8a3" alt="MIT license"></a>
  </p>
  <p><a href="https://github.com/AreChen/NDM-Next/releases/latest"><strong>Download</strong></a> · <a href="#features">Features</a> · <a href="#browser-extension">Browser extension</a> · <a href="README.zh-CN.md">简体中文</a></p>
</div>

<p align="center">
  <img src="assets/ndm-next-screenshot.png" width="1000" alt="NDM Next desktop app with a compact sidebar and download tasks">
</p>

## Features

| Downloads & media | Everyday controls |
| :--- | :--- |
| **Fast transfers** — HTTP(S), FTP(S) and SFTP, with multi-connection HTTP(S) downloads. | **Browser integration** — Capture downloads with file type, size and domain filters. |
| **Media downloads** — HLS and static DASH, plus a searchable resource library and batch downloads. | **Organized tasks** — Multilingual search, category filters, and quick file and folder actions. |
| **Storage & network** — Custom download/cache folders, cache cleanup and per-task proxy controls. | **Native interface** — Compact progress windows, light/dark themes, and English/Chinese support. |

## Download

Get your installer from [**GitHub Releases →**](https://github.com/AreChen/NDM-Next/releases/latest).

| Platform | Package | Install |
| :--- | :--- | :--- |
| Windows · x64 | `NDM-Next-Setup-*-x64.exe` | Run the installer. |
| macOS 12+ · Apple Silicon | `NDM-Next-*-macos-arm64.dmg` | Drag **NDM Next** to **Applications**. |
| macOS 12+ · Intel | `NDM-Next-*-macos-x64.dmg` | Drag **NDM Next** to **Applications**. |

The app checks GitHub for updates. Click **Upgrade** to download a verified package: Windows installs when idle; macOS opens the DMG for app replacement.

## Browser extension

Connect **Chrome, Edge or Brave** to capture downloads and discover webpage media. Optional download buttons appear beside media; the separate media library lets you search, select and download resources in batches.

**Setup:** [Windows](desktop/packaging/windows/README.md#browser-extension) · [macOS](desktop/packaging/macos/README.md#browser-extension). Select **NDM Next** as the extension backend. The extension also supports Neat Download Manager.

<details>
<summary><strong>Build & release</strong></summary>

GitHub Actions builds the Windows installer and both macOS DMGs. Pushing a `desktop-vX.Y.Z` tag runs the [release workflow](.github/workflows/release.yml), which publishes the packages after the builds succeed.

Local build guides: [Windows](desktop/packaging/windows/README.md) · [macOS](desktop/packaging/macos/README.md).

</details>

---

[MIT License](LICENSE) · [Privacy](PRIVACY.md) · Independent project, not affiliated with Neat Download Manager.
