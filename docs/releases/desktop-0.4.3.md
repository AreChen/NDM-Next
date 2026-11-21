## English

- Segmented controls now use a single sliding selection. The extra hover highlight and duplicated labels are gone from theme and plugin controls.
- Plugin market shows downloaded bytes and extraction progress during install and update, with a separate verification stage. Importing a local ZIP also shows extraction progress.
- In Plugin market, update **YouTube** to **0.1.3**. Downloads prefer the source container by default. You can choose MP4, WebM or MKV and prefer an available H.265 or H.264 source track. Subtitle languages can be selected together, and the separate proxy settings include a protocol menu, address validation and a hover hint. The FFmpeg bundled with Windows and macOS supports WebM output.
- Update **BitTorrent** to **0.1.1**. Choose individual files from a `.torrent` file or magnet link. The task list and separate progress window show peer and upload statistics, including the share ratio. Start or stop seeding a completed task from either view. Plugin settings include peer discovery, port mapping, connection limits and transfer speed limits.

Run the Windows x64 installer or drag the macOS app from the matching Apple silicon or Intel DMG into Applications. On Linux x64, install the runtime dependencies, extract the archive and run `./ndm-desktop`. The companion browser extension is version 1.1.4. Verify the downloads with `SHA256SUMS`. [Linux installation guide](https://github.com/AreChen/NDM-Next/blob/desktop-v0.4.3/desktop/packaging/linux/README.md)

## 中文

- 主题和插件的滑动分段控件统一使用一个选中滑块，去掉了悬停时叠加的底色和重复文字。
- 插件市场安装和更新时会显示下载字节数、校验状态和解包进度。导入本地 ZIP 时也会显示解包进度。
- 在插件市场将 **YouTube** 更新至 **0.1.3**。下载默认优先保留源站封装，也可手动选择 MP4、WebM 或 MKV，并优先选用源站已有的 H.265 或 H.264 轨道。字幕语言可多选，独立代理提供协议选择、地址校验和悬停说明。Windows 和 macOS 随包 FFmpeg 已支持 WebM 输出。
- 将 **BitTorrent** 更新至 **0.1.1**。添加 `.torrent` 文件或磁力链接后可勾选要下载的文件。主列表和独立进度窗口显示连接和上传数据，包括分享率，两处均可开始或停止做种。插件设置可调节节点发现、端口映射、连接上限和上传下载速度。

Windows x64 直接运行安装程序。macOS 选择适用于 Apple 芯片或 Intel 芯片的 DMG，将应用拖入「应用程序」。Linux x64 安装运行依赖后解压，运行 `./ndm-desktop`。配套浏览器扩展为 1.1.4，下载校验值见 `SHA256SUMS`。[Linux 安装指南](https://github.com/AreChen/NDM-Next/blob/desktop-v0.4.3/desktop/packaging/linux/README.md)
