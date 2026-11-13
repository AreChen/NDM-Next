<div align="center">
  <img src="extension/icons/icon-128.png" width="112" alt="NDM Next 图标">
  <h1>NDM Next</h1>
  <p>为 Neat Download Manager 打造的简洁、智能的浏览器扩展。</p>
  <p>简体中文 · <a href="README.md">English</a></p>
</div>

NDM Next 将浏览器下载交给 Neat Download Manager，同时过滤不需要的文件，让下载列表保持清爽。

仓库还包含 Rust Windows 下载器及浏览器集成，参见[桌面版构建说明](desktop/packaging/windows/README.md)。

- 按文件类型、大小和域名过滤下载。
- 一键将当前网站加入黑名单。
- 识别音频和视频，发送至 NDM。
- 支持中英文切换。

## 安装

1. 安装并运行 [Neat Download Manager](https://www.neatdownloadmanager.com/)。
2. 从 [Releases](https://github.com/AreChen/NDM-Next/releases/latest) 下载并解压 `NDM-Next-1.0.0.zip`。
3. 打开 `brave://extensions` 或 `chrome://extensions`，开启开发者模式，点击“加载已解压的扩展程序”，选择解压后的文件夹。
4. 停用原版 NDM 扩展，避免重复接管。

NDM Next 是独立项目，与 Neat Download Manager 官方无关联。

[MIT 许可证](LICENSE)
