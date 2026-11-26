<div align="center">
  <img src="assets/readme/hero.webp" width="1200" alt="OnesDM，AI 原生下載管理員。一隻遊隼俯衝，身後劃出多道平行的下載分段。">
  <h1><picture><source media="(prefers-color-scheme: dark)" srcset="assets/onesdm.svg"><img src="assets/onesdm-light.svg" width="36" alt=""></picture> OnesDM</h1>
  <p>Ones Download Manager · PlayOnes · AI-Native Download Manager · <a href="https://odm.playones.com/zh-hant/">odm.playones.com</a></p>
  <p><strong>Any Agent. All Downloads.</strong></p>
  <p>使用 Rust 和 GPUI 構建，支援 Windows、macOS 和 Linux，提供記憶體安全的下載核心、可擴充套件的下載引擎與可選的 AI 接入。</p>
  <p>透過<a href="#外掛與-ai">桌面外掛市場</a>安裝下載引擎，即可下載種子、磁力連結和流媒體。</p>
  <p>
    <a href="https://github.com/AreChen/OnesDM/releases/latest"><img src="https://img.shields.io/github/v/release/AreChen/OnesDM?label=download&amp;color=7c61ff" alt="最新版本"></a>
    <img src="https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-168aad" alt="支援 Windows、macOS 和 Linux">
    <a href="https://github.com/AreChen/OnesDM/actions/workflows/release.yml"><img src="https://github.com/AreChen/OnesDM/actions/workflows/release.yml/badge.svg" alt="桌面端釋出構建"></a>
    <a href="LICENSE"><img src="https://img.shields.io/github/license/AreChen/OnesDM?color=34b8a3" alt="MIT 許可證"></a>
  </p>
  <p><a href="https://github.com/AreChen/OnesDM/releases/latest"><strong>下載應用</strong></a> · <a href="#功能">功能</a> · <a href="#介面截圖">介面截圖</a> · <a href="#瀏覽器擴充套件">瀏覽器擴充套件</a></p>
</div>

<p align="center"><a href="README.md">English</a> · <a href="README.zh-CN.md">简体中文</a> · <strong>繁體中文</strong> · <a href="README.ja.md">日本語</a> · <a href="README.ko.md">한국어</a> · <a href="README.es.md">Español</a> · <a href="README.pt-BR.md">Português (Brasil)</a> · <a href="README.fr.md">Français</a> · <a href="README.de.md">Deutsch</a></p>

## 功能

| 下載與資源 | 操作與體驗 |
| :--- | :--- |
| **多協議下載** — 支援 HTTP(S)、FTP(S)、SFTP，HTTP(S) 支援多連線下載。 | **瀏覽器接管** — 按檔案型別、大小及域名過濾下載，減少無用檔案干擾。 |
| **媒體下載** — 支援 HLS、靜態 DASH，提供可搜尋的媒體資源視窗和批次下載。 | **任務管理** — 多語言搜尋、分類篩選、Shift 連續多選，以及快捷開啟檔案。 |
| **儲存與網路** — 自定義下載及快取目錄、清理快取，為單個任務啟用代理。 | **原生介面** — 平滑動效、緊湊進度視窗、亮暗主題、九種介面語言。 |

## 介面截圖

<table>
  <thead>
    <tr>
      <th scope="col" width="50%">淺色主題 · 設定中心</th>
      <th scope="col" width="50%">深色主題 · 下載清單</th>
    </tr>
  </thead>
  <tbody>
    <tr>
      <td><a href="assets/readme/downloads-light.png"><img src="assets/readme/downloads-light.webp" alt="OnesDM 的淺色設定中心，顯示 AI 整合、語言、下載執行緒和代理伺服器設定。"></a></td>
      <td><a href="assets/readme/downloads-dark.png"><img src="assets/readme/downloads-dark.webp" alt="深色介面下的 OnesDM 任務清單，顯示四筆已完成的下載。"></a></td>
    </tr>
  </tbody>
</table>

## 下載

前往 [**GitHub Releases 獲取安裝包 →**](https://github.com/AreChen/OnesDM/releases/latest)。

| 平臺 | 安裝包 | 安裝方式 |
| :--- | :--- | :--- |
| Windows · x64 | `OnesDM.exe` | 執行安裝程式。 |
| macOS 12+ · Apple Silicon | `OnesDM-*-macos-arm64.dmg` | 將 **OnesDM** 拖入 **Applications**。 |
| macOS 12+ · Intel | `OnesDM-*-macos-x64.dmg` | 將 **OnesDM** 拖入 **Applications**。 |
| Linux · x86-64 | `OnesDM-*-linux-x64.tar.gz` | 安裝 [Linux 執行依賴](desktop/packaging/linux/README.md)，解壓並執行 `ndm-desktop`。 |

**從 0.6.8、0.6.9 或 0.7.0 升級**　這些版本的更新器會跳過當前 GitHub 地址下的安裝包。請先暫停或完成下載，完全退出舊版，再按上表手動安裝一次 0.7.1 或更新版本。保留原使用者資料即可繼續使用下載記錄、設定和外掛。從 0.7.1 起，即使倉庫名再次調整大小寫，應用也能識別後續更新。[下載 OnesDM](https://github.com/AreChen/OnesDM/releases/latest)

應用透過 GitHub 檢查更新。點選**升級**下載並校驗安裝包。Windows 在空閒時安裝，macOS 替換應用程式，必要時開啟 DMG 供手動安裝，Linux 開啟壓縮包所在目錄。

新安裝預設允許每個 HTTP 下載最多併發請求 16 個分段，並開啟開機自啟。下載器根據檔案大小和伺服器首次響應的耗時選擇分段大小。已有設定會保留。

在「設定 → 外觀與語言」可切換介面語言。新安裝時會從系統首選語言中選擇受支援的一種。

## 命令列

安裝包內含 `onesdm`。可將其目錄加入 PATH，或用完整路徑呼叫。macOS 的命令位於 `OnesDM.app/Contents/MacOS`。使用 `onesdm download <URL>` 將 HTTP(S) 連結加入桌面佇列，`onesdm mcp` 啟動 stdio 服務，`onesdm --version` 檢視版本。

## 外掛與 AI

<img src="assets/readme/plugins-ai.webp" width="1200" alt="底片、音訊波形、點對點網路和終端機提示符號匯入同一束下載軌跡，落在一疊檔案上。">

在[桌面外掛市場](https://github.com/AreChen/onesdm-plugin-market)安裝 **BitTorrent**，即可下載 `.torrent` 檔案和磁力連結。安裝 **Media Downloader**，即可下載受支援網站的流媒體影片、音訊和播放列表。

開啟 AI 接入後，可透過 MCP 和配套 Skill 在 OpenAI Codex（CLI 與桌面版）、Claude Code、Pi、OpenCode 和 DeepSeek Harness 中建立和管理下載任務。[外掛與 AI 接入指南](docs/plugins-and-ai.md)

## 瀏覽器擴充套件

連線 **Chrome、Edge 或 Brave**，接管下載並嗅探網頁媒體。可選開啟媒體旁的下載按鈕，也可開啟獨立媒體資源視窗，搜尋、選擇並批次下載。

每次 Release 均附帶對應版本的擴充套件 ZIP。按照[擴充套件安裝說明](docs/extension-install.md)連線瀏覽器。

**平臺說明** [Windows](desktop/packaging/windows/README.md#browser-extension) · [macOS](desktop/packaging/macos/README.md#browser-extension) · [Linux](desktop/packaging/linux/README.md#browser-extension)。擴充套件後端請選擇 **OnesDM**。擴充套件也支援連線 Neat Download Manager。

<details>
<summary><strong>構建與釋出</strong></summary>

GitHub Actions 構建 Windows 安裝包、macOS 雙架構 DMG 和 Linux 壓縮包。推送 `desktop-vX.Y.Z` 標籤後，[釋出工作流](.github/workflows/release.yml)會在各平臺構建和測試成功後釋出安裝包及對應擴充套件。

本地構建說明 [Windows](desktop/packaging/windows/README.md) · [macOS](desktop/packaging/macos/README.md) · [Linux](desktop/packaging/linux/README.md)。

</details>

---

[MIT 許可證](LICENSE) · [隱私說明](PRIVACY.md) · 獨立專案，與 Neat Download Manager 官方無關聯。
