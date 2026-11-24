# 开发下载引擎插件 · Plugin development

先确定插件的输入与输出。例如，媒体网页可以产出一个视频和字幕，本地种子可以产出一组文件。为普通 HTTP 文件排队已有内置工具。需要新来源匹配、专用选项、内容选择或后处理时，可以实现一个 NDM 下载引擎。[完整 SDK 协议与清单](../../../../ndm-plugin-sdk/README.md)

## 运行一个最小插件

1. 示例接受本地 `.ndmexample` 文件，支持停止、恢复和传输进度，完成时报告工作区里的 `copy.bin`。准备好这两份文件。[Rust worker](../../../../ndm-plugin-sdk/examples/file-copy.rs) · [plugin.json 清单](../../../../ndm-plugin-sdk/examples/file-copy.json)
2. 在与目标桌面版匹配的 NDM Next 源码中，从 `desktop` 目录构建示例并运行宿主契约测试。独立 Rust 项目也可将这个 worker 保存为 `src/main.rs`，依赖同一份源码里的 `ndm-plugin-sdk` crate。其他语言可直接实现逐行 JSON 协议。[NDM Next SDK 源码](https://github.com/AreChen/NDM-Next/tree/main/desktop/crates/ndm-plugin-sdk)

   ```sh
   cargo +1.100.0 build --locked -p ndm-plugin-sdk --example file-copy
   cargo +1.100.0 test --locked -p ndm-core --test plugin_engine
   ```

3. 将示例清单复制并命名为 `plugin.json`，与编译好的 worker 放在同一目录。默认构建产物在 `target/debug/examples/`，设置过 `CARGO_TARGET_DIR` 时从相应目录取。Linux 和 macOS 保留可执行权限，在该目录执行下列命令。每个发布包对应一个平台和架构。

   ```sh
   chmod +x file-copy
   zip file-copy.zip plugin.json file-copy
   ```

   Windows 使用 `file-copy.exe`，将清单的 `engine` 也改为 `file-copy.exe`，再用 PowerShell 打包。

   ```powershell
   Compress-Archive -Path plugin.json,file-copy.exe -DestinationPath file-copy.zip
   ```

4. 在 **插件市场 → 导入插件** 安装 ZIP。在新建任务中选择一个本地 `.ndmexample` 文件，验证输出与原文件字节一致。打开 AI 接入后，用 `ndm_list_engines` 查询该文件的绝对路径，再通过返回的专用工具提交任务。示例清单导出的工具名为 `ndm_file_copy__copy`。

   ```json
   {"source":"/absolute/path/sample.ndmexample"}
   ```

   两个工具均可使用以上参数。Windows 路径在 JSON 中写成 `"C:\\Downloads\\sample.ndmexample"`。对返回的任务 ID 调用 `ndm_task_status`，确认 `completed` 并比较 `output` 文件的内容。

## 实现自己的 worker

设置稳定的插件 ID 和版本，用 `inputs` 定义自动匹配，用 `manual_inputs` 提供可手动选择的来源。把任务选项写入清单，插件工具、设置页和 MCP 共用这些定义。自定义工具的确切名称可用 `ndm_list_engines` 验证。[清单字段](../../../../ndm-plugin-sdk/README.md#package-manifest)

宿主以 `--ndm-engine` 启动 worker。标准输出专用于逐行 JSON，日志写到标准错误。Rust 的 `serve` 封装握手、停止信号和终态消息。在 `request.workspace` 写文件，关闭文件和子进程后返回相对路径，宿主负责发布到用户的下载目录。尽早报告已解析的标题。下载期间报告实际字节，取消时写好断点，恢复时核对来源和选项。需要预选文件或做种时，实现 `Engine` 并使用 `serve_engine`。[消息与生命周期](../../../../ndm-plugin-sdk/README.md) · [选择文件与做种](../../../../ndm-plugin-sdk/README.md#optional-file-selection-and-sharing)

## 复用已安装的 yt-dlp

增强插件可以只打包站点适配、参数和后处理代码。在消费方清单使用 schema 2，声明提供方插件的版本范围及包内可执行文件路径。这里的版本号指「流媒体下载」插件版本。Windows 包使用 `runtime/yt-dlp.exe`。

```json
{
  "schema": 2,
  "runtime_dependencies": [
    {"name":"yt-dlp","plugin":"yt-dlp","version":"^0.2.1","path":"runtime/yt-dlp"}
  ]
}
```

以上为需要合并到清单的字段。worker 通过 `ndm_plugin_sdk::runtime_path("yt-dlp")` 取得绝对路径，其他语言从 `NDM_PLUGIN_RUNTIMES` 的 JSON 对象读取。同一个 yt-dlp 可供多个插件使用。依赖 Deno 时也要声明对应工具。测试提供方缺失、版本不匹配、兼容升级和卸载提示。[共享工具约定](../../../../ndm-plugin-sdk/README.md#reusing-an-installed-executable)

## 让 AI 渐进发现这个插件

在清单设置 `"skill":"AI-GUIDE.md"`，并把这份 UTF-8 Markdown 放进 ZIP。说明匹配哪些来源、何时需要显式选择、最小的工具调用、常见选项、输出和限制。示例要使用清单声明的键。可以给出一个成功例子和一个需换用其他引擎的例子。用户开启插件的 AI 接入后，宿主会把该指南放进[当前安装的插件索引](../plugins/index.md)。

发布前至少实际走一次导入、来源匹配、下载、停止并恢复，以及 MCP 工具发现。多文件输出还要检查重名和部分选择。市场条目使用本地化名称与描述，分平台列出 ZIP 与 SHA-256。[市场上架格式](https://github.com/AreChen/NDM-Next-Plugin-Market#listing-and-translations)
