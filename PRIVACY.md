# Privacy Policy for NDM Next

Effective date: November 13, 2026

NDM Next is a browser extension that sends eligible downloads and selected media links to the Neat Download Manager application running on your computer.

## Information used

- Download URLs, file sizes, MIME types, and referring page URLs are used to apply your filters and send tasks to Neat Download Manager.
- The active page URL and network response metadata are used to apply domain rules and identify media resources.
- Cookies associated with a download URL are read when sending a task to Neat Download Manager, so it can access downloads requiring a signed-in session. URLs, referring page information, and matching cookies are sent to the local service at `127.0.0.1:10007`.
- Your extension preferences, including blocked domains, file types, size threshold, and language, are stored in browser local extension storage. Detected media entries are temporarily kept in memory.

NDM Next does not send these data to its developer or use analytics. It may make a HEAD request to the download URL to check the file size; the destination website receives that request and applicable browser credentials. Neat Download Manager and the websites you visit have their own data practices, which this policy does not control.

## Retention and control

Preferences remain in browser storage until you change them or remove the extension. The temporary media list is held in memory and can be cleared when the extension's background worker stops. You can disable download interception or media detection in the extension settings and uninstall the extension using your browser's extension manager.

## Contact

For questions, open an issue at https://github.com/AreChen/NDM-Next/issues. Do not include cookies, credentials, or private download URLs in a public issue.
