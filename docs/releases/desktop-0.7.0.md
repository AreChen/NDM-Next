## English

Ones Download Manager (OnesDM, ODM) is the AI-native download manager from PlayOnes. Any Agent. All Downloads.

**Upgrading from NDM Next 0.6.8** Download the OnesDM 0.7.0 package for your platform from the release assets. Finish or pause your downloads and quit NDM Next completely. On Windows, run `OnesDM.exe` over the existing installation. On macOS, copy `OnesDM.app` to Applications and open it. On Linux, extract the archive into a new folder and run `ndm-desktop` there. The new version uses your existing profile, including download records, settings and installed plugins. Users on 0.6.9 can upgrade in the app.

- The desktop, native installer and OnesDM Extension use the new silver-ring icon. Windows setup is published as `OnesDM.exe`.
- In Settings → Appearance and language, expand the details to choose Classic violet, Silver ring, Ocean, Forest or Sunset coral, or set two custom hex colors. Palettes work with system, light and dark modes.
- Launch the desktop with `onesdm`, queue HTTP(S) links with `onesdm download <URL>`, or start OnesDM MCP with `onesdm mcp`. Managed integrations use the `onesdm` Skill and `onesdm-mcp` server ID. Existing app data and the browser connection are preserved.
- ODM Core accepts rotating CDN signatures on redirected ranges when the resource selectors, strong ETag and byte range still match.

## 中文

PlayOnes 的 AI 原生高性能下载器更名为 OnesDM，英文全称 Ones Download Manager，简称 ODM。品牌口号为 Any Agent. All Downloads.

**从 NDM Next 0.6.8 升级**　请从本次发布的附件下载对应平台的 OnesDM 0.7.0 安装包。等待下载完成或暂停任务后完全退出旧版。Windows 直接运行 `OnesDM.exe` 覆盖安装。macOS 将 `OnesDM.app` 拖入「应用程序」并打开。Linux 将压缩包解压到新目录，运行其中的 `ndm-desktop`。新版沿用原用户数据，包括下载记录、设置和已安装插件。已装 0.6.9 的用户可在应用内升级。

- 桌面、安装包与 OnesDM Extension 采用银环图标，Windows 安装包名为 `OnesDM.exe`。
- 展开「设置 → 外观与语言」，可选经典紫、银环黑白、海蓝、森林绿、落日珊瑚，也可输入两种十六进制颜色自定义配色。所有配色均可搭配跟随系统、亮色或暗色。
- 使用 `onesdm` 打开桌面，使用 `onesdm download <URL>` 添加 HTTP(S) 链接。OnesDM Skill 的 ID 为 `onesdm`，OnesDM MCP 的 ID 为 `onesdm-mcp`。既有受管配置可迁移，原下载记录和浏览器连接继续使用。
- ODM Core 支持重定向后 CDN 分段签名轮换，继续校验资源、强 ETag、总长度和字节范围。
