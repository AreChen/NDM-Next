# Windows installer build

Build the media tools on Linux or WSL using `../ffmpeg/build.sh` and an empty output directory. The script verifies the FFmpeg source and produces a network-disabled, shared LGPL build with its corresponding source and licenses. Its dependencies are listed at the top of the script.

Run `build.ps1 -MediaToolDir <output-directory>` on Windows with the Rust 1.100.0 x64 MSVC toolchain, NSIS 3 and `cargo-about` installed. `-Nsis` and `-CargoAbout` can point to executables outside `PATH`.

The build verifies the media files and configuration, checks dependency licenses, and creates an installer under `desktop/dist/` with the relevant source and license notices. The per-user installer registers the Chrome and Edge native messaging hosts. Uninstalling offers to retain or remove settings and download records; completed downloads are kept. Silent uninstall retains the records.

Desktop releases are distributed through GitHub as `desktop-v{version}`. The Windows asset is `NDM-Next-Setup-{version}-x64.exe`; the updater verifies its GitHub SHA-256 asset digest and length, then runs it silently once transfers are idle. Automatic updates can be disabled in Settings. Setup closes the running installation before replacing its files and defaults to opening the app afterward. Silent `/UPDATE` installs also reopen it.

## Browser extension

Download the companion extension ZIP from the desktop GitHub release and extract it. Load the extracted folder as an unpacked extension in the browser's developer mode. The repository's `extension/` directory works too. Allow its displayed extension ID in the installed host:

```powershell
pwsh -File .\enable-dev-extension.ps1 -ExtensionId <extension-id>
```

Download `enable-dev-extension.ps1` from the same release (or use `desktop/packaging/windows/enable-dev-extension.ps1` in a source checkout). The command adds that one local extension to the native messaging manifest. Reload the extension and select **NDM Next** as its backend. The installer preserves existing valid extension IDs for the same installation. The existing extension store release has a separate version stream.

## Release checklist

- Run the Rust workspace suite, media integration tests, extension tests, formatting and clippy checks.
- Build the installer with the audited media tools. Publish its corresponding source and licenses with the installer (the build includes them).
- Publish the installer, companion extension, browser-integration helper and `SHA256SUMS` at a stable `desktop-v{version}` GitHub release. Keep incomplete uploads in a draft.
- Verify the published installer size and SHA-256 digest against the local build before considering the release ready for automatic updates.
