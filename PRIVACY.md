# Privacy Policy for NDM Next

Effective date: November 15, 2026

NDM Next connects eligible browser downloads and selected media links to the NDM Next desktop application or Neat Download Manager running on your computer.

## Information used

- Download URLs, suggested filenames, file sizes, MIME types, and referring page URLs are used to apply your filters and send tasks to your selected local download manager.
- The active page URL and network response metadata are used to apply domain rules and identify media resources. Optional page buttons read audio/video source URLs to match media already observed by the extension.
- Matching request headers, including cookies and Authorization when available, are used to download files that require your signed-in session. While NDM Next interception is enabled, the extension observes HTTP(S) request URLs and headers across websites and temporarily caches permitted GET request headers locally before matching them to downloads or media. For a handoff, the selected request information is sent to the local `com.ndmnext.host` native messaging host. With the Neat Download Manager backend, matching cookies and download information are sent to its local WebSocket service at `127.0.0.1:10007`.
- The NDM Next desktop app encrypts and stores the download request in its local task database for recovery. This can include the download URL and request headers containing cookies or Authorization; task records can be deleted from the app.
- Extension preferences, including blocked domains, file types, size threshold, backend, and language, are stored in browser local extension storage. Non-secret task identifiers and handoff state are also stored there to recover interrupted handoffs; captured cookies and Authorization headers are not written to persistent browser extension storage. Pending confirmations and detected media, including their permitted replay headers, may be temporarily kept in memory-only browser session storage so they survive a service-worker restart. This session cache is restricted to trusted extension contexts; page controls receive media metadata without replay headers.
- Notifications may request confirmation before handing off a download. Browser alarms retry pending local handoffs after an interruption.

The extension does not send these data to its developer or use analytics. The legacy Neat Download Manager backend may make a HEAD request to a download URL to check its size; the destination website receives that request and applicable browser credentials. Your selected desktop application downloads from the URLs you request. The NDM Next desktop application's updater contacts GitHub to check for and download releases; automatic updates can be disabled in desktop settings. Neat Download Manager, GitHub, and download websites have their own data practices.

## Retention and control

Preferences remain in browser storage until you change them or remove the extension. Handoff recovery records are removed once the handoff is resolved. The request-matching cache is cleared within two minutes or when the background worker stops. A pending confirmation has a five-minute approval window; its memory-only session data is removed when answered or dismissed, or when the browser session ends. Media entries expire after 30 minutes and are removed on page navigation, tab closure, disabling interception or media detection, or ending the browser session. The media cache is bounded to 50 entries per tab and at most 500 entries / 4 MiB in session storage. You can hide page download buttons without disabling media detection. The desktop application retains its local task records for task history and resuming downloads; these can be removed in the desktop application. You can disable interception or media detection in extension settings and uninstall the extension using your browser's extension manager.

## Contact

For questions, open an issue at https://github.com/AreChen/NDM-Next/issues. Do not include cookies, credentials, or private download URLs in a public issue.
