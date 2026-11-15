# macOS build and installation

NDM Next supports macOS 12 or newer on Apple Silicon (`arm64`) and Intel (`x86_64`). CI produces one ad-hoc signed DMG for each architecture. Ad-hoc signing supports local verification; public distribution should add an Apple Developer ID signature and notarization.

Open the matching `NDM-Next-*-macos-*.dmg`, then drag **NDM Next** to **Applications**. macOS may require **Open** in the Finder context menu on first launch because the build is not notarized. Downloads default to `~/Downloads`; download records and settings live under `~/Library/Application Support/NDM Next`, and cache data under `~/Library/Caches/com.ndmnext.desktop`. Download and cache locations can be changed in Settings. A random AES key stays in `credentials.key` with owner-only (`0600`) permissions, so the desktop and browser host share credentials without repeated Keychain dialogs.

The app registers the native messaging host for Chrome, Chromium, Brave, and Microsoft Edge under each browser's per-user `NativeMessagingHosts` directory. The manifest points to the host inside the installed app bundle. Reload the extension after installing or moving the app. Developer extensions can be allowed in Settings by using their 32-character extension ID.

## Local build

Run on the target Mac:

```sh
brew install gnupg nasm cargo-about
bash desktop/packaging/macos/build-media.sh "$TMPDIR/ndm-media"
bash desktop/packaging/macos/build.sh "$TMPDIR/ndm-media" desktop/dist
```

The media build verifies the FFmpeg archive checksum and developer signature, disables network access, and includes the corresponding source and license files. The app build verifies media checksums, builds `ndm-desktop` and `ndm-host`, creates `NDM Next.app`, signs its nested binaries ad hoc, and creates a compressed DMG.

## Browser extension

Use the same browser extension as Windows. Download and extract the source archive from the GitHub release, or clone the repository, then load its `extension/` directory as an unpacked extension. Store releases can also be used when available. Native messaging registration runs when NDM Next starts. Allow an unpacked extension's displayed ID in **Settings → Browser extension**, reload the extension, and select **NDM Next** as its backend.

## Updates

The app checks the `desktop-v{version}` GitHub release stream. A verified DMG opens in Finder. Quit NDM Next, drag the new app over the old copy, and launch it again. The app keeps download records and settings across replacement.
