# NDM Next download engine SDK

NDM launches the installed engine executable with `--ndm-engine` for one
operation, with the engine's installation directory as its current directory.
The engine communicates with NDM over stdin and stdout. Each message is one
UTF-8 JSON object followed by a newline. The message limit is 4 MiB, excluding
the newline. Reserve stdout for protocol messages.

The Rust crate provides the message types and synchronous framing helpers.
The JSON format allows engines written in other languages to use the same
interface.

| Direction | Message | Meaning |
| --- | --- | --- |
| NDM to engine | `{"type":"hello","version":1}` | Negotiate the protocol version. |
| Engine to NDM | `{"type":"ready","version":1}` | Accept that version. Wait for a request before starting work or launching subprocesses. |
| NDM to engine | `{"type":"download","request":{…}}` | Start work for the given task ID. |
| NDM to engine | `{"type":"inspect","request":{…}}` | Resolve a file list in a disposable workspace. Only resolve metadata. |
| Engine to NDM | `{"type":"inspected","source":{"revision":"…","files":[{"id":0,"path":"video.mp4","size":42}]}}` | Return file IDs, display paths, and sizes in bytes for an immutable source revision. |
| NDM to engine | `{"type":"share","request":{…},"output":"/saved/output"}` | Verify and share available pieces from the published file or directory. Open output with read-only access. |
| Engine to NDM | `{"type":"progress","progress":{"downloaded":42,"total":null,"transferred":42}}` | Report byte counts. Use `null` for the total if it is unknown. |
| Engine to NDM | `{"type":"task_update","update":{"kind":"title","title":"Example video"}}` | Report the resolved title once it is available. |
| Engine to NDM | `{"type":"task_update","update":{"kind":"finalizing"}}` | Report local media processing. The next byte progress message returns the task to downloading. |
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

Send `task_update` messages and use the `invalid_options` and `post_processing`
failure categories only when `hello` includes `"task_updates":true`. The Rust
`Worker::update_task` and `serve` helpers handle this for older hosts.

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
- `inputs` is a list of automatic matchers. Each matcher has `schemes`
  and can narrow matching with `hosts`, `suffixes` and `exclude_suffixes`.
  A domain matches itself and its subdomains. A local file uses the `file`
  scheme. A saved site choice takes precedence among automatic matches.
- `manual_inputs` uses the same matchers. These sources can use the engine
  when selected in the download form or specified by an MCP caller. For
  example, an engine can offer general HTTP(S) media pages here and limit
  `inputs` to known sites. Hosts that support these matchers require at
  least one matcher across the two lists.
- Set `capabilities.browser_page` to offer a matching HTTP or HTTPS
  webpage in the companion extension. The user selects the engine to
  submit the page URL. Page matching uses both input lists.
- Set `skill` to package a self-contained UTF-8 Markdown guide at a portable
  relative `.md` path, up to 32 KiB. Explain when to choose the engine, give tool
  and option examples, and describe source limitations. NDM Next 0.6.0 and
  later link this guide from the installed `ndm-next` Skill when the engine is
  available to AI. Hosts can generate a basic guide from the manifest when
  this field is omitted.
- `limits.active_tasks` sets the engine's maximum simultaneous tasks.
  `limits.download_threads` is an optional per-task cap. The passed hint
  already includes the user's chosen setting and the declared cap. The
  engine defines how the hint maps to connections, fragments, or peers.
- `options` declares the engine's arguments. A field has a `key`,
  localized `label`, and `kind` of `boolean`, `integer`, `text`,
  or `choice`. Integer fields set `min` and `max`. Text fields set
  `max_length`. Each choice has a `value` and localized `label`.
  Fields can declare a `default` or be marked `required`.
  The desktop uses a sliding two-segment control for `boolean`. Choices
  with two or three short labels can also use sliding segments. These
  controls are available in the download form and plugin settings.
  An optional localized `help` appears on hover. A `text` field can declare
  `editor.kind: "multi_select"` with localized `choices` and `max_selected`.
  The engine still receives a comma-separated string. Use a `proxy_address`
  editor with `schemes` for a protocol menu and host-and-port input. The first
  listed scheme is the default for a bare address. `required_when` can name
  a Boolean field that makes the address required when enabled. The engine
  receives a complete proxy URL.
- `tools` can expose named MCP download actions. Each has a stable `id`,
  localized `name` and `description`, and optional preset `defaults` for
  the options. NDM registers them while AI integration is enabled.

## Languages and marketplace listings

Use a localized object wherever the manifest accepts a display string. Supply
a non-empty `en` string and add `zh` for Simplified Chinese. NDM displays
English when the Chinese string is omitted or empty. Keep IDs, option keys,
choice values and category IDs stable across languages.

```json
{
  "name": {"en": "Video downloader", "zh": "视频下载"},
  "description": {"en": "Save a video to the download folder.", "zh": "将视频保存到下载目录。"}
}
```

The same format applies to option labels, choice labels, panel label overrides
and MCP tool metadata. The panel receives `locale: "en"` or `locale: "zh-CN"`.
MCP uses the English tool metadata and stable tool IDs.

When submitting a plugin to the market, give its listing a localized `name`
and `description`, choose a stable `category` ID from `sources.json`, and
optionally include search `keywords` such as protocol abbreviations and
translations. NDM displays listing text and category labels in the selected
language and searches both languages. An imported ZIP with no listing uses
its manifest text on the card. [Marketplace listing format](https://github.com/AreChen/NDM-Next-Plugin-Market#listing-and-translations)

### 中文

插件名称、描述、选项和选项值的显示标签，以及面板覆盖的标签和 MCP 工具的
名称与描述，都可填写 `{"en": "English", "zh": "简体中文"}`。英文必填，
省略或留空 `zh` 时显示英文。ID、选项键和选项值使用固定标识。JavaScript
面板收到的 `locale` 为 `en` 或 `zh-CN`，MCP 工具使用英文元数据。

提交市场时，在目录中填写中英文名称与描述，选用 `sources.json` 中的分类
ID，也可添加中英文关键词和协议简称。市场卡片和分类随应用语言切换，
搜索同时匹配两种语言。没有市场条目的本地导入插件使用包内清单的文本。
[插件市场上架格式](https://github.com/AreChen/NDM-Next-Plugin-Market#listing-and-translations)

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

## Optional file selection and sharing

- Declare `capabilities.inspect_files` to expose a native checkbox list in the
  new download form. `revision` is an opaque, nonempty ASCII identifier up to
  128 bytes. IDs must be unique. If the task includes `selection`, validate
  its `revision` and sorted `file_ids` against the actual metadata before
  starting the transfer. Publish only the selected files.
- Declare `capabilities.share_completed` to expose start and stop sharing on
  a completed task. NDM can retain the engine-owned `state/` directory after
  publishing, and cache cleanup may reclaim it while the task is idle. Use it
  for metadata required to verify the source again. The `output` argument is the
  published file or folder and may have been renamed to avoid a collision.
  Share only verified data and do not modify the output. Respond to `stop` with
  `stopped` and close all file and network handles. An engine which ends sharing
  by itself returns `completed` with an empty `files` array.
  Sharing has its own per-engine count, using `limits.active_tasks` as the limit.
- When `hello` includes both `task_updates` and `peer_stats`, the engine may
  send `kind: "peer_stats"` task updates with `connected`, `seeders`, `connecting`,
  `queued`, `known`, `uploaded`, and `upload_bytes_per_second`. Upload totals are
  bytes since the worker started. NDM accumulates them across stops and restarts.
  Send the first sharing statistics after the verified data is ready to serve.

In Rust, use `Engine` and `serve_engine` for optional operations.
`Worker::update_task` suppresses peer statistics for older hosts.
