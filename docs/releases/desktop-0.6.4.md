## English

- ModelScope and Hugging Face downloads show the model filename as soon as the task is queued. Other HTTP links can also supply a name in the URL or the server's attachment header. The task list, progress window and MCP pick up the resolved name during the transfer.
- The HTTP engine can download verified ranges in parallel when a model repository supplies a SHA-256 digest, including through rotating CDN links. NDM checks the merged file against that digest before publishing it. The progress window and MCP explain when a transfer uses one connection.
- Plugins can share tools such as yt-dlp. An enhancement plugin can declare a compatible version of the installed Media Downloader and use its executable. Running tasks keep their tool version during a compatible update. The app identifies dependent plugins when removing a provider.
- The NDM Next Skill now includes download recipes, troubleshooting, a plugin development guide, the full SDK reference and a working Rust example. Installed copies refresh after app or plugin updates and reflect each plugin's AI access setting.
- Choose **Upgrade** in NDM Next or download the package for your platform below. The companion browser extension is version 1.2.1.

## 中文

- ModelScope 和 Hugging Face 模型在任务入队时即可显示文件名。其他 HTTP 链接也可以从网址参数或服务器返回的附件名中提取名称，主列表、独立进度窗口和 MCP 会在下载过程中同步更新。
- 模型仓库提供 SHA-256 摘要时，HTTP 引擎可并行下载分段，支持每次请求使用不同签名地址的 CDN。NDM 会在合并后校验整个文件，再保存到下载目录。遇到单连接传输时，可在进度窗口或 MCP 中查看原因。
- 插件可共享 yt-dlp 等工具。增强插件声明兼容的「流媒体下载」插件版本后，即可调用已安装的可执行文件。兼容升级期间，运行中的任务继续使用原工具版本。卸载提供方时，应用会列出依赖它的插件。
- NDM Next Skill 现已内置下载示例、故障排查、插件开发指南、完整 SDK 协议和可编译的 Rust 示例。已安装的 Skill 会随软件和插件更新，并根据插件的 AI 接入开关同步使用指南。
- 在 NDM Next 中选择 **升级**，或从本页下载对应平台的安装包。配套浏览器扩展版本为 1.2.1。
