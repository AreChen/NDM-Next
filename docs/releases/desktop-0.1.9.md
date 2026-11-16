## English

- New profiles use **16 download connections** and **start at login** by default. Existing saved connection counts and startup choices are preserved.
- Fixed macOS Chrome/Chromium/Brave/Edge integration for unpacked companion extensions. Release extension **1.1.4** has a fixed, pre-authorized ID; Settings now offers **Browser extension → Repair connection** for older developer IDs. Existing custom registrations survive app restarts and upgrades.
- Every desktop release now builds Windows x64, macOS Apple Silicon and Intel packages **on GitHub Actions**, and packages the matching extension from the same release commit. The release includes the versioned extension ZIP, its installation guide and Windows helper, plus SHA-256 checksums.
- Fixed a Windows installer shutdown race: native hosts already terminating are waited on without repeating image queries or termination requests.

Install the desktop package for your platform. For manual extension installation, extract **NDM-Next-Extension-Desktop-0.1.9-v1.1.4.zip** into a permanent folder, then load that folder in `chrome://extensions` or `edge://extensions` with Developer mode enabled. Choose **NDM Next** as the download manager in extension settings. When replacing an older extension without a fixed ID, remove the old copy, load the new one and check your preferences. See [installation instructions](https://github.com/AreChen/NDM-Next/blob/desktop-v0.1.9/docs/extension-install.md).

The desktop updater does not update unpacked browser extensions. macOS packages are ad-hoc signed and are not notarized by Apple; Windows packages are unsigned.

## 简体中文

- 新安装默认使用 **16 个下载连接**，默认开启**开机自启**。已有用户保存的连接数和自启选择会保留。
- 修复 macOS Chrome/Chromium/Brave/Edge 对解压加载扩展的接管连接。配套扩展 **1.1.4** 使用固定 ID，桌面程序已预先授权；设置新增**浏览器扩展 → 修复连接**，可授权旧版开发者扩展 ID。重启及升级会保留有效的自定义授权。
- 每次桌面发版均由 **GitHub Actions** 构建 Windows x64、macOS Apple Silicon 和 Intel 安装包，并从同一发版提交打包对应扩展。Release 包含带版本号的扩展 ZIP，内含安装说明和 Windows 授权工具，同时提供 SHA-256 校验文件。
- 修复 Windows 更新前关闭 Native Host 的时序问题，避免进程退出过程中重复终止导致安装中断。

安装对应平台的桌面程序。手动安装扩展时，将 **NDM-Next-Extension-Desktop-0.1.9-v1.1.4.zip** 解压到固定目录，在 `chrome://extensions` 或 `edge://extensions` 开启开发者模式并加载该目录。扩展设置中的下载管理器选择 **NDM Next**。首次从未固定 ID 的旧扩展升级，请移除旧副本，再加载新版并检查设置。参见[安装说明](https://github.com/AreChen/NDM-Next/blob/desktop-v0.1.9/docs/extension-install.md)。

桌面自动更新不会替你更新解压加载的扩展。macOS 安装包使用临时签名，尚未经过 Apple 公证；Windows 安装包未签名。
