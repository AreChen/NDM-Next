# Windows installer build

Build the media tools on Linux or WSL using `../ffmpeg/build.sh` and an empty output directory. The script verifies the FFmpeg source and produces a network-disabled, shared LGPL build with its corresponding source and licenses. Its dependencies are listed at the top of the script.

Run `build.ps1 -MediaToolDir <output-directory>` on Windows with the Rust 1.100.0 x64 MSVC toolchain, NSIS 3 and `cargo-about` installed. `-Nsis` and `-CargoAbout` can point to executables outside `PATH`; use `-OutputDirectory` to keep a verification build separate from existing release files.

The build verifies the media files and configuration, checks dependency licenses, and creates an installer under `desktop/dist/` with the relevant source and license notices. The per-user installer registers the Chrome and Edge native messaging hosts. Uninstalling offers to retain or remove settings and download records; completed downloads are kept. Silent uninstall retains the records.

Desktop releases are distributed through GitHub as `desktop-v{version}`. Automatic checks run at most every 12 hours, with the last check remembered across restarts; they can be disabled in Settings. Available updates appear in the title bar. Click Upgrade to download and verify the GitHub SHA-256 digest and length, then install once transfers and confirmation dialogs finish. A remembered version hint is rechecked against GitHub before downloading after a restart. Setup closes the running installation before replacing its files and defaults to opening the app afterward. Silent `/UPDATE` installs also reopen it.

## Browser extension

Find the store links in **Settings → Browser extension**:

- [Chrome / Brave](https://chromewebstore.google.com/detail/ndm-next/caegebfdojcdlkkiaiokeoadefdooamd)
- [Microsoft Edge](https://microsoftedge.microsoft.com/addons/detail/ndm-next/hiblommiheggidolemmnnlocmocebckp)

The store pages may not be available yet; use manual installation below if needed. Store-installed extensions are already allowed by the desktop installer. Choose **NDM Next** in the extension settings when migrating from Neat Download Manager.

Download the companion extension ZIP from the desktop GitHub release and extract it. Load the extracted folder as an unpacked extension in the browser's developer mode. The repository's `extension/` directory works too. Allow its displayed extension ID in the installed host:

```powershell
pwsh -File .\enable-dev-extension.ps1 -ExtensionId <extension-id>
```

Download `enable-dev-extension.ps1` from the same release (or use `desktop/packaging/windows/enable-dev-extension.ps1` in a source checkout). The command adds that one local extension to the native messaging manifest. Reload the extension and select **NDM Next** as its backend. Setup preserves existing valid extension IDs across upgrades, including when changing the installation directory. The existing extension store release has a separate version stream.

Update the companion extension as well as the desktop app to receive media-button fixes. Reload open webpages after updating the extension. **Settings → Buttons beside media** hides the page buttons immediately; detected media remains available in the popup. Complete file sizes appear when the server supplies them; playlist and segment lengths are not presented as a whole video's size.

## Release checklist

- Run the Rust workspace suite, media integration tests, extension tests, formatting and clippy checks.
- Build the installer with the audited media tools. Publish its corresponding source and licenses with the installer (the build includes them).
- Publish the installer, companion extension, browser-integration helper and `SHA256SUMS` at a stable `desktop-v{version}` GitHub release. Keep incomplete uploads in a draft.
- Verify the published installer size and SHA-256 digest against the local build before considering the release ready for automatic updates.

## Windows signing

The current GitHub installer is unsigned. Windows may show “Windows protected your PC” or “Unknown publisher”. A Microsoft Store developer account does not sign an EXE distributed through GitHub, and a self-signed certificate only works on machines that explicitly trust it.

For public releases, obtain a publicly trusted code-signing certificate or an eligible signing service. Integrate signing and verification of the desktop executable and native host after release compilation but before NSIS packages them; then sign, timestamp, and verify the finished installer before computing release checksums and uploading assets. Keep the same publisher identity across releases. The current build script does not perform these signing steps.

Signing establishes the publisher's identity; new signed downloads can still trigger SmartScreen until reputation builds. See Microsoft's [code signing options](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/code-signing-options), including the SignPath Foundation option for eligible open-source projects.

The GitHub Actions installer workflow builds from source on GitHub-hosted runners and uploads an unsigned artifact for build verification. It does not publish a release or submit a signing request.
