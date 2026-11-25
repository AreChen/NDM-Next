---
name: ndm-next
description: Use NDM Next to download files or models, media and torrents, manage tasks through its local MCP server, diagnose speed or HTTP failures, or develop, test and package an NDM Next download-engine plugin with its SDK. 适用于下载、排查任务、开发插件和复用 yt-dlp。
---

<!-- Managed by the NDM Next desktop integration. -->

# NDM Next

Use the local `ndm-next` MCP server for downloads. For plugin development, start with the SDK bundled in this Skill. All paths below are relative to this file.

| What the user wants · 使用场景 | Where to start |
| --- | --- |
| Download a direct file or repository model, queue a batch, or set a task's destination, connections, proxy or UA | [Download recipes · 下载与批量任务](references/downloads.md) |
| Choose an engine for a webpage, playlist, magnet or local torrent, or use a plugin option | Call `ndm_list_engines` with the source. Open the matching [installed plugin guide · 已安装插件](references/plugins/index.md). |
| Find, pause or resume a task, explain one active range, or diagnose a failure | [Task status and diagnostics · 状态与排查](references/diagnostics.md) |
| Create a plugin, add a website or post-processor, reuse yt-dlp, or expose an engine to MCP | [SDK quickstart · 插件开发与验收](references/sdk/quickstart.md) · [SDK protocol and manifest](../../ndm-plugin-sdk/README.md) |

## Queue a file

Call `ndm_download` with the original source URL. Supply only the settings the user chose.

```json
{"items":[{"source":"https://example.org/releases/archive.zip"}]}
```

AI downloads are quiet by default and appear in the main task list. To show a separate window on request, set `show_progress` to true. For an existing task call `ndm_progress_window` with `task_id` and `visible` to open or close just that window. For a TUN routing request, first call `ndm_network_routes`, then consult the [network route examples](references/downloads.md#单任务网络设置).

Read `task_ids` from the result. For a status check, call `ndm_task_status` with one returned ID. Report the output path once `state` is `completed`. When a user asks about an existing transfer, look up its ID before creating another task.

## Discover the current installation

The client may prefix tool names. Use the names and input schemas in its live tool list. If the list is empty, check **Settings → AI integration** in NDM Next, enable **AI access**, and verify the client registration. Reopen the client session after installing the integration. Plugin tools also depend on the plugin's own AI access setting.

For a webpage or an engine-specific request, read `automatic_engine`, the matching plugin IDs, and their option schemas. Omit `plugin` for automatic selection. Use the exact ID for explicit selection. The installed plugin index follows local plugin and access changes. The SDK and examples are available offline, including when AI access is off.
