# 用聊天配置主程序 · Configure the desktop app

先调用 `ndm_get_settings`。用户说「这一次」时使用下载项参数；用户要以后都这样时才修改全局默认。用户只描述目标时，先用日常语言解释建议值、单位和影响。每次只提交用户希望修改的字段。成功回复包含桌面程序保存后的设置；超时后重新读取，确认当前值。

| 用户希望 | 调用与参数 |
| --- | --- |
| 默认限速 2 MiB/s | `ndm_set_settings`，传 `{"speed_limit_kib_s":2048}`。0 表示不限速。 |
| 默认存到某个目录 | `download_directory` 填用户给出的本机绝对路径。空字符串恢复系统下载文件夹。 |
| 重启后继续下载，自动检查更新 | `{"auto_resume":true,"auto_update":true}`。自动更新选项控制检查。 |
| 修改外观或语言 | `theme` 支持 `system`、`light`、`dark`，`color_scheme` 选择 `classic`、`silver`、`ocean`、`forest`、`sunset`、`custom`。`language` 使用工具列出的语言码，`ui_animations` 为布尔值。 |
| 自定义界面色 | `{"theme_colors":{"primary":"#237A89","secondary":null,"surface":null}}` 会显示自定义主题。`null` 恢复主色默认值或让辅助色、底色自动配色。 |
| 修改任务状态色 | `{"download_colors":{"downloading":"#42C7EB","failed":null}}`。状态键还有 `completed`、`paused`、`queued`。`null` 恢复对应状态的默认色。 |
| 更换 UA 或调整缓存保留 | `user_agent` 为空恢复 Chrome 默认。 `cache_retention_days` 为 0–3650，0 关闭自动清理。 |
| 配置 TUN 直连端口 | `{"tun_direct_port":18086}`。端口须是用户的本地 HTTP DIRECT 入站。只有用户要求全局默认直连时才附加 `"download_route":"tun_direct"`。0 清除出口，同时把全局默认路由切回用户指定的其他模式。Linux/macOS 可改用 `tun_direct_exit` 传入网卡名。[路由说明](downloads.md) |
| 保存应用代理 | `{"proxy":{"url":"http://127.0.0.1:8080"},"use_saved_proxy":true}`。代理对象会替换旧配置。用户名和密码可单独写入，查询只返回代理地址和认证状态。空 URL 清除代理。 |
| 更换缓存目录、开机自启或侧栏 | `cache_directory` 接受绝对路径，空字符串恢复默认。用 `start_at_login` 调整开机自启，用 `sidebar_collapsed` 和 `sidebar_width` 调整侧栏。 |
| 改全局并发或线程数 | 使用 `ndm_set_download_limits`。例如 `{"active_tasks":2,"download_threads":8}`。随后用 `ndm_get_settings` 回读。 |

下载参数应用于后续工作。用户可以继续在桌面修改这些设置。旧客户端若未列出上述工具，刷新 MCP 会话后再检查实时 schema。

AI 接入授权、客户端与浏览器扩展安装、手动清理缓存及检查更新在桌面设置页和相应客户端中操作。
