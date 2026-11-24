---
name: ndm-next
description: Use the local NDM Next MCP server to download files, media, playlists, and torrents, choose installed engines, track and control tasks, or manage download concurrency.
---

<!-- Managed by the NDM Next desktop integration. -->

# NDM Next downloads

## Choose an engine

- Use the `ndm-next` MCP server. Check that your client is configured under
  **Settings → AI integration** and that **AI access** is on in NDM Next.
  Restart the client session after installing or updating the registration.
  Clients can prefix these tool names with the server name. Use the names
  shown in the client's tool list, such as `mcp__ndm_next__ndm_download` in Pi.
- Direct HTTP(S), FTP(S), SFTP, HLS and static DASH downloads use the built-in
  engine when there is no automatic plugin match. Pass URLs as supplied or use
  absolute paths for local inputs such as torrent files.
- For a webpage, magnet link, torrent, or a request for engine-specific options,
  call `ndm_list_engines` with the source. Its `automatic_engine` reports the
  default route. An engine with `automatic_match: false` requires explicit
  selection. See the [installed plugin guides](references/plugins/index.md) and
  read the matching guide for examples and source-specific options. The live MCP
  tool schemas describe what the current installation accepts.
- For one download, use the plugin's tool when it fits the request. For a batch
  or a plugin without a dedicated tool, use `ndm_download`. Specify `plugin`
  with the exact engine ID when the user chooses one. Omit `plugin` to use
  automatic routing and omit unspecified options to use the app's defaults.

## Download and report

This `ndm_download` request queues a direct file.

```json
{"items":[{"source":"https://example.com/archive.zip"}]}
```

- Omit `destination` to use the app's default. An explicit destination must be
  an absolute directory. An item-specific destination overrides the batch folder.
- `batch_active_tasks` limits simultaneous downloads within the request.
  `download_threads` sets the thread count for the batch or an individual item.
  The app-wide limit and engine limits still apply. Use
  `ndm_get_download_limits` to inspect global defaults and
  `ndm_set_download_limits` for an explicit application-wide change.
- For a built-in HTTP(S) item, omit `proxy_mode` to inherit the app's route.
  Use `"proxy_mode":"direct"` on that item when the user asks to bypass both
  the app and system proxies. Use `user_agent` on the item when the user
  specifies a User-Agent for the request.
- Report the returned task IDs and say the tasks were queued. To confirm
  completion, use `ndm_task_status` with a returned ID. A completed task's
  `output` is the path to report. Summarize any failure with its task ID.
- For a failed built-in file download, report the available `error.code`,
  `error.http_status`, `error.stage`, and `proxy.effective`. Check
  `progress.http` for the Range probe status and redirect count. A null field
  means the detail is unavailable. Use `progress.activity` and its sample time
  to describe slow downloads, and compare samples before calling a task stalled.
- For a single-connection HTTP task, read `progress.http.range_mode` before
  changing thread settings. It distinguishes a task limit, server rate limiting,
  an ignored or rejected Range request, and a missing file validator.
- Use `ndm_list_tasks` when you need to find a task or look at recent activity.
  Filter by `state` when appropriate. Pass `next_cursor` back as `cursor` with
  the same filter to read the next page. A null cursor means the list ended.
- Use `ndm_pause_task` and `ndm_resume_task` for requested task actions and
  check the resulting state with `ndm_task_status` when confirmation matters.
