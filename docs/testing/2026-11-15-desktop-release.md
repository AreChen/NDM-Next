# Desktop 0.1.0 release validation

## Scope

Settings expansion now uses equal 12px body padding; the collapsed sidebar toggle is centered. The project PNG appears in the title bars, sidebar and About panel, with bundled kit icons retained. The default 500px main window and compact progress windows remain.

GitHub updates download and verify the exact versioned installer, then reserve task admission before checking idleness. The reservation stays alive through cached-file verification and installer startup. Installation closes the registered application gracefully, preserves task storage, and relaunches after releasing its installation guard.

## Verification before merging

- Rust 1.100.0 workspace: **187 passed**, no failures. Local FFmpeg media integration cases included.
- Browser extension: **42 passed**, no failures.
- `cargo +1.100.0 fmt --all` and `git diff --check`: clean.
- `cargo +1.100.0 clippy --workspace --all-targets --locked -- -D warnings`: passed.
- Optimized Windows x64 installer built with the checked media distribution and generated third-party notices.
- Companion extension ZIP: 12 entries, all CRC checks passed; `manifest.json` is at the archive root.

The workspace test command used `--include-ignored` and excluded four separately controlled cases: the real HTTP download smoke, the not-yet-published GitHub update smoke, and the `shutdown::fixture_process` / `update_admission_fixture` child-process entry points. The real GitHub archive download and ZIP integrity check passed in the previous validation report.

## Review fixes and regression evidence

One fresh-context release review found two important issues. Both were reproduced with failing behavioral tests before the fix:

- **Idle transition:** an update could accept another commit/resume while rechecking the installer. Store admissions and update reservation now share one profile mutex. Cross-process commits, resumes, task start and recovery cannot bypass the reservation; releasing it reopens admission. Concurrent reservations have one winner. A separate desktop test first reproduced startup bypassing the reservation, then passed after singleton registration joined the same boundary.
- **Relocation:** changing the installation directory lost the previous unpacked extension ID. Setup now imports validated origins from the identified old installation as well as the target manifest. Wrong host names, paths, types and arbitrary manifest sources are rejected.

The installer lifecycle tests used unique temporary profiles, registry keys and fixture executables. They passed normal launch/replacement, overlapping-install rejection, native-host and attributable media cleanup, unrelated-process preservation, quoted legacy custom locations, relocation, data retention and uninstall. A real 30-second graceful-shutdown timeout left the waiting fixture alive, aborted installation without force-killing it, and did not relaunch.

## Artifacts

| File | Bytes | SHA-256 |
| --- | ---: | --- |
| `NDM-Next-Setup-0.1.0-x64.exe` | 28,980,846 | `9048fd466ba850779d951dab8d6ac760aa7a5781d19f73296e2f42f260209716` |
| `NDM-Next-Extension-Desktop-0.1.0.zip` | 48,852 | `5ced8c5ad088204a6d9512b6f6d4d468baa38060d11e4c69b277e2362d2485e4` |

The release also includes `enable-dev-extension.ps1` and `SHA256SUMS`.

## Published release verification

- [Desktop 0.1.0](https://github.com/AreChen/NDM-Next/releases/tag/desktop-v0.1.0) is public, stable and marked latest. The existing extension `v1.0.0` release is retained.
- The release tag resolves to tested source commit `5a3092fb77be18975ff91ac927a2b540cc3e155a`. The fast-forwarded main tree matched the feature tree, and the merged checkout again passed all 187 Rust and 42 extension tests.
- All four remote assets matched their local SHA-256 digests and byte lengths before publication.
- The application's real updater discovered the published release, followed GitHub's download redirect, downloaded all 28,980,846 installer bytes, and verified SHA-256 `9048fd466ba850779d951dab8d6ac760aa7a5781d19f73296e2f42f260209716`. The dedicated public smoke passed without executing the installer.

## Coverage boundaries

- Visual appearance, scaling and keyboard acceptance remain with the user. No browser or computer automation was used.
- Actual user installations and running downloads were not modified during verification; installer execution used fixtures.
- The pre-existing native-host cap of 16 extension origins remains. A combined source/target list beyond that cap can omit extra origins; normal default and local extension IDs are covered.
- FTP(S)/SFTP remain single-connection; static fMP4 DASH is supported, dynamic DASH and DRM are not. See the previous compact-library validation for protocol coverage.
- Authenticode reputation, Microsoft Store packaging and comparative NeatDM memory benchmarks were outside this release request.
