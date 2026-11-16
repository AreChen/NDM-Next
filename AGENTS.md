# NDM Next

NDM Next 是 Rust 1.100.0 / GPUI 桌面下载器，支持 Windows 和 macOS；浏览器扩展通过 Native Messaging 连接桌面端。

- 修改桌面或扩展 UI 前阅读 [DESIGN.md](DESIGN.md)。保持紧凑、中英文、亮暗主题；界面视觉验收由用户进行，未经本次任务明确授权不要操作用户的桌面窗口。
- 在仓库根目录运行扩展测试：`rtk proxy node --test tests/*.test.js`。
- Rust 命令在 `desktop` 目录运行：`rtk proxy cargo +1.100.0 test --locked --workspace`；提交前运行 `rtk proxy cargo +1.100.0 clippy --locked --workspace --all-targets -- -D warnings` 和 `rtk proxy cargo +1.100.0 fmt -p ndm-core -p ndm-desktop -p ndm-host -- --check`。默认测试会跳过需要公共网络、媒体工具或子进程显式入口的用例，不能把它们记为通过。
- 每个外部命令使用 `rtk`，需要原始输出时使用 `rtk proxy`。Windows 行尾检查使用 `rtk proxy git -c core.safecrlf=false diff --check`，不要修改全局 Git 行尾设置。
- 下载请求可能包含 Cookie、鉴权链接和代理凭据。排查时只输出必要的状态、计数和错误码，不输出 Cookie、鉴权链接或代理密码。真实用户数据库、缓存和安装目录不能当测试夹具。
- 浏览器接管先由宿主持久化 prepare，确认后取消浏览器下载并 commit；用户已选择立即接管策略。未知长度不是拒绝接管的理由。协议行为变更需要保留 HTTP 分段／断点续传、HLS／DASH 合并和 FTP／SFTP 能力。
- UI 渲染不得扫描磁盘、发网络请求或重新构建搜索索引。只为可见条目创建控件；检索／多选遵循当前筛选及排序。退出、安装交接必须独立于窗口存活，安装仍要等待下载安全落盘。
- GPUI 依赖必须保持同一分支和版本。动效使用 `gpui-motion-kit`；其 0.5.0 适配器暂放于 `desktop/vendor/gpui-motion-kit`，仅将依赖升级到 Kit 0.10.0，保留上游源码与许可。不要启用 CE 原生渲染或编辑其通用核心。动效状态持久保存在所属视图，停稳后不持续请求帧。
- 发布遵循 [.github/workflows/release.yml](.github/workflows/release.yml)；版本来自 `desktop/Cargo.toml`，发布说明放在 `docs/releases/`，文件名为 desktop-X.Y.Z.md，英中文分段。生产安装包仅由 GitHub Actions 构建；保持 Windows x64、macOS arm64／x64 和扩展打包成功，并校验发布文件与该标签 CI 产物一致。不要将本地构建冒充正式发布包。
- Windows 安装包目前接受不签名。macOS 使用现有 DMG 安装流程，不引入微软商店分发。扩展与桌面端独立版本，不为纯桌面变更修改扩展版本或固定 ID。

安装与更新相关变更同时阅读 [Windows 打包说明](desktop/packaging/windows/README.md) 或 [macOS 打包说明](desktop/packaging/macos/README.md)；扩展安装参见 [安装指南](docs/extension-install.md)。下载依赖可使用用户提供的本地代理 `http://127.0.0.1:18085`，不要写入仓库或全局配置。
