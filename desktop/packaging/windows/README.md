# Windows installer build

Build the media tools on Linux or WSL using `../ffmpeg/build.sh` and an empty output directory. The script verifies the FFmpeg source and produces a network-disabled, shared LGPL build with its corresponding source and licenses. Its dependencies are listed at the top of the script.

The bundled tools can remux compatible streams to MP4, MKV, or WebM.

Run `build.ps1 -MediaToolDir <output-directory>` on Windows with the Rust 1.100.0 x64 MSVC toolchain, NSIS 3 and `cargo-about` installed. `-Nsis` and `-CargoAbout` can point to executables outside `PATH`; use `-OutputDirectory` to keep a verification build separate from existing release files.

The build verifies the media files and configuration, checks dependency licenses, and creates an installer under `desktop/dist/` with the relevant source and license notices. The per-user installer registers the Chrome and Edge native messaging hosts. Uninstalling offers to retain or remove settings and download records; completed downloads are kept. Silent uninstall retains the records.

Desktop releases are distributed through GitHub as `desktop-v{version}`. Automatic checks run at most every 12 hours, with the last check remembered across restarts; they can be disabled in Settings. Available updates appear in the title bar. Click Upgrade to download and verify the GitHub SHA-256 digest and length, then install once transfers and confirmation dialogs finish. A remembered version hint is rechecked against GitHub before downloading after a restart. Setup closes the running installation before replacing its files and defaults to opening the app afterward. Silent `/UPDATE` installs also reopen it.

## Browser extension

Find the store links in **Settings → Browser extension**:

- [Chrome / Brave](https://chromewebstore.google.com/detail/ndm-next/caegebfdojcdlkkiaiokeoadefdooamd)
- [Microsoft Edge](https://microsoftedge.microsoft.com/addons/detail/ndm-next/hiblommiheggidolemmnnlocmocebckp)

The store pages may not be available yet; use manual installation below if needed. Store-installed extensions are already allowed by the desktop installer. Choose **OnesDM** in the extension settings when migrating from Neat Download Manager.

Download the companion extension ZIP from the same desktop GitHub Release and extract it. Load the extracted folder as an unpacked extension in the browser's developer mode. The repository's `extension/` directory works too. Starting with extension 1.1.4, the fixed ID is already authorized by desktop 0.1.9 and later. See the [installation guide](../../../docs/extension-install.md). Only older or custom builds without that fixed ID need the helper:

```powershell
pwsh -File .\enable-dev-extension.ps1 -ExtensionId <extension-id>
```

Use `enable-dev-extension.ps1` inside the extension ZIP (or `desktop/packaging/windows/enable-dev-extension.ps1` in a source checkout). The command adds that one local extension to the native messaging manifest. Reload the extension and select **OnesDM** as its backend. Setup preserves existing valid extension IDs across upgrades, including when changing the installation directory. The existing extension store release has a separate version stream.

Update the companion extension as well as the desktop app to receive media-button fixes. Reload open webpages after updating the extension. Media buttons appear above the player where space allows, with a viewport-clamped fallback near its top edge. A non-modal top-layer host avoids ordinary page stacking and clipping; controls stay within their own iframe. **Settings → Buttons beside media** hides the page buttons immediately; detected media remains available in the popup. Complete file sizes appear when the server supplies them; playlist and segment lengths are not presented as a whole video's size.

The popup's **Open media library ↗** button opens a separate resource window bound to that source tab. Search decoded filenames or source hosts, filter video/audio/HLS/DASH, and select resources for batch download. Reopening focuses the existing window. Navigating or closing the source tab clears stale selections; browser restart requires capturing the media again. Recognition covers observed audio/video responses and common media extensions served with generic content types. This is not a site-specific extractor, and it does not decrypt DRM-protected media. Downloading a resource explicitly activates its desktop progress window. Page-button success notices reset after 1.2 seconds.

## Download proxy

Saving a proxy does not activate it. Enable **Proxy for new downloads** and save to apply it by default; otherwise downloads retain the system network settings. The lightning button in a download window restarts only that HTTP(S), HLS or DASH task through the saved proxy, retaining supported resume data and cumulative statistics. Other tasks and the default setting are unchanged. The task keeps its proxy through pause, resume and application restart. FTP(S) and SFTP do not use this HTTP proxy control. Proxy credentials remain encrypted in preferences and task storage.

## Release checklist

- Run the Rust workspace suite, media integration tests, extension tests, formatting and clippy checks.
- Update the workspace version and push a matching `desktop-v{version}` tag. The [release workflow](../../../.github/workflows/release.yml) builds the Windows installer and both macOS DMGs on GitHub, packages the matching extension from the same commit, and publishes the complete draft after all builds and tests succeed. Release assets include the installer, DMGs, companion ZIP (with the installation guide and browser-integration helper), and `SHA256SUMS`. The desktop packages include the corresponding media source and license notices.
- Write every release announcement in both **English and 简体中文**, with matching changes and installation instructions in each language.
- Verify successful build runs and compare published package sizes and SHA-256 digests with their GitHub build artifacts before considering the release ready for automatic updates.

## Windows signing

The current GitHub installer is unsigned. Windows may show “Windows protected your PC” or “Unknown publisher”. A Microsoft Store developer account does not sign an EXE distributed through GitHub, and a self-signed certificate only works on machines that explicitly trust it.

For public releases, obtain a publicly trusted code-signing certificate or an eligible signing service. Integrate signing and verification of the desktop executable and native host after release compilation but before NSIS packages them; then sign, timestamp, and verify the finished installer before computing release checksums and uploading assets. Keep the same publisher identity across releases. The current build script does not perform these signing steps.

Signing establishes the publisher's identity; new signed downloads can still trigger SmartScreen until reputation builds. See Microsoft's [code signing options](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/code-signing-options), including the SignPath Foundation option for eligible open-source projects.

The GitHub Actions installer workflow builds from source on GitHub-hosted runners and uploads an unsigned artifact. The desktop release workflow calls it together with the macOS workflow and publishes the resulting packages; it does not submit a signing request.
