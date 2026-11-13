# NDM Next Windows 下载器实施计划

> **面向 Agent 型工作者：** 使用 superpowers:subagent-driven-development 或 superpowers:executing-plans 逐项实施。步骤使用复选框跟踪。

**目标：** 在现有仓库交付独立的 Windows 桌面下载器，并让浏览器扩展能够安全地把下载交给它。

**架构：** Rust 工作区按核心、桌面界面和 Native Messaging 宿主划分。核心负责持久化任务、协议下载和受控的接管状态；扩展通过浏览器启动宿主，宿主在持久保存和明确提交前不启动对应任务。旧版 NeatDM 仍通过现有兼容路径连接。

**技术栈：** Rust 1.100、GPUI Kit 0.9.1、Tokio、Reqwest、SQLite、Windows Native Messaging 与用户级凭据保护；FTP/FTPS、SFTP 和 HLS 使用经过验证的 Rust 依赖，必要时采用经审查的 FFmpeg。

**规格说明：** `docs/superpowers/specs/2026-11-14-ndm-next-desktop-design.md`

## 全局约束

- Windows 10 1903+、Windows 11 22H2+ x64；ARM64 只在构建及实机验证通过后发布。
- 首版同时覆盖 HTTP(S)、FTP、FTPS、SFTP 和 HLS，不以中间阶段替代最终交付。
- 仅提供安装版；桌面与扩展同仓库、各自版本；亮暗主题和中英文。
- 服务端不支持分段时回退单连接；续传不能混合不同资源；下载目标文件不得无意覆盖。
- 新桌面端必须持久接单后浏览器才可取消；原版 NeatDM 的旧协议不支持这种确认。
- 不绕过 DRM；敏感资料不可明文写入普通日志；许可证和安装包签名状态如实披露。
- 依赖下载需要代理时仅为当前进程设置 `HTTP_PROXY` 和 `HTTPS_PROXY` 为 `http://127.0.0.1:18085`。
- 所有外部命令使用 `rtk` 或 `rtk proxy`，不覆盖现有 `store-assets/` 用户文件。

<a id="review-focus"></a>
## 审查重点

1. 浏览器取消成功但确认消息丢失、服务工作线程重启或进程意外退出：任务必须可恢复，重复消息不得创建多项桌面任务。
2. 服务端拒绝 Range、返回错误区间、更换资源或要求认证：不能拼接错误内容、泄露凭据或报告成功。
3. 临时文件、数据库或目标磁盘出现权限/空间/异常断电问题：保留可恢复状态，不覆盖用户现有文件。
4. HLS 音轨、AES 密钥、直播列表、缺段或时序不一致：不把损坏媒体标记为完成，也不绕过 DRM。
5. 不同浏览器、扩展 ID、MSIX 注册表虚拟化、桌面程序未运行：不能因接管失败而中断原下载。

## 文件结构与接口

- `desktop/Cargo.toml`：Rust 工作区；`desktop/rust-toolchain.toml`：精确工具链版本。
- `desktop/crates/ndm-core/src/`：任务模型、持久存储、凭据保护、协议实现、下载调度与媒体处理；`tests/` 放可控服务集成测试。
- `desktop/crates/ndm-desktop/src/`：GPUI 桌面程序、状态订阅、界面、托盘及启动项。
- `desktop/crates/ndm-host/src/`：Native Messaging 消息帧、浏览器交接及桌面程序按需启动。
- `extension/background.js`、`extension/rules.js`、弹出面板与 `tests/`：后端选择、保留过滤行为及持久接单交接。
- `desktop/installer/`：Windows 安装与卸载、浏览器宿主注册、MSIX 构建和许可证资料。
- `desktop/crates/ndm-core/src/model.rs` 定义 `TaskId`（UUID）、`NewTask`（URL、目标目录、建议文件名、可选来源 URL、大小和请求头）、`Task`（ID、请求、状态、进度、错误）及 `TaskState`（Prepared、Queued、Running、Paused、Completed、Failed）。
- `desktop/crates/ndm-core/src/store.rs` 暴露 `Store::open(path: &Path) -> Result<Store>`、`prepare(&self, id: TaskId, task: &NewTask) -> Result<Task>`、`commit(&self, id: TaskId) -> Result<Task>`、`abort(&self, id: TaskId) -> Result<()>`、`get(&self, id: TaskId) -> Result<Option<Task>>` 和 `list(&self) -> Result<Vec<Task>>`；唯一 ID 和事务保证准备/提交幂等。调度器使用同一存储负责暂停、恢复和删除。

### Task 1：持久任务模型和 HTTP 下载

**文件：** 创建 `desktop/Cargo.toml`、`desktop/rust-toolchain.toml` 和 `desktop/crates/ndm-core/`；修改 `.gitignore`。

- [x] 建立任务状态及 SQLite 事务存储；为重复 `prepare`、重启恢复、手动暂停及目标文件名冲突编写失败测试。
- [x] 用 Windows 用户级保护封装敏感字段，验证保存内容不含明文凭据；按平台隔离系统调用。
- [x] 用本地 HTTP 服务测试有效 Range、拒绝 Range、错误 `Content-Range`、未知长度、资源标识改变及暂停续传；实现有界并发、速率限制和安全回退。
- [x] 验证 `rtk proxy cargo +1.100.0 test --manifest-path desktop/Cargo.toml -p ndm-core` 全部通过。

### Task 2：FTP、FTPS、SFTP 与 HLS

**文件：** 创建 `desktop/crates/ndm-core/src/protocol/`、媒体模块和对应集成测试。

- [ ] 为 FTP、显式/隐式 FTPS 及 SFTP 添加可控服务测试，覆盖认证失败、TLS/SSH 主机身份变化、取消和不可续传的退化；实现对应任务执行器。
- [ ] 为 TS/fMP4、分离音轨、AES-128、直播播放列表刷新和缺段建立本地可重复样本测试；实现播放列表刷新、选轨、密钥获取、有界写盘、停止和 MP4/MKV 输出。
- [ ] 若 Rust 媒体路径不能可靠覆盖已约定范围，准备独立 FFmpeg 兜底，并记录准确构建选项、许可证、源码、安装包增加体积与调用时机。
- [ ] 运行核心协议及媒体测试，确认产物可解码且错误输入不会标记成功。

### Task 3：GPUI 桌面程序

**文件：** 创建 `desktop/crates/ndm-desktop/`，使用现有 `assets/ndm-next-icon-source.png`。

- [ ] 以 GPUI Kit 实现任务列表、添加下载、暂停/恢复/删除、进度和速度；桌面操作调用核心接口而非重复实现状态。
- [ ] 实现中英文与亮色、暗色、跟随系统主题，保留紫蓝视觉风格；验证中文显示、输入、键盘操作和常见缩放比例。
- [ ] 提供并发连接、速度、代理/认证、自动恢复和敏感资料清除设置，以及过期链接更新；验证各设置作用于实际任务。
- [ ] 完成托盘、关闭窗口继续下载、开机自启开关和启动恢复；手动暂停任务不自动恢复。
- [ ] 构建并运行桌面程序，验证状态从真实下载任务更新，不把静态界面视为完工。

### Task 4：扩展与 Native Messaging 接管

**文件：** 创建 `desktop/crates/ndm-host/`；修改 `extension/manifest.json`、`extension/background.js`、`extension/popup.*` 和 `tests/`。

- [ ] 为宿主编写长度前缀与消息解析测试；任务 ID 重复、无效消息、崩溃恢复及凭据泄露均有明确结果。
- [ ] 实现 `prepare` 持久提交、扩展取消浏览器下载、`commit` 开始任务，以及失联/重启时的幂等恢复；请求不可安全重放、来源不明且启用域名黑名单、或接管失败时保留浏览器原下载。
- [ ] 保留原 NeatDM 后端和现有过滤设置，增加后端选择、新后端状态和自动/逐项确认模式；旧安装默认保留原后端，新安装默认新后端；旧后端不伪报持久确认。
- [ ] 运行 `rtk proxy node --test tests` 和宿主集成测试，最后用实际 Chrome/Edge 扩展与桌面进程验证接管及故障回退。

### Task 5：仅安装版、发布资料和端到端验证

**文件：** 创建 `desktop/installer/` 和必要的 CI；更新简洁的中英文说明与隐私政策。

- [ ] 实现 GitHub 安装版与商店 MSIX 的构建配置，注册和卸载浏览器宿主、保留或清理用户数据；不发布便携版。
- [ ] 实测安装、卸载、扩展自动接管与按需启动；GitHub 版提供更新通知，商店版使用商店更新；MSIX 注册表虚拟化不能通过时如实记录并继续解决，不宣称已支持。
- [ ] 整理第三方许可证、FFmpeg 许可及实际签名状态；在真实签名和商店审核前不声称商店版或签名包已发布。
- [ ] 以相同状态测量空闲、窗口、单任务和多任务的内存、CPU、吞吐量，分别报告可执行文件和安装包大小。
- [ ] 运行所有 Rust/JavaScript 测试及 release 构建，对照规格逐项核验；只在全部已承诺功能和必要平台验证完成后宣布交付。
