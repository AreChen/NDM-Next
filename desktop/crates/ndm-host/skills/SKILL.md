---
name: ndm-next
description: Queue downloads in the local NDM Next app, check task status, and control batch or global download concurrency via the ndm-next MCP server.
---

<!-- Managed by the NDM Next desktop integration. -->

# NDM Next downloads

Use the local `ndm-next` MCP server when the user wants to download files with
NDM Next. The app's AI integration switch must be enabled.

- Call `ndm_download` with the requested sources. Leave `destination` out to
  use the app's default folder. For an explicit folder, pass its absolute path.
- Installed plugins can expose their own download tools. Discover them through
  MCP and use the advertised options. The generic download tool accepts a
  `plugin` ID when the user selects an engine. The app resolves matching
  plugins automatically when the ID is omitted.
- Use `batch_active_tasks` to limit simultaneous downloads in one request.
  Use `download_threads` for the transfer threads of those tasks. Engine limits
  and the app-wide active-task limit still apply.
- Use `ndm_set_download_limits` only for an explicit application-wide settings
  change. A one-time batch request belongs in `ndm_download`.
- Report the returned task IDs. Use `ndm_task_status` to confirm completion
  and the actual output path. Use the pause and resume tools if requested.
