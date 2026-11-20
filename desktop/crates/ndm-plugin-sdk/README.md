# NDM Next download engine SDK

NDM launches the installed engine executable with `--ndm-engine` for one
download, with the engine's installation directory as its current directory.
The engine communicates with NDM over stdin and stdout. Each message is one
UTF-8 JSON object followed by a newline. The message limit is 4 MiB, excluding
the newline. Reserve stdout for protocol messages.

The Rust crate provides the message types and synchronous framing helpers.
The JSON format allows engines written in other languages to use the same
interface.

| Direction | Message | Meaning |
| --- | --- | --- |
| NDM to engine | `{"type":"hello","version":1}` | Negotiate the protocol version. |
| Engine to NDM | `{"type":"ready","version":1}` | Accept that version. Wait for a download before starting work or launching subprocesses. |
| NDM to engine | `{"type":"download","request":{…}}` | Start work for the given task ID. |
| Engine to NDM | `{"type":"progress","progress":{"downloaded":42,"total":null,"transferred":42}}` | Report byte counts. Use `null` for the total if it is unknown. |
| NDM to engine | `{"type":"stop"}` | Stop work and persist a checkpoint if the engine can resume. |
| Engine to NDM | `{"type":"stopped"}` | Confirm that work has stopped and any checkpoint is ready to use. |
| Engine to NDM | `{"type":"completed","files":["video.mp4"]}` | Report finished regular files under the task workspace, with each file closed and ready to deliver. Use slash-separated relative paths. |
| Engine to NDM | `{"type":"failed","reason":"network"}` | Return a failure category. |

The request carries the task ID, absolute workspace path, source and
plugin-specific options. A URL source is `{"kind":"url","url":"…","headers":{}}`.
A file source is `{"kind":"file","path":"…"}`. Optional hints include
`suggested_filename`, `expected_size`, and `download_threads`. Keep credentials
out of diagnostics. Reuse a checkpoint from the same workspace and task ID
when the source and options match.

An engine should respond to `stop` while a transfer is active. Once it sends
a terminal message, NDM reclaims the worker and its child processes. Finish
subprocess work and flush any data needed for completion or resume before that
message. Give subprocesses their own redirected streams so the engine owns
the protocol pipes. If the control input closes, stop the transfer, clean up
its subprocesses and exit.

## Package manifest

Distribute a ZIP with `plugin.json` at the root. Paths use forward slashes.
An engine can include sibling tools and other resources. A market archive
targets one OS and architecture and carries a SHA-256 checksum. The ZIP and
manifest are both checked before installation.

- `schema` and `engine_protocol` are both `1`. Set a stable lowercase
  `id`, a semantic `version`, a relative `engine` path, and localized
  `name` and `description` objects. Use `en` and optionally `zh`.
- `inputs` is a list of matchers. Each matcher has `schemes` and can
  narrow matching with `hosts` and `suffixes`. A domain matches itself
  and its subdomains. A local file uses the `file` scheme. A saved site
  choice takes precedence over automatic matching.
- `limits.active_tasks` sets the engine's maximum simultaneous tasks.
  `limits.download_threads` is an optional per-task cap. The passed hint
  already includes the user's chosen setting and the declared cap. The
  engine defines how the hint maps to connections, fragments, or peers.
- `options` declares the engine's arguments. A field has a `key`,
  localized `label`, and `kind` of `boolean`, `integer`, `text`,
  or `choice`. Integer fields set `min` and `max`. Text fields set
  `max_length`. Each choice has a `value` and localized `label`.
  Fields can declare a `default` or be marked `required`.
- `tools` can expose named MCP download actions. Each has a stable `id`,
  localized `name` and `description`, and optional preset `defaults` for
  the options. NDM registers them while AI integration is enabled.

## JavaScript panels

The optional `panel` path points to a JavaScript file with a
`panel({schema, scheme, locale})` function. It returns an object with a
`fields` array. Each entry names an option with `key` and can override its
localized `label`. The script can order fields and hide optional ones.

GPUI renders the native controls from that object. The script runs when the
form is prepared, with bounded execution time and memory and no filesystem
or network APIs. NDM uses the manifest controls if the script fails.

## Rust template and contract tests

Start with [examples/file-copy.rs](examples/file-copy.rs) and
[examples/file-copy.json](examples/file-copy.json). Build the example and
package the matching platform executable and manifest at the ZIP root.
Use the `.exe` suffix in the manifest on Windows. In NDM Next, choose
**Plugin market → Import plugin**, then add a local `.ndm-example` file.

From the `desktop` directory:

```sh
cargo +1.100.0 build --locked -p ndm-plugin-sdk --example file-copy
cargo +1.100.0 test --locked -p ndm-core --test plugin_engine
```

The contract suite checks launch, progress, cancellation, child-process
reaping, output validation, installation, routing, and scheduler limits.
The Rust `serve` helper handles the handshake, receives stop commands,
and sends a terminal result. The `NDM_MEDIA_TOOLS` environment variable
can point to the app's FFmpeg directory when available.
