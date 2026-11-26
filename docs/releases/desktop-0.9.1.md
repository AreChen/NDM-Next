## English

- Plugin market cards adapt to the available width. Search and filters stack in narrow windows, long descriptions have more room, and the last row uses its full width.
- Media Downloader 0.2.3 reports estimated sizes for streams when yt-dlp has enough data. Estimates are marked with `≈` in the task list and progress window. Completed tasks show the actual file size.
- New Media Downloader tasks using the default cache stage media on the destination volume so publishing can avoid a full cross-volume copy. Media Downloader also skips the extra MP4 index relocation pass during local remuxing.
- The HTTP resume test waits for the client to write partial bytes before pausing, removing a timing failure on macOS CI.

## 中文

- 插件市场卡片随窗口宽度排列。窄窗口下搜索框和筛选按钮分行，长描述可多显示一行，最后一行充分利用可用宽度。
- 流媒体下载 0.2.3 可显示由 yt-dlp 得到的体积估算。主任务列表和独立进度窗口用 `≈` 标记估算，完成后显示文件实际大小。
- 使用默认缓存时，新建流媒体任务在保存目录所在的磁盘写临时数据，省去最后的跨盘整文件复制。流媒体插件也省去了本地封装后的 MP4 索引前移步骤。
- HTTP 续传测试在客户端实际写入部分数据后再暂停，修复 macOS CI 上的时序失败。
