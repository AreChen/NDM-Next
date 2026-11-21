# Companion extension / 配套扩展

## English

1. Install and start the desktop version shown in the ZIP filename.
2. Extract `NDM-Next-Extension-Desktop-<desktop-version>-v<extension-version>.zip` into a permanent folder.
3. Open `chrome://extensions` (or `edge://extensions`), turn on Developer mode, choose **Load unpacked**, and select that folder containing `manifest.json`.
4. Open the extension settings and choose **NDM Next** as the download manager. Eligible downloads are now captured; files smaller than 1 MB stay in the browser by default.

On macOS, allow the requested download-folder access when the first download is saved. Saving can wait for that system prompt to be answered.

On Linux, start the app from its extracted directory once to register the native host. See the [Linux installation guide](../desktop/packaging/linux/README.md#browser-extension).

Starting with desktop 0.1.9 / extension 1.1.4, release extensions have the fixed ID `cpapmfpgmpbpnjgibdgkfkdeblamiodm`. The native host authorizes this ID on Windows, macOS, and Linux. The public `key` in the manifest preserves this ID across installation paths; it is not a secret or a signing credential. Store extensions retain their own IDs.

If upgrading an older unpacked extension without a fixed ID, remove the older copy and load the new folder. Check your extension preferences afterward; browser settings from the old ID do not migrate automatically. Keep just one active copy to avoid duplicate takeover. Future ZIP upgrades can replace files in the same folder, followed by **Reload** on the extensions page and a refresh of open webpages.

To keep an older unpacked extension on macOS or Linux, copy its ID from the extensions page into **NDM Next → Settings → Browser extension → Repair connection**. On macOS, the equivalent command is:

```sh
"/Applications/NDM Next.app/Contents/MacOS/ndm-host" --register-browser-hosts <extension-id>
```

On Windows, the bundled helper allows a custom developer ID:

```powershell
pwsh -File .\enable-dev-extension.ps1 -ExtensionId <extension-id>
```

The desktop updater updates the desktop app. Unpacked extensions must be updated manually from the matching release ZIP.

GitHub Actions tests and packages changes to the extension on the main branch.
Use **Run workflow** to build a ZIP on demand and download the
`ndm-next-extension` artifact from the completed run. Desktop releases
include the extension ZIP built from the release tag. An `extension-v*` tag
publishes the extension ZIP and checksum as a separate GitHub Release.
[Browser extension build](https://github.com/AreChen/NDM-Next/actions/workflows/extension-build.yml)

Use **Open page resources** to search and select the images, video, audio,
HLS and DASH resources observed in a tab. The window shows available image
and player previews. The takeover size setting applies to automatic browser
downloads. Under **Settings**, select English, Simplified Chinese or the
browser language. **Versions and plugins** lists the extension, desktop
and installed plugin versions whenever the desktop provides them.

The extension offers **Send page to a plugin** when the desktop reports
a compatible installed engine. This sends the page URL to the selected
engine after you click. The compact player control appears on hover or
keyboard focus. Close it for the current page with **×**, or switch off
**Buttons beside media** in the extension settings.

## 简体中文

1. 安装并启动 ZIP 文件名标明的桌面版本。
2. 将 `NDM-Next-Extension-Desktop-<桌面版本>-v<扩展版本>.zip` 解压到固定目录。
3. 打开 `chrome://extensions`（Edge 为 `edge://extensions`），开启开发者模式，点击**加载已解压的扩展程序**，选择包含 `manifest.json` 的目录。
4. 打开扩展设置，将下载管理器设为 **NDM Next**。符合规则的下载将自动接管；默认小于 1 MB 的文件仍由浏览器下载。

macOS 首次保存下载时，请允许系统请求的下载目录访问权限。未处理系统提示时，保存过程会等待授权。

Linux 用户从解压目录启动桌面应用一次，即可注册原生宿主。参见 [Linux 安装指南](../desktop/packaging/linux/README.md#browser-extension)。

从桌面版 0.1.9 / 扩展 1.1.4 开始，Release 扩展使用固定 ID `cpapmfpgmpbpnjgibdgkfkdeblamiodm`。Windows、macOS 和 Linux 的原生宿主均预先授权此 ID。manifest 中的公钥 `key` 用于保持不同安装目录下的 ID 一致，不是私钥或签名凭据。商店版继续使用各自的商店 ID。

首次从旧版未固定 ID 的扩展升级，请移除旧副本，再加载新版目录。旧 ID 的扩展设置不会自动迁移，请检查新版设置；只保留一个启用的副本，避免重复接管。以后的 ZIP 更新可以覆盖原目录文件，然后在扩展管理页点击**重新加载**并刷新已打开的网页。

macOS 或 Linux 若要继续使用旧扩展，可复制扩展 ID，粘贴到 **NDM Next → 设置 → 浏览器扩展 → 修复连接**。Windows 自定义开发者 ID 可使用 ZIP 内的 `enable-dev-extension.ps1`。

桌面自动更新只更新桌面程序。解压加载的扩展需手动下载对应 Release ZIP 更新。

主分支的扩展改动会触发 GitHub Actions 测试和打包，也可以点 **Run workflow** 手动构建。
构建完成后，在对应运行中下载 `ndm-next-extension` 产物。桌面发布附有同一标签构建的扩展 ZIP。
`extension-v*` 标签会单独发布扩展 ZIP 和校验值。[扩展构建](https://github.com/AreChen/NDM-Next/actions/workflows/extension-build.yml)

点 **打开网页资源** 可搜索和选择当前标签页已发现的图片、音视频、HLS 和 DASH。
有可用的图片、播放器封面或视频帧时，资源条目会显示缩略图。文件大小门槛用于自动接管。
在 **设置** 可选择简体中文、英文或跟随浏览器，**版本与插件** 会显示扩展、
桌面端和已安装插件的版本。

桌面端发现适用于当前网页的已安装引擎时，扩展会提供用该插件解析本页的入口。
点击后才会把网页链接交给所选插件。播放器旁的小胶囊在鼠标悬停或键盘聚焦时显示。
点 **×** 可在本页隐藏，也可以在扩展设置中关闭 **网页媒体按钮**。
