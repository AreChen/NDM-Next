# GitHub desktop release 0.1.0

The user explicitly authorizes fixing the compact UI, adding the existing project logo, automatic GitHub installer updates, automatically closing the app for installation, launching it after installation, merging into `main`, and publishing desktop 0.1.0. Browser/computer automation remains prohibited.

## Design and interfaces

- Keep the 500px compact layout. Give expanded settings 12px padding on all sides. Keep sidebar toggle centered at a fixed size when collapsed. Display the existing PNG logo in the custom title bar and expanded sidebar, while preserving bundled kit icons.
- Default automatic updates on, with a saved opt-out. Check GitHub at startup and periodically. Accept stable `desktop-v*` releases with the exact Windows installer asset; authenticate transport with HTTPS and verify the published SHA-256 digest and byte length. Download in the background. Install only when the scheduler has no active transfers; atomically reserve task admission before the final idle check, retain that reservation through verification/installer startup, and shut down gracefully before replacing files.
- The installer owns a per-profile installation guard, asks the desktop to shut down through a named event, waits for exit, and closes only installation-owned legacy/native-host processes when needed. Never terminate arbitrary processes by executable name. Finish defaults to launching the app; silent `/UPDATE` installation also relaunches it.
- Preserve existing task storage, credentials and all previous development work. Keep the managed worktree and its artifacts. Publish `desktop-v0.1.0` so extension `v1.0.0` is an independent version stream.

## Pre-flight

- Update state consumes preferences (automatic updates and proxy) and scheduler idleness; installer launch delegates graceful shutdown to the installation IPC.
- Installation IPC is shared by core, desktop instance and host helper. The desktop main wires the shutdown callback to the existing graceful Quit command.
- Release discovery, packaging and published assets must agree on `NDM-Next-Setup-{version}-x64.exe` and its SHA-256 digest.
- Branding assets must fall back to kit assets so existing icons remain available.
- Ruling: main/release authorization is already explicit in the user's request; no integration-choice menu is needed. The app-managed worktree remains available for ongoing development.

## Tasks

- [x] UI spacing, toggle alignment and logo asset integration; asset loading regression.
- [x] GitHub release discovery/download/hash validation, periodic automatic updates and bilingual controls; behavioral and local-server tests.
- [x] Graceful installer shutdown, installation guard and launch-after-install behavior; isolated process/installer tests.
- [x] Fresh-context final review, full Rust/extension checks and optimized installer build.
- [x] Commit the desktop work, merge into main while preserving unrelated main-worktree files, verify the merged tree, push, publish release assets, and verify the public release/digests.

## Review focus

Reject untrusted update URLs, wrong assets, overlarge or truncated responses, bad digests and partial installers. Do not install during a transfer or media merge. Do not kill an unrelated executable or lose download checkpoints. Never launch an installer before verification. Prevent overlapping installs and native-host startup during file replacement. Preserve both themes/languages and existing file/delete operations. Publication must contain the exact merged source and tested artifacts.
