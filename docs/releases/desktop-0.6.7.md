## English

- MCP downloads now run quietly in the main task list. Set `show_progress` on a batch or item to open independent progress windows. Use `ndm_progress_window` to show or close an existing task’s window while its download continues. Plugin downloads support quiet mode too.
- Choose the default built-in HTTP(S) route in **Settings → Proxy and network**, or select a route for one new download. OS routing follows the active TUN. For TUN direct, configure a dedicated loopback HTTP listener mapped to DIRECT in your proxy client. Linux and macOS can also bind download connections to a network interface. The setting covers HLS and DASH as well.
- The updated NDM Next Skill documents window controls, route discovery with `ndm_network_routes`, and per-item `proxy_mode` options.

## 中文

- MCP 下载默认静默，主任务列表照常显示进度。批次或单项设置 `show_progress` 可弹出独立窗口；`ndm_progress_window` 可打开或关闭已有任务的进度窗，下载继续运行。插件下载也支持静默模式。
- 在**设置 → 代理与网络**选择内置 HTTP(S) 的默认路由，也可在新建下载时单独指定。系统路由遵循当前 TUN。需要 TUN 直连时，可在代理客户端配置出站为 DIRECT 的本地 HTTP 端口；Linux 和 macOS 还支持绑定下载网卡。该设置同样用于 HLS 和 DASH。
- 配套 Skill 已补充窗口控制、`ndm_network_routes` 路由查询以及单项 `proxy_mode` 用法。
