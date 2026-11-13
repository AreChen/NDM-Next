# NDM Next

A Chromium extension for Neat Download Manager, with configurable download filtering and a compact bilingual interface.

[中文](#中文) · [English](#english)

## 中文

NDM Next 将符合规则的浏览器下载交给本机运行的 Neat Download Manager（NDM）。项目仍处于早期开发阶段，建议先用不重要的文件测试。

### 功能

- 下载接管开关；默认排除小于 1 MiB 的文件，可修改或关闭。
- 默认排除 `.data`、字体等文件类型，支持自定义。
- 域名黑名单：检查来源页面和下载地址，包含子域名；可一键添加当前网站。
- 在扩展面板列出检测到的音频和视频资源，并手动发送到 NDM。
- 中英文界面，默认跟随浏览器。
- NDM 无法连接时保留浏览器下载。

### 安装

1. 安装并运行 Neat Download Manager。
2. 下载或克隆本仓库，打开 Brave 的 `brave://extensions` 或 Chrome 的 `chrome://extensions`。
3. 开启开发者模式，点击“加载已解压的扩展程序”，选择仓库中的 `extension` 文件夹。
4. 关闭旧的 NDM 扩展以避免重复接管。点击 NDM Next 图标检查连接状态，再使用测试文件验证。

### 注意事项

- 浏览器有时无法提前提供下载文件大小；扩展会尝试 HEAD 请求，仍未知时允许接管。已经完成的浏览器下载不会接管。
- 扩展通过 `ws://127.0.0.1:10007/download` 向 NDM 发送任务。此接口没有已知的任务确认机制，因此连接成功不保证 NDM 已接受任务。下载接管和回退仍需在实际环境验证。
- 为支持登录下载，任务可能包含该地址的 Cookie 和来源页面地址，仅发送给本机 NDM。扩展不会向其他服务器上传这些信息。请求目标网站可能会收到用于获取大小的 HEAD 请求。
- 媒体识别依赖浏览器观察到的响应类型，不能保证支持所有网站、分段流或受保护内容。媒体列表保存在扩展的临时内存中，可能在后台休眠后清空。
- 浏览器或 NDM 更新后，本地通信接口可能发生变化。NDM Next 与 Neat Download Manager 官方无关联。

## English

NDM Next hands eligible browser downloads to a locally running Neat Download Manager (NDM). This project is in early development; test it with nonessential files first.

### Features

- Toggle download interception; exclude files smaller than 1 MiB by default, with a configurable threshold.
- Customize excluded file extensions and blocked domains, including subdomains and the current site.
- Discover audio and video resources and send them to NDM manually.
- English and Chinese interface, following the browser language by default.
- Leave downloads with the browser when NDM is unavailable.

### Installation

1. Install and run Neat Download Manager.
2. Clone or download this repository. Open `brave://extensions` or `chrome://extensions`.
3. Enable developer mode, select **Load unpacked**, and choose this repository's `extension` folder.
4. Disable the original NDM extension to avoid duplicate interception. Check the connection status and test with a disposable download.

### Limitations and privacy

File sizes are not always available before interception; a HEAD request may be used and unknown sizes remain eligible. Downloads already completed by the browser are left alone. The local NDM protocol has no known task acknowledgment, so a successful connection does not prove task acceptance. Actual interception and fallback require testing in your environment.

Cookies and the referring page may be sent to the local NDM instance for authenticated downloads; they are not uploaded to other servers by the extension. The target website may receive a HEAD request. Media detection depends on response types and may not support every site, segmented stream, or protected content. The temporary media list can be cleared when the service worker sleeps. This project is not affiliated with Neat Download Manager.

## Development

Run `node --test tests/*.test.js` with a recent Node.js version. No dependencies are required.

## License

MIT. See [LICENSE](LICENSE).
