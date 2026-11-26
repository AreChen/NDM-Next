# 用聊天配置主程序 · Configure the desktop app

先调用 `ndm_get_settings`。用户说「这一次」时使用下载项参数；用户要以后都这样时才修改全局默认。用户只描述目标时，先用日常语言解释建议值、单位和影响。每次只提交用户希望修改的字段。成功回复包含桌面程序保存后的设置；超时后重新读取，确认当前值。

| 用户希望 | 调用与参数 |
| --- | --- |
| 默认限速 2 MiB/s | `ndm_set_settings`，传 `{"speed_limit_kib_s":2048}`。0 表示不限速。 |
| 默认存到某个目录 | `download_directory` 填用户给出的本机绝对路径。空字符串恢复系统下载文件夹。 |
| 重启后继续下载，自动检查更新 | `{"auto_resume":true,"auto_update":true}`。自动更新选项控制检查。 |
| 修改外观或语言 | `theme` 支持 `system`、`light`、`dark`；`language` 支持 `en`、`zh-CN`、`zh-Hant`、`ja`、`ko`、`es`、`pt-BR`、`fr`、`de`；`ui_animations` 为布尔值。 |
| 更换 UA 或调整缓存保留 | `user_agent` 为空恢复 Chrome 默认。 `cache_retention_days` 为 0–3650，0 关闭自动清理。 |
| 配置 TUN 或应用代理 | `download_route` 控制内置 HTTP(S)。用户已指定的 `tun_direct_exit` 可以与 `"download_route":"tun_direct"` 一起保存。Windows 使用映射至 DIRECT 的本地 HTTP 端口。Linux/macOS 也接受物理网卡名。AI 可通过 `use_saved_proxy` 启用已保存的代理；密码在桌面设置填写。[路由说明](downloads.md) |
| 改全局并发或线程数 | 使用 `ndm_set_download_limits`。例如 `{"active_tasks":2,"download_threads":8}`。随后用 `ndm_get_settings` 回读。 |

下载参数应用于后续工作。用户可以继续在桌面修改这些设置。旧客户端若未列出上述工具，刷新 MCP 会话后再检查实时 schema。
