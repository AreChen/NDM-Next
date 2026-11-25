# 下载与批量任务 · Download recipes

## 文件和模型仓库

直链、ModelScope 的 `repo?Revision=…&FilePath=…`、Hugging Face 的 `/resolve/…` 可以直接交给 `ndm_download`。保留仓库 URL、revision 和原有编码，这样任务重试时还能向仓库获取有效的下载地址。任务入队后即可从仓库路径得到文件名，服务器提供的附件名和最终落盘名也会反映到任务状态。

下面的参数把同一批的并行任务数设为 2，第二个文件使用 4 个下载线程。省略 `destination` 会使用应用的下载目录。单项的绝对目录可覆盖批次目录。

```json
{
  "batch_active_tasks": 2,
  "items": [
    {"source":"https://example.org/models/config.json"},
    {"source":"https://example.org/models/weights.safetensors","download_threads":4}
  ]
}
```

每批可提交 1 至 128 项。`batch_active_tasks` 的范围是 1 至 32，`download_threads` 的工具输入范围是 1 至 256，可在批次或单项指定。内置 HTTP 引擎同时最多调度 16 个分段，服务器、文件大小、插件能力和应用设置也会影响实际并发。只有用户要改变整个应用的默认值时才使用 `ndm_set_download_limits`，可以先用 `ndm_get_download_limits` 读取现值。

## 网页、播放列表、种子和本地文件

先用 `ndm_list_engines` 查询来源。这个调用只在本机匹配已安装的引擎，可以在排队前使用。

```json
{"source":"https://www.youtube.com/watch?v=example"}
```

从返回的 `plugins[].tools` 选择专用工具并使用其实际参数。专用工具直接接收 `source`，通用 `ndm_download` 则接收 `items` 数组。若某引擎的 `automatic_match` 为 `false`，使用专用工具，或在相应下载项中指定该插件 ID。结合插件指南和实时选项 schema 选择格式、字幕或其他设置。省略未指定的选项，让插件沿用用户已保存的值。[本机插件指南](plugins/index.md)

HTTP(S)、FTP(S)、SFTP、HLS、静态 DASH 是内置引擎支持的来源。媒体网页、磁力链接和本地种子的能力取决于已安装的引擎。本地来源使用桌面端可访问的绝对路径。MCP 返回的 `builtin` 是发现结果，下载项的 `plugin` 参数用于已安装插件的 ID。

## 单任务网络设置

省略 `proxy_mode` 或使用 `inherit` 沿用桌面设置。用户要按默认 TUN 分流时选 `system_route`。它和旧的 `direct` 都跳过 HTTP 代理，连接仍服从系统路由。用户要在 TUN 开启时使用直连出口，先调用 `ndm_network_routes`。若 `tun_direct_exit` 为空，请用户在「设置 → 代理与网络」配置出口。已有出口时使用以下参数。

```json
{"items":[{"source":"https://example.org/models/weights.bin","proxy_mode":"tun_direct"}]}
```

`bound_interface` 表示绑定网卡（Linux/macOS），DNS 仍使用系统解析器。`local_direct_listener` 表示发送到用户在代理客户端配置为 DIRECT 的本地 HTTP 入站（三个平台均支持）。这些值描述下载器的连接方式，代理客户端日志可以确认最终出站。出口不可用时报告失败，不自动切换线路。单项路由和 `user_agent` 用于内置 HTTP(S)，包括 HLS 和 DASH。插件选项以其实时 schema 为准。

## 静默下载与进度窗口

默认只在主任务列表显示。用户要求单独显示某项时，对该项传 `"show_progress":true`。整批需要显示时在 `items` 旁设置同名字段，单项值优先。插件专用工具直接接收该字段。已有任务无需重新下载，调用 `ndm_progress_window` 即可操作独立窗口，例如：

```json
{"task_id":"11111111-1111-4111-8111-111111111111","visible":false}
```

`visible:true` 打开或激活该窗口，`false` 关闭该窗口。任务继续运行，返回的 `requested:true` 表示桌面端已收到待处理的请求。

成功入队后记录 `task_ids`，并在用户要求跟踪时用这些 ID 查询状态。设置了批次并发时，结果还会包含 `batch_id`。[状态与排查](diagnostics.md)
