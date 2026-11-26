## English

- During startup or a service worker restart, the request-matching cache stays off until settings load successfully and a feature needs it. Switching off both download takeover and media detection clears the cache.
- Download takeover and media detection can be used independently.
- The extension connects to `com.onesdm.host` on OnesDM 0.8.2 and later. It uses `com.ndmnext.host` when an older desktop version is installed and the new host name is unavailable. This alias is scheduled for removal in 0.9.0.

## 中文

- 扩展启动或 Service Worker 重启时，只有设置成功读取且有功能需要时，才会暂存用于匹配的请求头。关闭下载接管和媒体识别后会清空缓存。
- 下载接管和媒体识别可以独立使用。
- OnesDM 0.8.2 起使用原生宿主名 `com.onesdm.host`。连接旧版桌面端且新宿主名不可用时，扩展会使用 `com.ndmnext.host`。旧名计划在 0.9.0 移除。
