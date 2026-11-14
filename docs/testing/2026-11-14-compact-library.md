# Compact library verification — 2026-11-14

This is a local development build. Publishing, signing and store submission remain deferred. No browser or computer automation was used; the user performs visual acceptance.

## Delivered behavior

- Main window: 1000 × 500 logical pixels, compact horizontal task rows, status gradients, virtualized list and collapsible sidebar.
- Search: Unicode compatibility normalization and full case folding, decoded URL names, indexed substring matching with AND terms. Automatic file categories and recent/name/size sorting. Completed sizes are cached outside painting; running downloads can supply discovered totals without changing integrity metadata.
- Progress window: 480 × 168 while active, 480 × 136 when finished, expandable details; 100 ms speed samples with a rolling average. File assembly measures copied bytes; media remux measures tool-reported timestamps with an estimated remaining time. Unknown totals use an indeterminate indicator.
- Settings: compact appearance/language, download, proxy, startup and update groups; initially collapsed. Connection count explicitly applies to HTTP(S).
- FTP/FTPS: persistent partial data, safe full-file SHA-256 + REST continuation when the server supports it, verified-prefix fallback otherwise. SFTP preserves partial data and validates the remote prefix on the same handle before reusing it. Cancellation and changed-content tests cover both paths.
- Static fMP4 DASH: quality and audio selection, one bitrate per adaptation group, up to eight audio tracks, scoped credentials, validated segment recovery, local remux and atomic publication. The extension does not compare HLS/DASH manifest length against the final-media size threshold.

## Verification

Toolchain: Rust 1.100.0, Windows x64. Media tests use `NDM_TEST_FFMPEG_DIR` pointing at the audited, network-disabled local tool bundle.

- Workspace tests ran with `--include-ignored --skip real_http_download_follows_redirects_and_publishes_complete_output`: 164 passed, zero failed, including real generated-media remux fixtures. The real-network test ran separately and passed.
- The 38 desktop behavioral tests cover Unicode matching, classification, actual/discovered size sorting, safe single/batch deletion, audio-group selection and maximum track count.
- Extension: `node --test tests` — 42 passed, zero failed.
- Formatting, workspace clippy with warnings denied, and whitespace validation passed.
- Independent source review covered the compact UI, continuation, manifest/segment handling, local remux, credential scope and publication paths.
- A review regression confirmed and fixed false DRM rejection of clear MP4 metadata containing words such as `essence` or `latency`. The final implementation inspects bounded ISO-BMFF boxes/sample entries and keeps free-text/configuration payloads opaque.

All external commands were invoked through RTK. The packaging script was copied to a temporary directory with RTK wrappers and explicit source paths; the repository's general-purpose build script was preserved.

## Real GitHub smoke test

An explicit opt-in test downloaded `AreChen/NDM-Next`'s `main` archive through the configured local proxy. GitHub redirected to codeload over HTTPS. The final response contained neither Content-Length nor Content-Range. The core published `NDM-Next-main.zip` without inventing an expected size.

- Actual length: 1,494,906 bytes.
- SHA-256: `f47bd69a16336fbb8de7b7165267beb3187c81d5019b925474f7fd12b8f7b246`.
- Separate Python ZIP validation: 23 entries / 18 files, all CRCs valid, no parent-traversal entry.
- One measured core transfer took 1,244 ms; this is one archive/proxy observation, not a comparative speed benchmark.

## Remaining acceptance and limits

- Visual appearance, scaling and keyboard behavior on the user's actual displays require manual acceptance. No screenshot-based claim is made.
- No same-condition memory, CPU or throughput comparison against NeatDM has been performed for this build.
- SFTP and FTP without a usable full-file SHA-256 reread retained bytes for validation; they reuse disk content but do not avoid that validation traffic. These protocols remain single-connection.
- DASH supports bounded static ISO-BMFF/fMP4 manifests. DRM, dynamic/live DASH, multiple periods, SegmentBase, WebM and complex timing remain explicitly unsupported. Same-source manifest changes produce a recovery conflict; replacing the requested URL safely restarts owned staging files.
- Immediate browser takeover retains the previously accepted risk for one-use URLs. The GitHub check verifies this archive snapshot, not every website.

## Local artifacts

The optimized installer was rebuilt after the final DRM parser fix. These files remain local; no application was installed, restarted or published during verification.

| File in `desktop/dist` | Bytes | SHA-256 |
| --- | ---: | --- |
| `NDM-Next-Setup-0.1.0-x64-compact.exe` | 28,875,281 | `1399efa2dce0c58ecb7756c5f10d1ba3097eb1c077e95916bf276b9472351106` |
| `NDM-Next-Extension-local.zip` | 48,852 | `5ced8c5ad088204a6d9512b6f6d4d468baa38060d11e4c69b277e2362d2485e4` |

Checksums are also saved as `desktop/dist/SHA256SUMS-local.txt`. Install the desktop update and reload the updated extension together when performing manual acceptance.
