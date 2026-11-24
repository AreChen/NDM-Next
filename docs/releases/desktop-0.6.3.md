## English

- Built-in HTTP(S), HLS and DASH downloads now use the Chrome 142 for Windows User-Agent by default. A saved custom value, a User-Agent captured from the browser, or an MCP task override still takes precedence. Clear the field in **Settings → Download behavior → More settings** to restore Chrome.
- HTTP range downloads choose part sizes using file size and the initial response time. Free connections pick up pending parts, and changing the connection limit preserves verified partial downloads. Truncated ranges resume from the saved offset. When a server rejects parallel requests with HTTP 429, outstanding work can finish on one connection.
- The desktop app and MCP calculate recent download speed from bytes received over a four-second window. The progress window labels local merge and save speeds separately.
- DeepSeek Harness can now install the NDM Next MCP tools and Skill under **Settings → AI integration**. Installed Skills pick up changes to plugin guides automatically.
- Choose **Upgrade** in NDM Next or download the package for your platform from this release. The companion browser extension is version 1.2.1.

## 中文

- 内置 HTTP(S)、HLS 和 DASH 下载现在默认使用 Windows 版 Chrome 142 的 User-Agent。已保存的自定义 UA 继续生效，浏览器接管时捕获的 UA 和 MCP 单任务指定的 UA 优先。要恢复 Chrome 默认值，在 **设置 → 下载行为 → 更多设置** 中清空 UA 并保存即可。
- HTTP 分段下载会根据文件大小和首次响应耗时选择分段大小。空闲连接会继续领取待下载分段，调整连接数后仍可沿用已验证的断点。分段响应提前结束时从已保存的偏移续传，遇到限制并发的 HTTP 429 时可降为串行完成。
- 桌面端和 MCP 根据最近四秒实际接收的字节计算下载速度。独立进度窗口会单独标明本地合并、保存阶段的速度。
- 在 **设置 → AI 集成** 中可为 DeepSeek Harness 安装 NDM Next 的 MCP 工具和 Skill。插件指南有变动时，已安装的 Skill 会自动同步。
- 在 NDM Next 中点击 **升级**，或从本发布页下载对应平台的安装包。配套浏览器扩展版本为 1.2.1。
