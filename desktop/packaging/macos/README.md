# macOS build and installation

OnesDM supports macOS 12 or newer on Apple Silicon (`arm64`) and Intel (`x86_64`). CI produces one ad-hoc signed DMG for each architecture. Ad-hoc signing supports local verification; public distribution should add an Apple Developer ID signature and notarization.

Open the matching `OnesDM-*-macos-*.dmg`, then drag **OnesDM** to **Applications**. macOS may require **Open** in the Finder context menu on first launch because the build is not notarized. Downloads default to `~/Downloads`; download records and settings live under `~/Library/Application Support/NDM Next`, and cache data under `~/Library/Caches/com.ndmnext.desktop`. Download and cache locations can be changed in Settings. A random AES key stays in `credentials.key` with owner-only (`0600`) permissions, so the desktop and browser host share credentials without repeated Keychain dialogs.

The app registers the native messaging host for Chrome, Chromium, Brave, and Microsoft Edge under each browser's per-user `NativeMessagingHosts` directory. The manifest points to the host inside the installed app bundle. Reload the extension after installing or moving the app. Developer extensions can be allowed in Settings by using their 32-character extension ID.

## Local build

Run on the target Mac:

```sh
brew install gnupg nasm cargo-about
bash desktop/packaging/macos/build-media.sh "$TMPDIR/ndm-media"
bash desktop/packaging/macos/build.sh "$TMPDIR/ndm-media" desktop/dist
```

The media build verifies the FFmpeg archive checksum and developer signature, disables network access, and includes the corresponding source and license files. The app build verifies media checksums, builds `ndm-desktop` and `ndm-host`, creates `OnesDM.app`, signs its nested binaries ad hoc, and creates a compressed DMG.

The bundled tools can remux compatible streams to MP4, MKV, or WebM.

## Browser extension

Use the matching extension ZIP from the same GitHub Release, the repository's `extension/` directory, or the store release. Release extension 1.1.4 and later has a fixed ID that the app already authorizes. Native messaging registration runs when OnesDM starts. For older unpacked extensions, copy the ID from `chrome://extensions` into **Settings → Browser extension → Repair connection**. Reload the extension after starting the app and select **OnesDM** as its backend. See the [installation guide](../../../docs/extension-install.md).

New profiles enable startup at login and use 16 download connections. Upgrading preserves saved connection counts and the existing startup choice.

## Updates

The app checks the `desktop-v{version}` GitHub release stream. Choose **Upgrade** to download and verify the DMG. After active downloads finish, OnesDM closes, replaces the application at its installed location, and reopens. The app keeps the previous bundle until the updated main window opens. Download records and settings stay in their existing directories.

The installed application's folder must be writable for in-app replacement. When it needs a manual installation, OnesDM opens the verified DMG in Finder.
