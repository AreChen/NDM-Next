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
