# Windows installer build

Run `build.ps1` on Windows with the Rust 1.100.0 x64 MSVC toolchain, NSIS 3 and `cargo-about` installed. The `-Nsis` and `-CargoAbout` parameters can point to executables outside `PATH`.

The script builds both programs, checks dependency licenses against `about.toml`, generates the third-party notices and creates an installer under `desktop/dist/`. It includes the project's license, the generated notices and the separate Lucide/Feather icon license. The generated notices link to the exact published crate sources, including the unmodified MPL-2.0 component.

This installer is per-user and is unsigned until a release signing process is configured. It registers the Chrome and Edge extension IDs for native messaging and retains download data on uninstall. An MSIX/Store package is a separate build and is not produced by this script.
