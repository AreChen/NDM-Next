# Companion extension / 配套扩展

## English

1. Install and start the desktop app from the same GitHub Release.
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

## 简体中文

1. 安装并启动同一 GitHub Release 中的桌面程序。
2. 将 `NDM-Next-Extension-Desktop-<桌面版本>-v<扩展版本>.zip` 解压到固定目录。
3. 打开 `chrome://extensions`（Edge 为 `edge://extensions`），开启开发者模式，点击**加载已解压的扩展程序**，选择包含 `manifest.json` 的目录。
4. 打开扩展设置，将下载管理器设为 **NDM Next**。符合规则的下载将自动接管；默认小于 1 MB 的文件仍由浏览器下载。

macOS 首次保存下载时，请允许系统请求的下载目录访问权限。未处理系统提示时，保存过程会等待授权。

Linux 用户从解压目录启动桌面应用一次，即可注册原生宿主。参见 [Linux 安装指南](../desktop/packaging/linux/README.md#browser-extension)。

从桌面版 0.1.9 / 扩展 1.1.4 开始，Release 扩展使用固定 ID `cpapmfpgmpbpnjgibdgkfkdeblamiodm`。Windows、macOS 和 Linux 的原生宿主均预先授权此 ID。manifest 中的公钥 `key` 用于保持不同安装目录下的 ID 一致，不是私钥或签名凭据。商店版继续使用各自的商店 ID。

首次从旧版未固定 ID 的扩展升级，请移除旧副本，再加载新版目录。旧 ID 的扩展设置不会自动迁移，请检查新版设置；只保留一个启用的副本，避免重复接管。以后的 ZIP 更新可以覆盖原目录文件，然后在扩展管理页点击**重新加载**并刷新已打开的网页。

macOS 或 Linux 若要继续使用旧扩展，可复制扩展 ID，粘贴到 **NDM Next → 设置 → 浏览器扩展 → 修复连接**。Windows 自定义开发者 ID 可使用 ZIP 内的 `enable-dev-extension.ps1`。

桌面自动更新只更新桌面程序。解压加载的扩展需手动下载对应 Release ZIP 更新。
