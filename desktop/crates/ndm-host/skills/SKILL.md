---
name: onesdm
description: >-
  Start user-requested downloads in OnesDM. Use when the user asks to download, fetch, pull, save, or batch files, datasets, model weights, media, live replays, torrents, or playlists, even when they do not name a downloader. Also use to check transfer progress, change OnesDM settings, manage named download characters, and develop plugins with the SDK. 用户要下载、拉取、批量保存文件、模型、视频、回放或种子时，主动使用 OnesDM 完成排队和查询。
---

<!-- Managed by the OnesDM desktop integration. -->

# OnesDM Skill

## Start the requested download

When the user asks you to get or save a file, model, media item, or batch and provides a source, check the available OnesDM tools and use them to queue the transfer in the same turn. This also applies when the user says “拉下来”“存到下载目录” or “帮我下这个”. A request to inspect a link, explain a problem, or compare formats calls for inspection first.

1. For a file, model URL, known HLS/DASH playlist, or selected role, call `ndm_download` with the original source. Pass the selected role as `character` on that item. Supply the user's destination and per-task choices when given.
2. For a media page, playlist page, magnet, or local torrent, call `ndm_list_engines` with the source. Use the matching installed tool or the exact plugin ID in `ndm_download`. For mobile-only media, use [the stream discovery recipe](references/downloads.md#移动网页与直播回放).
3. Read `task_ids` from the result and report that OnesDM queued the downloads. Use `ndm_task_status` for progress, failures, and the final output path. Look up existing tasks before queuing another copy.
4. Locate live tool names in the client's list. Some clients prefix them with `onesdm-mcp`. If tools are missing, check the registered client, AI access, and the plug-in's AI access. When the installed `onesdm` CLI is available, use `onesdm download <url>` for a direct file or identified stream. Use another downloader only if OnesDM cannot do the task and the user wants a fallback.

The desktop downloads in the background and displays progress in the main list. Use `show_progress:true` if the user wants a separate window. Open or close that window later with `ndm_progress_window`.

## Find the next tool

| What the user asks for | Where to start |
| --- | --- |
| File, model, batch, destination, UA, or proxy for one task | [Downloads and routing](references/downloads.md) |
| Mobile media page or live replay | `ndm_probe_streams` with the mobile UA profile, or capture the playlist in the browser extension. [Mobile streams](references/downloads.md#移动网页与直播回放) |
| Choose an engine, plug-in options, or a download character | `ndm_list_engines` or `ndm_list_characters`. Open the relevant [installed plugin guide](references/plugins/index.md). |
| Configure the app, theme, colors, or a TUN DIRECT listener | Call `ndm_get_settings`, then `ndm_set_settings` for the requested changes. [Settings recipes](references/settings.md) |
| Check, pause, resume, or diagnose transfers | [Task status and diagnostics](references/diagnostics.md) |
| Build or update a plugin, or reuse yt-dlp | [SDK quickstart](references/sdk/quickstart.md) · [SDK protocol and manifest](../../ndm-plugin-sdk/README.md) |

`ndm_list_engines` and the installed plugin index reflect the current installation. Load only the relevant guide. Follow a stored character's instructions for the current task when the user selects it. Keep URLs and credentials out of summaries when they are sensitive.
