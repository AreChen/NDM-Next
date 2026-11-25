# Plugins and AI clients

Open **Plugin market** to browse, install, update, or uninstall plugins.
Search by name or keyword in English or Chinese, and narrow the cards by
category or **Installed only**. The display language follows **Settings → Language**.
The market validates the downloaded package against its published SHA-256
checksum. **Import plugin** also accepts a compatible local ZIP from a
developer you trust. An engine package contains a native executable.
The market shows downloaded bytes and the verification and installation stages.
An enhancement plugin can use the yt-dlp executable supplied by an installed
Media Downloader plugin. If its required provider or compatible version is
missing, the installer names it. Uninstall dependent plugins before their provider.

Open **Settings** on an installed plugin card to save default download options.
They apply to new tasks, including downloads requested by AI clients.
An option chosen for an individual task takes precedence.
Saved defaults use the same local protection as download task options.
Media Downloader can preserve the native container or copy video tracks to
MP4, WebM, or MKV. Choose uploaded or automatic subtitles and check the
languages you want. For a separate HTTP(S) or SOCKS proxy, enable **Use a
separate proxy**, choose a protocol, and enter the host and port.

Paste a URL or select a local source file in **New download**. NDM Next
selects an installed engine that matches the source. Media Downloader
matches pages on common media sites such as YouTube, Bilibili, TikTok and
Vimeo. For another site, choose **Media Downloader** in **Download engine**.
You can also select it for direct HLS and DASH streams. The built-in engine
is the default for those streams and other direct media files. A choice
between automatic matches can be saved for that site.

For a torrent or magnet link, use **Choose files** to check the files you want.
The task list shows connected peers and the share ratio. Open download details
for the connected seeders, discovered peers, and upload statistics. Use
**Start seeding** and **Stop seeding** in either window after download completes.

Open **Settings → AI integration** and expand the client list. Install MCP
and the Skill for each client you use, then turn on **AI access** to make
download tools available. AI access starts disabled. Reopen the client
session after installation. Each row shows the local configuration state
and offers repair when the NDM Next executable has moved.

| Client | User MCP configuration | User Skill location |
| --- | --- | --- |
| OpenAI Codex CLI and desktop | `~/.codex/config.toml` | `~/.agents/skills/ndm-next/` |
| Claude Code | `~/.claude.json` | `~/.claude/skills/ndm-next/` |
| Pi with built-in MCP | `~/.pi/agent/mcp.json` | `~/.pi/agent/skills/ndm-next/` |
| OpenCode | `~/.config/opencode/opencode.json(c)` | `~/.config/opencode/skills/ndm-next/` |
| DeepSeek Harness | `~/.dsh/cordis.patch.yml` | `~/.dsh/skills/ndm-next/` |

Codex CLI and the desktop app share MCP settings on the same Codex host.
NDM Next can write this configuration even on a desktop-only installation.
Pi's built-in MCP is available from version 0.99.0. NDM Next works with both
OpenCode's legacy MCP map and the V2 `mcp.servers` structure. These are the
default locations. Absolute configuration paths inherited by NDM Next from
the client's environment can override them.

DeepSeek Harness loads NDM Next through its built-in stdio MCP client. Its
home-level Cordis patch applies across profiles. Profiles with filesystem
Skills discover the guide under the same `DSH_HOME`. [DeepSeek Harness MCP guide](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/user/guide/mcp-memory.md)

NDM Next refreshes its installed Skill copies after a desktop upgrade,
a plugin install, update or removal, and an AI access change. The plugin
market's **AI on** control lets you disable AI use for one engine.
The engine stays available for regular desktop and browser downloads.

Use `ndm_list_engines` with a source to see compatible engines, the automatic
selection and current option schemas. The installed Skill includes download
recipes and a guide to interpreting task status. Each AI-enabled plugin has a
guide under `references/plugins/` with matching rules, options, MCP tools and
any usage notes supplied by its author. For task history,
`ndm_list_tasks` supports a state filter and a cursor for older results.

For plugin development, open `references/sdk/quickstart.md` inside the installed
Skill. The full SDK reference, a Rust worker and its matching JSON manifest are
available offline alongside it. The quickstart covers building, packaging,
importing and testing a plugin, sharing another plugin's tools and shipping a
usage guide with your package.

AI clients can queue downloads in the default folder or an explicit absolute
directory, check status, pause, and resume. A batch can specify its own
simultaneous-task and per-download thread settings. The app-wide task limit
and the engine's limits still apply. A request to change global settings
updates the app's defaults for queued and future work. Built-in HTTP caps
segment connections at 16 and HLS uses up to four concurrent fragments.
A plugin defines how it uses its thread setting.

Set the default User-Agent under **Settings → Download behavior → More
settings**. Start with a Chrome, Firefox, or Safari preset and edit it as
needed. Saving a blank value restores the Chrome for Windows preset. A User-Agent captured
by the browser extension or specified for an MCP item takes precedence.

Use `ndm_get_settings` and `ndm_set_settings` to read and change desktop preferences through chat. The desktop returns the saved values. The managed Skill includes examples for bandwidth, download folders, startup recovery, TUN routing, appearance and language. Use the switch in each installed plugin’s Settings to control its AI access.

MCP downloads run quietly by default. Follow progress in the main list or with `ndm_task_status`. Set `show_progress: true` on the batch or item for a separate window. Plugin download tools accept it too. Call `ndm_progress_window` with a task ID and `visible: true` or `false` to open or close that window while the task continues.

For built-in HTTP(S), including HLS and DASH, select the default route in **Settings → Proxy and network**, or override it in **New download** or the MCP item's `proxy_mode`. Omit it or use `inherit` for the app default. `system_route` and legacy `direct` skip HTTP proxies and use normal OS routing, including an active TUN. `tun_direct` uses the configured exit for all transfer connections and fails if it is unavailable. Set `user_agent` for an individual UA override.

For a DIRECT listener, configure your TUN client to listen for HTTP proxy connections on a dedicated IPv4 loopback port and route that inbound to DIRECT. Enter only the port in NDM Next. A normal mixed/proxy port follows its own rules. Linux and macOS also accept a physical interface name for TCP binding. DNS still uses the system resolver with an interface binding, and the OS or VPN can block the connection. Check `ndm_network_routes` for the configured exit type. Use the VPN client's connection log to verify its final outbound.

`ndm_task_status` and `ndm_list_tasks` provide a redacted `error` for failed
tasks. For built-in file downloads, look at `error.http_status`, `error.stage`,
`progress.http.range_probe_status`, `progress.http.redirect_count`, and
`proxy.effective` for the latest attempted route. Fields are null until the
relevant information is available, including for failures recorded by older
versions. While AI access is on, the scheduler samples ongoing task progress
about once a second. Use `progress.activity` to check the sample time,
recent transfer rate, and time since the last observed progress.

For HTTP transfers, `progress.http.range_mode` reports the range validator
(`strong_etag` or `verified_sha256`) or the reason for using one connection,
such as `missing_validator`, `range_ignored`, `connection_limit` or `rate_limited`.
Repository file paths can supply a name when a task is queued. A server attachment
name becomes visible while the download is running.

- [Plugin market catalog](https://github.com/AreChen/onesdm-plugin-market)
- [Download engine SDK](../desktop/crates/ndm-plugin-sdk/README.md)
- [NDM Next Skill](../desktop/crates/ndm-host/skills/SKILL.md)

# 插件与 AI 客户端

打开 **插件市场** 浏览、安装、更新或卸载插件。输入中英文名称或关键词，
可按分类筛选，也可以选择 **仅已安装**。卡片与分类跟随应用语言。市场会校验安装包的
SHA-256。也可以通过 **导入插件** 安装你信任的开发者提供的本地 ZIP。
插件包包含可执行的下载引擎。
从市场安装或更新时会显示已下载大小，以及校验与安装阶段。
增强插件可以直接使用已安装的流媒体下载插件提供的 yt-dlp。安装时如果缺少
所需插件或版本不兼容，界面会显示依赖名称。卸载时先移除依赖它的增强插件。

在已安装的插件卡片上打开 **设置**，可保存该插件的默认下载选项。默认值用于
新任务，也适用于 AI 客户端发起的下载。单次任务可单独修改选项。
已保存的默认值与下载任务选项使用同样的本地加密方式。
流媒体下载插件可优先保留原生封装，也可指定 MP4、WebM 或 MKV。字幕支持作者上传
和自动字幕，可以勾选多种语言。需要单独使用 HTTP(S) 或 SOCKS 代理时，在插件
设置中开启 **使用独立代理**，选择协议并填写主机和端口。

新建下载时粘贴链接或选择本地文件，NDM Next 会匹配已安装的引擎。
YouTube、Bilibili、TikTok、Vimeo 等常见媒体网页会匹配 **流媒体下载**。
其他站点的媒体网页可在 **下载引擎** 中手动选用该插件。HLS、DASH 和媒体直链
默认走内置引擎，也可选用流媒体下载。在多个自动匹配的插件之间选择时，应用会记住站点偏好。

添加 `.torrent` 或磁力链接时，点击 **选择下载文件** 勾选要保存的文件。
任务列表显示连接数和分享率，独立下载窗口可展开查看做种节点、已发现节点
及上传速度。下载完成后，两处均可 **开始做种** 或 **停止做种**。

在 **设置 → AI 集成** 展开客户端列表，为需要的客户端安装 MCP 和 Skill，再开启
**AI 接入**。AI 接入默认关闭。安装后重新打开对应客户端的会话。客户端行会显示
本地配置状态，移动过 NDM Next 时可点 **修复**。

| 客户端 | 用户级 MCP 配置 | 用户级 Skill 目录 |
| --- | --- | --- |
| OpenAI Codex CLI 与桌面版 | `~/.codex/config.toml` | `~/.agents/skills/ndm-next/` |
| Claude Code | `~/.claude.json` | `~/.claude/skills/ndm-next/` |
| 支持内置 MCP 的 Pi | `~/.pi/agent/mcp.json` | `~/.pi/agent/skills/ndm-next/` |
| OpenCode | `~/.config/opencode/opencode.json(c)` | `~/.config/opencode/skills/ndm-next/` |
| DeepSeek Harness | `~/.dsh/cordis.patch.yml` | `~/.dsh/skills/ndm-next/` |

同一 Codex 主机上的 CLI 和桌面版共用 MCP 配置，只有桌面版也可安装。
Pi 需要 0.99.0 或更新版本使用内置 MCP。OpenCode 支持旧版 MCP 配置和 V2 的
`mcp.servers` 结构。表中为默认位置，NDM Next 启动时继承的客户端绝对配置路径
可覆盖默认值。

DeepSeek Harness 通过自带的 stdio MCP 客户端连接 NDM Next。用户级 Cordis 配置对
各个 profile 生效。启用了文件系统 Skill 的 profile 会在同一个 `DSH_HOME` 下发现使用指南。[DeepSeek Harness MCP 官方指南](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/user/guide/mcp-memory.zh.md)

桌面升级、安装、更新或卸载插件以及更改 AI 接入后，应用会刷新各客户端中
已安装的 NDM Next Skill。点击插件市场卡片的 **AI 已开** 可关闭单个引擎的
AI 接入。桌面和浏览器仍可使用该引擎下载。

将来源传给 `ndm_list_engines` 可查看兼容引擎、自动选择和当前参数。
主 Skill 附有下载示例和任务诊断指南，并链接到 `references/plugins/` 中对 AI
开放的各个插件。每份插件指南列出来源匹配规则、选项、MCP 工具和作者提供的使用说明。
`ndm_list_tasks` 可按状态筛选，并通过分页游标继续查询更早的任务。

开发插件时，打开已安装 Skill 中的 `references/sdk/quickstart.md`。同目录包含可离线阅读的
完整 SDK 协议、Rust worker 示例和配套 JSON 清单。按指南可以构建、打包、导入、测试插件，
复用其他插件的工具，并将插件自己的使用指南随包交付。

AI 客户端可发起下载、查询状态、暂停与恢复。省略保存位置时使用 NDM
默认下载目录，也可传入绝对路径。单批任务可以设置并发任务数和每项
下载线程数，并且仍受全局任务上限与引擎上限约束。明确要求修改全局
设置时，新值会应用到排队和后续任务。内置 HTTP 最多使用 16 个分段
连接，HLS 同时下载最多 4 个分片。插件可根据协议决定线程设置的用途。

在 **设置 → 下载行为 → 更多设置** 可设置默认 User-Agent。可以选用
Chrome、Firefox 或 Safari 预设，再按需编辑。留空保存会恢复为 Windows 版 Chrome 的 UA。
浏览器接管请求捕获的 User-Agent 和 MCP 单项指定的值优先于默认值。

可以在聊天中使用 `ndm_get_settings` 和 `ndm_set_settings` 读取或修改主程序设置。桌面端返回保存后的值。Skill 提供限速、下载目录、恢复、路由、外观与语言的配置示例。每个已安装插件的设置列表中可单独切换 AI 接入。

AI 下载默认静默，进度可在主列表或 `ndm_task_status` 查看。批次或单项设置 `show_progress: true` 可弹出独立进度窗，插件专用工具也支持该参数。调用 `ndm_progress_window`，传入任务 ID 和 `visible: true` 或 `false` 可打开或关闭独立窗，任务继续运行。

内置 HTTP(S)、HLS 和 DASH 在**设置 → 代理与网络**选择默认路由，也可在新建下载或 MCP 单项的 `proxy_mode` 中覆盖。省略或选 `inherit` 沿用应用默认。`system_route` 和旧值 `direct` 会跳过 HTTP 代理，跟随操作系统路由，包括活动 TUN。`tun_direct` 使用已配置的直连出口，出口不可用时下载失败。单项可用 `user_agent` 指定 UA。

使用本地端口时，先在 TUN 客户端建立独立的 HTTP 代理入站，监听 IPv4 回环地址，并将这个入站路由到 DIRECT。在 NDM Next 填入端口号。普通混合端口仍遵循它自己的分流规则。Linux 和 macOS 也可以填物理网卡名称，绑定下载 TCP 连接；域名解析仍使用系统 DNS，系统或 VPN 策略可能阻止连接。用 `ndm_network_routes` 可查看已配置的出口类型，最终出站可在代理客户端的连接日志中核对。

`ndm_task_status` 与 `ndm_list_tasks` 为失败任务返回脱敏的 `error`。
内置文件下载会记录 `error.http_status`、`error.stage`、
`progress.http.range_probe_status`、`progress.http.redirect_count`，
以及最近一次请求使用的 `proxy.effective`。缺少记录时字段返回空值，
旧版本失败的任务也适用。开启 AI 接入后，调度器约每秒记录一次进行中
任务的进度。通过 `progress.activity` 可查询采样时间、近期传输速度和
自上次观察到进展以来的时长。

HTTP 任务的 `progress.http.range_mode` 会显示分段所用的校验方式（`strong_etag`
或 `verified_sha256`），也会说明因缺少校验依据、服务器忽略 Range、连接数设置
或服务端限流而使用单连接的情况。任务排队时可以从仓库路径获取文件名，
收到服务器的附件名后，下载过程中就会更新。
