# Windows installer build

Build the media tools on Linux or WSL using `../ffmpeg/build.sh` and an empty output directory. The script verifies the FFmpeg source and produces a network-disabled, shared LGPL build with its corresponding source and licenses. Its dependencies are listed at the top of the script.

Run `build.ps1 -MediaToolDir <output-directory>` on Windows with the Rust 1.100.0 x64 MSVC toolchain, NSIS 3 and `cargo-about` installed. `-Nsis` and `-CargoAbout` can point to executables outside `PATH`.

The build verifies the media files and configuration, checks dependency licenses, and creates an installer under `desktop/dist/` with the relevant source and license notices. The per-user installer registers the Chrome and Edge native messaging hosts. Uninstalling offers to retain or remove settings and download records; completed downloads are kept. Silent uninstall retains the records.

The installer is unsigned until a release signing process is configured. This script does not create an MSIX or submit an app to the Microsoft Store.
