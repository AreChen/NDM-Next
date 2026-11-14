# Compact library and protocol completion implementation plan

> Agent workers: execute the existing approved design and the user's explicit refinements. Use independent parallel investigations/implementations for disjoint subsystems; integrate and review sequentially.

**Goal:** deliver a compact searchable download library and progress window, grouped settings and stronger protocol/media support.

**Architecture:** retain GPUI Kit, task storage and scheduler. Add a pure indexed task-library helper and media helpers; compose these into the existing desktop view. Keep unsafe protocol fallbacks explicit.

**Stack:** Rust 1.100, GPUI Kit 0.9.1, Tokio, existing TLS/SFTP libraries and audited local FFmpeg tools.

**Specification:** ../specs/2026-11-14-compact-library-design.md

## Global constraints

- Publishing and signing are deferred. Preserve all pre-existing edits.
- No browser/computer automation. All external commands use RTK.
- No fabricated completion, media progress, integrity guarantees or performance comparisons.
- Credentials and cookies remain protected and scoped to their origin.

## Review focus

- Unicode normalization and encoded filenames must match consistently without rebuilding the index on every paint.
- Changed FTP/SFTP content must not be appended to old partial content.
- MPD URL resolution, size limits and DRM/unsupported formats must not create corrupt completed outputs.
- 500px-high windows must keep primary actions visible and details/settings scrollable.
- Completed/failed tasks must retain their file operations and clear status in both themes/languages.

## Tasks

- [x] Add `ndm-desktop/src/library.rs` with indexed Unicode search, categories and sorting; use behavioral tests first.
- [x] Improve FTP/FTPS/SFTP continuation in the core and controlled-server tests; preserve authentication/security behavior.
- [x] Add DASH media handling and measurable local remux progress; test actual generated media and failure cases.
- [x] Integrate search/category/sort controls into `view.rs`, compact state-gradient rows and collapsed settings sections.
- [x] Resize the main/progress windows and replace oversized progress metrics with compact button chips.
- [x] Integrate media choices and honest protocol status in desktop/extension paths.
- [x] Run targeted then full Rust/extension validation, review changes, run CLI HTTP/GitHub smoke tests and measure a local release build.
- [x] Build a refreshed local installer and document measured results and remaining real-device/visual acceptance.
