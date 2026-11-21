# Plugins and Codex CLI

Open **Plugin market** to browse, install, update, or uninstall plugins.
Search by name or keyword in English or Chinese, and narrow the cards by
category or **Installed only**. The display language follows **Settings → Language**.
The market validates the downloaded package against its published SHA-256
checksum. **Import plugin** also accepts a compatible local ZIP from a
developer you trust. An engine package contains a native executable.

Open **Settings** on an installed plugin card to save default download options.
They apply to new tasks, including downloads requested through Codex CLI.
An option chosen for an individual task takes precedence.
Saved defaults use the same local protection as download task options.

Paste a URL or select a local source file in **New download**. NDM Next
selects an installed engine that supports the source. When several match,
choose one in the **Download engine** menu. Choosing a plugin for a site
saves it as that site's default. YouTube and BitTorrent have their own
format and peer settings in the native form.

In **Settings → AI integration**, install the Codex CLI MCP registration
and Skill when you want them. Turn on **AI access** to make download tools
available. AI access starts disabled. When enabled, tools from installed
plugins become available automatically. Restart an existing Codex CLI
session after first installing the integration.

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

在已安装的插件卡片上打开 **设置**，可保存该插件的默认下载选项。默认值用于
新任务，也适用于 Codex CLI 发起的下载。单次任务可单独修改选项。
已保存的默认值与下载任务选项使用同样的本地加密方式。

新建下载时粘贴链接或选择本地文件，NDM Next 会匹配已安装的引擎。
有多个匹配项时，可在 **下载引擎** 中手动选择。为某个网站选择插件后，
应用会记住这个选择。YouTube 和 BitTorrent 都在原生面板中提供自己的选项。

在 **设置 → AI 集成** 按需为 Codex CLI 安装 MCP 和 Skill，再开启
**AI 接入**。AI 接入默认关闭。开启后，已安装插件会自动向 Codex
公开各自的下载工具。首次安装后请重启已打开的 Codex CLI 会话。

Codex 可发起下载、查询状态、暂停与恢复。省略保存位置时使用 NDM
默认下载目录，也可传入绝对路径。单批任务可以设置并发任务数和每项
下载线程数，并且仍受全局任务上限与引擎上限约束。明确要求修改全局
设置时，新值会应用到排队和后续任务。内置 HTTP 最多使用 16 个分段
连接，HLS 同时下载最多 4 个分片。插件可根据协议决定线程设置的用途。
