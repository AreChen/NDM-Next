## English

- Choose a compatible download engine when adding a media URL. Plugins can offer a manual choice for additional sites, while direct media files and HLS or DASH streams use the built-in engine by default.
- The desktop app reports its version and installed plugin versions to the browser extension. With extension 1.2.0 and a compatible media plugin, you can send the current webpage to that plugin from the extension.
- The plugin SDK lets engines declare manual source matching and opt into browser page downloads.

Run the Windows x64 installer or drag the app from the matching Apple silicon or Intel DMG into Applications. On Linux x64, install the runtime dependencies, extract the archive and run `./ndm-desktop`. [Linux installation guide](https://github.com/AreChen/NDM-Next/blob/main/desktop/packaging/linux/README.md)

## 中文

- 添加媒体链接时可以选择兼容的下载引擎。插件可以为更多站点提供手动选项，媒体直链和 HLS、DASH 流默认使用内置引擎。
- 桌面端向浏览器扩展提供自身版本和已安装插件的版本。配合扩展 1.2.0 和兼容的流媒体插件，可从扩展将当前网页交给该插件解析。
- 插件 SDK 支持声明可手动选择的链接范围，以及是否接受浏览器发起的网页下载。

Windows x64 直接运行安装程序。macOS 请打开对应 Apple 芯片或 Intel 芯片的 DMG，将应用拖入「应用程序」。Linux x64 安装运行依赖后解压，运行 `./ndm-desktop`。[Linux 安装指南](https://github.com/AreChen/NDM-Next/blob/main/desktop/packaging/linux/README.md)
