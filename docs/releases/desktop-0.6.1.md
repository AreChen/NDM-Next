## English

- Set a custom User-Agent in download settings or start from a Chrome, Firefox, or Safari preset.
- MCP built-in HTTP(S) items support a per-task User-Agent and direct connections. Status includes redacted failure details and recent activity. Built-in file downloads report the HTTP status, failure stage, and effective proxy route when available.
- The HTTP engine retries with a plain GET when a server rejects Range with a supported fallback status. It verifies the final length against the available size information before publishing the file.
- **Download again** restarts a running or queued download from the beginning using the same task ID.
- Includes companion extension 1.2.1. [Plugin and AI guide](https://github.com/AreChen/NDM-Next/blob/desktop-v0.6.1/docs/plugins-and-ai.md)

## 中文

- 下载设置支持自定义 User-Agent，并提供 Chrome、Firefox 和 Safari 预设。
- MCP 内置 HTTP(S) 任务支持单项直连和 User-Agent。状态接口返回脱敏的失败详情与近期活动。内置文件下载还会在有记录时返回 HTTP 状态码、失败阶段和实际代理路由。
- 遇到特定的 Range 拒绝响应时，HTTP 引擎会尝试普通 GET，并在保存前按已知大小校验完整性。
- 对正在运行或排队的任务点击 **再次下载**，会使用同一个任务 ID 从头下载。
- 附带配套扩展 1.2.1。[插件与 AI 接入说明](https://github.com/AreChen/NDM-Next/blob/desktop-v0.6.1/docs/plugins-and-ai.md)
