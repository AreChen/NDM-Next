<div align="center">
  <img src="assets/readme/hero.webp" width="1200" alt="OnesDM，AI 原生下载器。一只游隼俯冲，身后划出多道并行的下载分段。">
  <h1><picture><source media="(prefers-color-scheme: dark)" srcset="assets/onesdm.svg"><img src="assets/onesdm-light.svg" width="36" alt=""></picture> OnesDM</h1>
  <p>Ones Download Manager · PlayOnes · AI-Native Download Manager</p>
  <p><strong>Any Agent. All Downloads.</strong></p>
  <p>使用 Rust 和 GPUI 构建，支持 Windows、macOS 和 Linux，提供内存安全的下载核心、可扩展的下载引擎与可选的 AI 接入。</p>
  <p>通过<a href="#插件与-ai">桌面插件市场</a>安装下载引擎，即可下载种子、磁力链接和流媒体。</p>
  <p>
    <a href="https://github.com/AreChen/OnesDM/releases/latest"><img src="https://img.shields.io/github/v/release/AreChen/OnesDM?label=download&amp;color=7c61ff" alt="最新版本"></a>
    <img src="https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-168aad" alt="支持 Windows、macOS 和 Linux">
    <a href="https://github.com/AreChen/OnesDM/actions/workflows/release.yml"><img src="https://github.com/AreChen/OnesDM/actions/workflows/release.yml/badge.svg" alt="桌面端发布构建"></a>
    <a href="LICENSE"><img src="https://img.shields.io/github/license/AreChen/OnesDM?color=34b8a3" alt="MIT 许可证"></a>
  </p>
  <p><a href="https://github.com/AreChen/OnesDM/releases/latest"><strong>下载应用</strong></a> · <a href="#功能">功能</a> · <a href="#界面截图">界面截图</a> · <a href="#浏览器扩展">浏览器扩展</a> · <a href="https://odm.playones.com/zh/">官网</a></p>
</div>

<p align="center"><a href="README.md">English</a> · <strong>简体中文</strong> · <a href="README.zh-Hant.md">繁體中文</a> · <a href="README.ja.md">日本語</a> · <a href="README.ko.md">한국어</a> · <a href="README.es.md">Español</a> · <a href="README.pt-BR.md">Português (Brasil)</a> · <a href="README.fr.md">Français</a> · <a href="README.de.md">Deutsch</a></p>

## 功能

| 下载与资源 | 操作与体验 |
| :--- | :--- |
| **多协议下载** — 支持 HTTP(S)、FTP(S)、SFTP，HTTP(S) 支持多连接下载。 | **浏览器接管** — 按文件类型、大小及域名过滤下载，减少无用文件干扰。 |
| **媒体下载** — 支持 HLS、静态 DASH，提供可搜索的媒体资源窗口和批量下载。 | **任务管理** — 多语言搜索、分类筛选、Shift 连续多选，以及快捷打开文件。 |
| **存储与网络** — 自定义下载及缓存目录、清理缓存，为单个任务启用代理。 | **原生界面** — 平滑动效、紧凑进度窗口、亮暗主题、九种界面语言。 |

## 界面截图

### 亮色主题

[![亮色界面下的 OnesDM 任务列表，显示四个已完成的下载。](assets/readme/downloads-light.webp)](assets/readme/downloads-light.png)

### 暗色主题

[![暗色界面下的 OnesDM 任务列表，显示四个已完成的下载。](assets/readme/downloads-dark.webp)](assets/readme/downloads-dark.png)

## 下载

前往 [**GitHub Releases 获取安装包 →**](https://github.com/AreChen/OnesDM/releases/latest)。

| 平台 | 安装包 | 安装方式 |
| :--- | :--- | :--- |
| Windows · x64 | `OnesDM.exe` | 运行安装程序。 |
| macOS 12+ · Apple Silicon | `OnesDM-*-macos-arm64.dmg` | 将 **OnesDM** 拖入 **Applications**。 |
| macOS 12+ · Intel | `OnesDM-*-macos-x64.dmg` | 将 **OnesDM** 拖入 **Applications**。 |
| Linux · x86-64 | `OnesDM-*-linux-x64.tar.gz` | 安装 [Linux 运行依赖](desktop/packaging/linux/README.md)，解压并运行 `ndm-desktop`。 |

**从 0.6.8、0.6.9 或 0.7.0 升级**　这些版本的更新器会跳过当前 GitHub 地址下的安装包。请先暂停或完成下载，完全退出旧版，再按上表手动安装一次 0.7.1 或更新版本。保留原用户数据即可继续使用下载记录、设置和插件。从 0.7.1 起，即使仓库名再次调整大小写，应用也能识别后续更新。[下载 OnesDM](https://github.com/AreChen/OnesDM/releases/latest)

应用通过 GitHub 检查更新。点击**升级**下载并校验安装包。Windows 在空闲时安装，macOS 替换应用，必要时打开 DMG 供手动安装，Linux 打开压缩包所在目录。

新安装默认允许每个 HTTP 下载最多并发请求 16 个分段，并开启开机自启。下载器根据文件大小和服务器首次响应的耗时选择分段大小。已有设置会保留。

在「设置 → 外观与语言」可切换界面语言。新安装时会从系统首选语言中选择受支持的一种。

## 命令行

安装包内含 `onesdm`。可将其目录加入 PATH，或用完整路径调用。macOS 的命令位于 `OnesDM.app/Contents/MacOS`。使用 `onesdm download <URL>` 将 HTTP(S) 链接加入桌面队列，`onesdm mcp` 启动 stdio 服务，`onesdm --version` 查看版本。

## 插件与 AI

<img src="assets/readme/plugins-ai.webp" width="1200" alt="胶片、音频波形、对等网络和终端提示符汇入同一束下载轨迹，落在一叠文件上。">

在[桌面插件市场](https://github.com/AreChen/onesdm-plugin-market)安装 **BitTorrent**，即可下载 `.torrent` 文件和磁力链接。安装 **Media Downloader**，即可下载受支持网站的流媒体视频、音频和播放列表。

开启 AI 接入后，可通过 MCP 和配套 Skill 在 OpenAI Codex（CLI 与桌面版）、Claude Code、Pi、OpenCode 和 DeepSeek Harness 中创建和管理下载任务。[插件与 AI 接入指南](docs/plugins-and-ai.md)

## 浏览器扩展

连接 **Chrome、Edge 或 Brave**，接管下载并嗅探网页媒体。可选开启媒体旁的下载按钮，也可打开独立媒体资源窗口，搜索、选择并批量下载。

每次 Release 均附带对应版本的扩展 ZIP。按照[扩展安装说明](docs/extension-install.md)连接浏览器。

**平台说明** [Windows](desktop/packaging/windows/README.md#browser-extension) · [macOS](desktop/packaging/macos/README.md#browser-extension) · [Linux](desktop/packaging/linux/README.md#browser-extension)。扩展后端请选择 **OnesDM**。扩展也支持连接 Neat Download Manager。

<details>
<summary><strong>构建与发布</strong></summary>

GitHub Actions 构建 Windows 安装包、macOS 双架构 DMG 和 Linux 压缩包。推送 `desktop-vX.Y.Z` 标签后，[发布工作流](.github/workflows/release.yml)会在各平台构建和测试成功后发布安装包及对应扩展。

本地构建说明 [Windows](desktop/packaging/windows/README.md) · [macOS](desktop/packaging/macos/README.md) · [Linux](desktop/packaging/linux/README.md)。

</details>

---

[MIT 许可证](LICENSE) · [隐私说明](PRIVACY.md) · 独立项目，与 Neat Download Manager 官方无关联。
