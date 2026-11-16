## English

- Windows updates now hand control to the installer and exit once its installation guard is confirmed, allowing replacement and relaunch without waiting for the installer to finish inside the old app. Shutdown requests can be retried and no longer depend on the originating window.
- Update packages now use the regular multi-connection HTTP engine, following your connection setting (up to 16). Servers without compatible range support use one connection. GitHub source restrictions, exact size checks and SHA-256 verification remain enforced.
- The title bar shows update download percentage and speed. Settings also shows downloaded bytes, a progress bar, merging and verification stages. Network, storage, verification and installer failures have separate messages and available error codes.
- Verified cached installers are reused after an installation failure. Interrupted update staging files are covered by cache cleanup and protected while an update is active.

**Windows 0.1.8 / 0.1.9:** if an earlier update attempt has already failed while shutting down, quit NDM Next from the tray and run **NDM-Next-Setup-0.1.10-x64.exe** once. A running older app cannot gain the repaired updater until it is replaced. Download history and settings are preserved.

Windows x64, macOS Apple Silicon / Intel packages and the unchanged companion extension **1.1.4** are built and published by GitHub Actions. macOS updates continue to open the verified DMG for replacement in Applications.

## 简体中文

- Windows 更新会在确认安装程序已接管安装锁后退出旧程序，让覆盖安装与重启继续执行，不再在旧程序内部等待安装器退出。退出通知支持重试，退出动作不再依赖发起操作的窗口。
- 更新包改用常规下载任务的多连接 HTTP 引擎，沿用你的连接数设置，最多 16 个连接；服务器不支持兼容的分段下载时使用单连接。仍严格限制 GitHub 来源，并核验文件大小和 SHA-256。
- 标题栏显示更新下载百分比和速度；设置页显示已下载大小、进度条、合并与校验阶段。网络、存储、校验和安装失败会分别提示，并显示可用的错误码。
- 安装失败后重试会复用已通过校验的缓存安装包。中断的更新临时文件纳入缓存清理，更新过程中受到保护。

**Windows 0.1.8 / 0.1.9 用户：**如果之前的更新已在退出旧程序时失败，请先从托盘退出 NDM Next，再运行一次 **NDM-Next-Setup-0.1.10-x64.exe**。正在运行的旧程序需要被替换后才能获得更新器修复。下载记录和设置会保留。

Windows x64、macOS Apple Silicon / Intel 安装包及未变更的配套扩展 **1.1.4** 均由 GitHub Actions 构建和发布。macOS 更新仍通过已校验的 DMG 替换 Applications 中的程序。
