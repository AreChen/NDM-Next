# Windows installer build

Build the media tools on Linux or WSL using `../ffmpeg/build.sh` and an empty output directory. The script verifies the FFmpeg source and produces a network-disabled, shared LGPL build with its corresponding source and licenses. Its dependencies are listed at the top of the script.

Run `build.ps1 -MediaToolDir <output-directory>` on Windows with the Rust 1.100.0 x64 MSVC toolchain, NSIS 3 and `cargo-about` installed. `-Nsis` and `-CargoAbout` can point to executables outside `PATH`.

The build verifies the media files and configuration, checks dependency licenses, and creates an installer under `desktop/dist/` with the relevant source and license notices. The per-user installer registers the Chrome and Edge native messaging hosts. Uninstalling offers to retain or remove settings and download records; completed downloads are kept. Silent uninstall retains the records.

The installer is unsigned until a release signing process is configured. This script does not create an MSIX or submit an app to the Microsoft Store.

## Try the unreleased browser integration

The current store extension does not yet contain the new desktop backend. To test locally, load the repository's `extension/` directory as an unpacked extension in developer mode, then allow its displayed extension ID in the installed host:

```powershell
pwsh -File desktop/packaging/windows/enable-dev-extension.ps1 -ExtensionId <extension-id>
```

The command adds that one local extension to the native messaging manifest. Reload the extension and select **NDM Next** as its backend. Re-run the command after reinstalling the desktop app, since installation resets the list to the official store IDs.

## Microsoft Store

MSIX browser integration remains unresolved. Edge and Chrome discover a native host through a registry entry, while MSIX normally isolates registry writes from other applications. A packaged execution alias was verified to support Native Messaging, but it does not make that registry entry visible. Microsoft's [unvirtualized resources](https://learn.microsoft.com/en-us/windows/msix/desktop/flexible-virtualization) capability requires restricted-capability approval and leaves those writes behind after uninstall, so it is not included as a finished Store solution. An [unpackaged Store submission](https://learn.microsoft.com/en-us/windows/apps/distribute-through-store/how-to-distribute-your-win32-app-through-microsoft-store) is a different distribution option and requires publisher signing.
