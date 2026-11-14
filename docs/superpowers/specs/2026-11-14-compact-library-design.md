# Compact download library and protocol completion

This change follows the user's approved desktop design, new screenshots and requested refinements. Publishing, signing and store submission are deferred. Browser and computer automation remain disabled; the user performs visual acceptance.

## User-facing design

- Main window defaults to 1000 × 500 logical pixels, with a minimum allowing a useful list at common display scales. Retain the collapsible sidebar and custom title bar.
- Each task is one compact horizontal row. Filename/source can share a small identity block, but state, progress and actions must not become separate full-width rows. Running, completed, paused and failed states use subtle purple/blue, teal, amber and red gradients plus accessible text or icons.
- Keep controls compact: an inline search field, file-type filter, sort control and a plus button. Search covers displayed filename, URL and source host; normalize Unicode and case, decode URL names and intersect an in-memory index rather than rescan all task strings every frame. Categories are automatic: archives, documents, video, audio, images, applications and other. UI remains Chinese/English.
- The independent progress window uses compact metric/button chips, a small progress bar and optional scrollable details. Remove empty bottom space. Completion should not retain irrelevant speed/ETA blocks.
- Settings use compact, collapsible groups: appearance/language, download behavior, proxy/network, startup, and updates. Advanced groups are closed initially.

## Download behavior

Improve FTP/FTPS/SFTP continuation without silently joining changed content. Persist continuation metadata and use available server identity/remote content validation; if a safe continuation cannot be proved, restart clearly. Keep authentication, host-key and TLS validation intact.

Implement ordinary unencrypted static DASH downloads in the Rust core: bounded MPD parsing, correct relative URL resolution, representation/audio selection, segmented download, cancellation and safe final media publication. Reuse the audited network-disabled media tools for local remuxing. Reject DRM, dynamic/live or unsupported manifests explicitly until implemented; do not save a manifest as a completed video. Retain HLS behavior and add measurable final remux progress where source duration is known.

Real-world HTTP/GitHub smoke tests and resource measurements may use CLIs and controlled servers. They must not interact with the user's browser or claim a full Windows-version/DPI/website compatibility matrix. One-use-link risks of the user-selected immediate takeover remain explicit.

## Implementation choices

Use the existing Rust 1.100/GPUI Kit workspace, existing task storage and rendering patterns. Extract small library/presentation helpers when it simplifies view.rs. Keep current uncommitted edits intact. Parallel work is limited to disjoint search, remote-protocol and media-core files; integration and validation remain sequential.

## Acceptance

- Indexed multilingual matching and classification have behavioral tests, including Chinese, decomposed accents, full-width characters and URL-encoded filenames.
- Protocol tests exercise continuation, cancellation and resource changes; unsupported cases fail honestly.
- DASH tests exercise relative resources, separate tracks, truncation, cancellation and protected/unsupported manifests.
- Workspace tests, extension tests, formatting and clippy pass; release builds and a local installer are produced for the user to inspect.
- No claims of visual verification, store publication or NeatDM-equivalent performance without measured evidence.
