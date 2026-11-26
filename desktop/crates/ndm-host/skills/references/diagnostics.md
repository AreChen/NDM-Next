# 任务状态与排查 · Task status and diagnostics

手上有任务 ID 时，直接调用 `ndm_task_status`。只有文件名时，用 `ndm_list_tasks` 按状态筛选，从最近任务开始查。把返回的 `next_cursor` 连同原筛选条件传入下一次请求，直到游标为空。

```json
{"state":"running","limit":20}
```

| 用户看到的情况 | 读取哪些字段，如何解释 |
| --- | --- |
| 已排队、还没有速率 | 查看 `state`。常见状态为 `prepared`、`queued`、`running`、`paused`、`completed`、`failed`。完成后用 `output` 确认落盘位置，文件名可能因重名而变化。 |
| 看起来卡住了 | 隔几秒取两次状态，比较 `progress.activity.sampled_at_unix_ms`、`idle_ms`、`recent_bytes_per_second` 和 `progress.downloaded`。检查 `progress.phase`。合并、封装和保存阶段有本地工作，网络速度可能为零。未知总大小时报告已收字节。 |
| 想知道流媒体文件有多大 | `progress.estimated_total` 是按当前流数据得出的估算字节数。完整下载后用 `progress.total` 报告实际大小。 |
| 只有一个活动分段 | 出现 `segment_fallback` 或分段失败时，查看 `progress.http.segment_failure` 的分段索引（从 0 开始）、请求的 start/end、response_range、content_length、strong_etag、issue、stage、http_status、redirects 和 retries。范围结束字节包含在内。任务完成时顶层的 200 可能来自后续完整 GET。读取 `progress.http.range_probe_status`、`range_mode` 和 `progress.segments`。`connection_limit` 表示任务限为一个连接，`rate_limited` 表示服务端限制并发。`range_ignored`、`range_rejected`、`invalid_content_range`、`segment_fallback` 分别表示忽略 Range、拒绝 Range、范围不一致或分段后退回完整传输。`missing_validator` 表示缺少可靠版本或摘要。 |
| 模型经过签名 CDN | `strong_etag` 和 `verified_sha256` 说明分段使用的校验方式。单个 `206` 仅说明该请求得到了部分内容。对于 SHA-256 模式，最终合并的数据还要通过摘要校验。 |
| HTTP 任务失败 | 一起读取 `error.code`、`error.http_status`、`error.stage` 和 `progress.http.redirect_count`。例如，探测得到 `206`，随后分段请求得到 `403`，这两个状态属于不同阶段。`checksum_mismatch` 表示内容校验失败。 |
| 想知道是否走了代理 | `proxy.requested` 是请求策略，`proxy.effective` 表示最近一次 HTTP 请求尝试使用的连接方式。`os_route` 和 `direct` 服从 OS 路由；`system_proxy`、`configured_proxy`、`bound_interface`、`local_direct_listener` 分别表示系统 HTTP 代理、应用代理、指定网卡和本地 DIRECT 入站。最终 TUN 出站由系统或代理客户端决定。诊断报告可给出任务 ID、阶段和状态码。签名查询串、Cookie、Authorization 和代理凭据应留在本机。 |

若相关字段缺失或为 `null`，说明这条记录缺少对应观测。旧任务也可能出现这种情况。复述原因时区分服务器返回的状态、下载器采取的回退动作和仍需验证的推断。

用户要求暂停时调用 `ndm_pause_task`，继续已暂停任务或重试失败任务时调用 `ndm_resume_task`。这两个工具都接收相同形式的参数，并向桌面调度器提交命令。用返回的任务 ID 再查状态，确认命令生效。

```json
{"task_id":"11111111-1111-4111-8111-111111111111"}
```
