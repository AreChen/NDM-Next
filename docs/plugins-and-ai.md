# Plugins and Codex CLI

Open **Plugin market** to browse, install, update, or uninstall plugins.
Search by name or keyword in English or Chinese, and narrow the cards by
category or **Installed only**. The display language follows **Settings → Language**.
The market validates the downloaded package against its published SHA-256
checksum. **Import plugin** also accepts a compatible local ZIP from a
developer you trust. An engine package contains a native executable.
The market shows downloaded bytes and the verification and installation stages.

Open **Settings** on an installed plugin card to save default download options.
They apply to new tasks, including downloads requested through Codex CLI.
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

In **Settings → AI integration**, install the Codex CLI MCP registration
and Skill when you want them. Turn on **AI access** to make download tools
available. AI access starts disabled. When enabled, tools from installed
plugins become available automatically. Restart an existing Codex CLI
session after first installing the integration. From NDM Next 0.6.0, the
installed Skill refreshes after a desktop upgrade, a plugin install, update
or removal, and an AI access change. The plugin market's **AI on** control
lets you disable AI use for one engine. The engine stays available for regular
desktop and browser downloads.

Use `ndm_list_engines` with a source to see compatible engines, the automatic
selection and current option schemas. The main Skill links to plugin guides
under `references/plugins/` for source-specific usage. For task history,
`ndm_list_tasks` supports a state filter and a cursor for older results.

Codex can queue downloads in the default folder or an explicit absolute
directory, check status, pause, and resume. A batch can specify its own
simultaneous-task and per-download thread settings. The app-wide task limit
and the engine's limits still apply. A request to change global settings
updates the app's defaults for queued and future work. Built-in HTTP caps
segment connections at 16 and HLS uses up to four concurrent fragments.
A plugin defines how it uses its thread setting.

- [Plugin market catalog](https://github.com/AreChen/NDM-Next-Plugin-Market)
- [Download engine SDK](../desktop/crates/ndm-plugin-sdk/README.md)
- [Codex CLI Skill](../desktop/crates/ndm-host/skills/SKILL.md)

# 插件与 Codex CLI

打开 **插件市场** 浏览、安装、更新或卸载插件。输入中英文名称或关键词，
可按分类筛选，也可以选择 **仅已安装**。卡片与分类跟随应用语言。市场会校验安装包的
SHA-256。也可以通过 **导入插件** 安装你信任的开发者提供的本地 ZIP。
插件包包含可执行的下载引擎。
从市场安装或更新时会显示已下载大小，以及校验与安装阶段。

在已安装的插件卡片上打开 **设置**，可保存该插件的默认下载选项。默认值用于
新任务，也适用于 Codex CLI 发起的下载。单次任务可单独修改选项。
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

在 **设置 → AI 集成** 按需为 Codex CLI 安装 MCP 和 Skill，再开启
**AI 接入**。AI 接入默认关闭。开启后，已安装插件会自动向 Codex
公开各自的下载工具。首次安装后请重启已打开的 Codex CLI 会话。
从 NDM Next 0.6.0 起，桌面升级、安装、更新或卸载插件以及更改 AI 接入后，
应用会刷新已安装的 Skill。点击插件市场卡片的 **AI 已开** 可关闭单个引擎的
AI 接入。桌面和浏览器仍可使用该引擎下载。

将来源传给 `ndm_list_engines` 可查看兼容引擎、自动选择和当前参数。
主 Skill 会链接到 `references/plugins/` 中各个引擎的使用指南。
`ndm_list_tasks` 可按状态筛选，并通过分页游标继续查询更早的任务。

Codex 可发起下载、查询状态、暂停与恢复。省略保存位置时使用 NDM
默认下载目录，也可传入绝对路径。单批任务可以设置并发任务数和每项
下载线程数，并且仍受全局任务上限与引擎上限约束。明确要求修改全局
设置时，新值会应用到排队和后续任务。内置 HTTP 最多使用 16 个分段
连接，HLS 同时下载最多 4 个分片。插件可根据协议决定线程设置的用途。
