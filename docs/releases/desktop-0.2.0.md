## English

- Upgraded to GPUI Kit **0.10.0** and integrated **gpui-motion-kit 0.5.0**. The sidebar resizes smoothly, page changes fade in, and progress bars ease between updates. The Kit adapter is vendored with its dependency updated to 0.10.0, keeping a single GPUI backend.
- **Shift + click** a task row or checkbox to select an inclusive range in the current filtered and sorted list. Task action buttons do not change the selection.
- Type and sort menus highlight the current option without checkmarks or an empty icon column. The page header shows its title; task counts now appear beside sidebar navigation.
- The search field starts unfocused and releases focus when you click outside it. **Ctrl+K / Cmd+K** still focuses search.
- Compact download windows reuse task-row icon buttons with tooltips. Elapsed time, average speed and actions share one row; the default height is **168px during downloads / 136px after completion**.
- Active download windows have a subtle moving background that reuses progress updates. It is hidden when the window is inactive, hidden, paused or finished. Turn off animations under **Settings → Appearance & language → Motion**.
- Added **AGENTS.md** and **DESIGN.md** to document project workflows, design rules and animation performance limits.

Windows x64 and macOS Apple Silicon / Intel packages are built by GitHub Actions. The matching browser extension remains **1.1.4**. Windows packages are unsigned; macOS updates continue to use a verified DMG for replacing the app in Applications.

## 中文

- 升级至 **GPUI Kit 0.10.0**，接入 **gpui-motion-kit 0.5.0**：侧栏宽度平滑变化、页面切换淡入、进度条平滑过渡。保留上游 Kit 适配器源码，仅调整其依赖到 0.10.0，保持单一 GPUI 后端。
- 按住 **Shift 点击任务行或复选框**，即可按当前筛选和排序连续选择首尾之间的任务。点击操作按钮不会改变选中状态。
- 类型与排序菜单使用背景高亮表示当前选项，去掉勾选标记及空图标列。页面顶部只保留标题，任务数量移至侧栏导航右侧。
- 搜索框默认不再聚焦，点击外部后取消焦点；保留 **Ctrl+K / Cmd+K** 快捷键。
- 独立窗口复用下载条目的图标按钮并提供悬浮说明；耗时、平均速度与操作合为一行，默认高度缩至**下载中 168px／完成后 136px**。
- 活动下载窗口增加轻微流光背景，复用进度刷新，不添加独立循环定时器。窗口未激活、隐藏、暂停或完成时不显示流光；可在**设置 → 外观与语言 → 动效**中关闭动画。
- 新增 **AGENTS.md** 与 **DESIGN.md**，记录开发流程、视觉规范及动效性能边界。

Windows x64、macOS Apple Silicon／Intel 安装包由 GitHub Actions 构建；配套浏览器扩展保持 **1.1.4**。Windows 安装包不签名；macOS 更新仍通过已校验的 DMG 替换 Applications 中的程序。
