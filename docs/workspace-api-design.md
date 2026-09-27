# 工作区 API 设计审阅稿（ProjectWorkspace / ProjectController / FloatingPanel）

- 日期：2026-09-27。状态：**待审阅**。
- 基线：`Compositor/Compositor/Document/ProjectWorkspace.swift`（210 行）、
  `Compositor/Compositor/IO/ProjectController.swift`（323 行）、
  `Compositor/Compositor/UI/FloatingPanel.swift`（174 行）、`UI/ProjectTabs.swift`（304 行，仅 UI）。
- 原则：模型层逐方法 1:1；被平台差异迫改的两处（async 对话框、面板命令式 API）改形状不改语义；
  全部偏差集中列在第 9 节。

## 0. 组件总览

| Swift | Rust 位置 | 说明 |
| --- | --- | --- |
| `ProjectTab` | `crates/app/src/tabs.rs` | id + session + controller + 默认名 |
| `ProjectWorkspace` | `crates/app/src/tabs.rs` | 标签集合与切换守卫，纯逻辑为主 |
| `ProjectController` | `crates/app/src/project.rs` | 每 tab 一份的文件生命周期（open/save/export/size/trim/quit/导入队列） |
| `FloatingPanelController` | `crates/app/src/panels.rs` | 面板注册表 + 浮层容器元素（gpui-component 配方） |
| `copyLayer` 核心逻辑 | `crates/core`（`project.rs` 或 `session.rs`） | 纯逻辑下沉 core，可测 |
| 标签条 UI | `workspace.rs`（现布局内新行） | gpui-component `TabBar` pill 变体 |

前置核对结果（本次已查证）：
- 五个编辑会话状态在 core session 均存在：`hue_saturation`/`filter_edit`/`gradient_edit`/
  `pixel_move`/`color_picker`（session.rs:346-424）→ `can_switch` 可逐字移植。
- `canStartProjectOperation`/`isProjectBusy`/`isImporting` **core 不建模**（session.rs:1713 注释明确
  "app-side"）→ busy 状态归 app 层（`is_managing` + controller busy），`can_switch` 接收 app 侧快照。
- `project_url` **core 不存在** → 需新增（见 §7）。

## 1. 存储与所有权（方案 A，备选 B）

**方案 A（推荐，忠实结构）**：`ProjectWorkspace` 持有全部 tab（含当前），`AppState` 不再有平铺的
`session` 字段，现有 386 处 `self.session` 调用点（canvas 137 / state 69 / workspace 81 / tools 35 /
sheets_ui 34 / sheets 17 / layers_panel 7 / color_picker_ui 6）改走访问器：

```rust
impl AppState {
    fn tab(&mut self) -> &mut ProjectTab { self.workspace.current_tab_mut() }
    // self.session  →  self.tab().session   （机械改写 + 编译修周期）
}
```

优点：单一事实来源（无双表示去同步 bug 类）；标签条直接渲染 `workspace.tabs`，与 Swift UI 代码同构；
`select()` 退化为 `selected_id` 变更 + 重适配标记 + notify，和 Swift 一样轻。
成本：386 处机械改写，其中少量借用冲突点需局部重构；估 1–2 个工作日。

**方案 B（备选，最小改动）**：`AppState` 保留平铺的当前 `TabState`，`workspace.tabs` 只存非当前 tab，
`select()` 时整体 `mem::swap`。优点是调用点零改动；偏差是 tabs 数组与 Swift 不同构（标签条要
`all_tabs() = [current] + tabs` 拼装），且平铺字段与 tab 内副本是长期隐患。

Swift 按 `.id(tab.id)` 整体重建视图 ⇒ 每页视口不持久、切页即重适配 fit；Rust 照抄（切换后置
`fit_for = (0,0)` 强制 `zoom_fit`），因此 `TabState == ProjectTab`，**无需**每页另存 zoom/pan。
frame 缓存（`frame_image`）以 `revision` 为键 → 键改为 `(tab_id, revision)`，避免切页后显示上一页
陈旧帧。缩略图缓存键 `(layer_id, mask_selected)` 的 UUID 全局唯一，跨页共享安全。

## 2. ProjectTab

```rust
pub type TabId = uuid::Uuid;

pub struct ProjectTab {
    pub id: TabId,
    pub session: EditorSession,
    pub controller: ProjectController,
    pub default_name: SharedString,
}

impl ProjectTab {
    pub fn new(name: impl Into<SharedString>) -> Self;  // 建 session + controller
    pub fn title(&self) -> SharedString;                // project_url 文件名（去扩展）?? default_name
}
```

Swift `skipsInitialClipboardCanvasSize`（首 tab 跳过启动时按剪贴板定画布尺寸）：Rust 会话无此
启动探测行为，待实现时核对后标 N/A（偏差 #7）。

## 3. ProjectWorkspace（逐方法：签名 / 效果 / 出处）

方法尽量不含 gpui 类型（守卫、集合操作、序），只有发起对话框的方法收 `&mut AppState, &mut Window,
&mut Context<AppState>` —— 这是 `ProjectWorkspaceTests` 能以普通断言移植的前提。

```rust
pub struct ProjectWorkspace {
    tabs: Vec<ProjectTab>,
    selected_id: TabId,
    pub is_managing: bool,
    next_number: u32,                       // 从 2 起，"Untitled 2"、"Untitled 3"…
}

impl ProjectWorkspace {
    pub fn new() -> Self;
    pub fn current(&self) -> &ProjectTab;                 // selected 命中 ?? tabs[0]（:26）
    pub fn current_mut(&mut self) -> &mut ProjectTab;
    pub fn tabs(&self) -> &[ProjectTab];                  // 标签条渲染用
    pub fn selected_id(&self) -> TabId;

    pub fn can_switch(&self, busy: &ProjectBusy) -> bool; // :27-31
    pub fn add_tab(&mut self, reuse_empty: bool) -> &mut ProjectTab;   // :38-46
    pub fn select(&mut self, id: TabId, cx: &mut Context<AppState>) -> bool;   // :47-52
    pub fn new_canvas(&mut self, app: &mut AppState, window, cx);      // :53-57
    pub fn open(&mut self, urls: Option<Vec<PathBuf>>, app, window, cx);       // :58-76，两阶段
    pub fn load_project(&mut self, url: &Path, app, window, cx) -> bool;       // :77-88
    pub fn close(&mut self, id: TabId, app, window, cx);                  // :89-96，两阶段
    pub fn remove_tab(&mut self, id: TabId);                             // :97-102
    pub fn quit_order(&self) -> Vec<TabId>;                              // :105
    pub fn confirm_quit(&mut self, app, window, cx) -> bool;             // :106-114，两阶段
    pub fn close_window(&mut self, app, window, cx);                     // :115-119
    pub fn receive(&mut self, urls: &[PathBuf], into: Option<TabId>,
                   at: Option<Point<f64>>, app, window, cx);             // :123-141
    pub fn copy_layer(&mut self, id: Uuid, into: Option<TabId>,
                      at: Option<Point<f64>>, app, window, cx);          // :157-209
    pub fn can_receive_drag(&self, into: Option<TabId>, source: Option<TabId>) -> bool; // :276-280
}

pub struct ProjectBusy { pub is_project_busy: bool, pub is_importing: bool, pub has_brush_stroke: bool }
```

**效果明细**（每条标注 Swift 行号）：

- `new()` — 首个 tab 名 `"Untitled"`（:32-37）。
- `can_switch` — `!is_managing && current.controller.can_start(busy) && session 五个编辑会话全空`
  （hue_saturation/filter_edit/gradient_edit/pixel_move/color_picker 均 None；:27-31）。
- `add_tab(reuse_empty)` — `reuse_empty && tabs.len()==1 && current.session.document==None` 时返回
  当前 tab（空页复用）；否则新建 `"Untitled {next_number}"` 并 `next_number += 1`、置为选中（:38-46）。
- `select(id)` — `id == selected` 或 `!can_switch` 或不存在 → false；否则 `current.session.commit_transform()`
  后换 `selected_id`，置视口重适配标记（偏差 #6）。
- `new_canvas()` — `can_switch` 才继续；`commit_transform`；`add_tab(reuse_empty: false)`（:53-57）。
- `open(urls)` — `!can_switch` → false；置 `is_managing`；无 urls 时先开 .comp 文件选择器
  （**多选**、包文件不当作目录）；逐个 `load_project`，返回"是否开成了至少一个"（:58-76）。两阶段：
  第一半开选择器并在 controller 挂 `Pending::Open`，选择器回调续第二半。
- `load_project(url)` — 按 **resolveSymlinksInPath 后的路径**去重，已开则仅选中（:78-80）；否则新建
  tab（**游离态**：失败即整 tab 丢弃，:81-84）；`ProjectStore` 载入 + PSD 转换说明计入状态栏；若仅有的
  一个 tab 无文档则先移除再入列；选中新 tab（:85-87）。
- `close(id)` — `!can_switch` → 不动；置 `is_managing`；`controller.confirm_quit` 流（未保存询问）；
  通过后 `remove_tab`（:89-96）。
- `remove_tab(id)` — 移除；空了 → `add_tab(reuse_empty:false)` 补一个；若移除的是选中页 → 选中
  `tabs[min(index, count-1)]`（:97-102）。
- `quit_order()` — `[current] + 其余（保持序）`，保证"先问屏幕上这页"（:105）。
- `confirm_quit()` — `!can_switch` → false；对 `quit_order` 逐 tab 走 `confirmReplacement`
  询问，任一取消即整体中止（:106-114）。Rust 为逐 tab 推进的状态机（偏差 #4）。
- `close_window()` — `confirm_quit` 通过后清空并补一个空白 tab，再关窗口（:115-119）。
  接 `gpui on_window_should_close`（可否决）。
- `receive(urls, into, at)` — .comp → `load_project`；图片 → `into` 指定 tab（不存在则跳过该文件）
  或 `add_tab()`（每图一页）；`session.import_images([url], at)`（:123-141）。Swift 的 30ms 轮询等待
  canSwitch 改为事件驱动：挂入 `controller.incoming` 队列，每次对话框/sheet 完成后尝试 drain（偏差 #5）。
- `copy_layer(id, into, at)` — 核心 100 行下沉 core（§7 `copy_layers_between`）；此处只做：
  找含该 id 的源 tab、守卫、把 `into==None` 解析成 `add_tab(reuse_empty:false)`、调用 core 函数、
  选中目标 tab（:157-209）。
- `can_receive_drag(into, source)` — 文件拖放恒真；图层拖放若 `into == 源tab` 则否（Option 拖拽复制
  在图层面板内部处理，不算跨页拖放；:276-280）。source 不再嗅探拖拽粘贴板，由拖拽项直接携带
  `(tab_id, layer_id)`（偏差 #3）。

## 4. ProjectController（两阶段对照表）

```rust
pub struct ProjectController {
    save_generation: u32,           // saveCurrent 成功 +1；open 前后比对检测"询问期间存过盘"
    incoming: Vec<Incoming>,        // 导入队列（files, point）
    processing: bool,
    pending: Option<Pending>,       // 两阶段状态机的挂起步骤
}
pub enum Pending { SaveAs { .. }, Confirm { tab: TabId, step: QuitStep }, Open { .. }, Error(..) }
```

| Swift（行号） | Rust 入口（第一半） | 完成回调（第二半） | 效果要点 |
| --- | --- | --- | --- |
| `canStart`（:11-13） | `can_start(&self, session, busy)` | — | `canStartProjectOperation && !is_managing` |
| `begin()`（:16-22） | `begin(session) -> bool` | — | canStart 才过；`cancel_crop` + `commit_transform` + busy 置位 |
| `save(asNew:)`（:25-29,152-182） | `save(as_new)` | `on_save_target(url)` | 有 projectURL 且 !as_new → **直接写盘不开面板**；否则 .comp 保存面板（默认名 `projectURL ?? "Untitled.comp"`，标题 Save (Project) As）；成功：写盘、`session.project_url = url`、`history.mark_saved()`、`save_generation += 1` |
| `exportPNG()`（:31-49） | `export_png()` | `on_export_target(url)` | .png 面板，默认名 `projectURL ?? "Untitled"` + `.png`；core `export.rs` 已有 png_data |
| `exportJPEG()`（:119-151） | `export_jpeg()` | （现有 JPEG sheet 流） | 质量滑杆 + 哑光色，已有 |
| `canvasSize()/imageSize()/trim()`（:51-117） | 现有 sheet 流搬入 | — | 守卫改走 `begin()`；选项 sheet → core resizer → `apply_document_size`/`apply_image_size` |
| `open(_:)`（:185-223） | `open(url)` | `on_open_target(urls)` | 挂 workspace 时**转投 `workspace.open`**（:186）；直连路径语义并入 `load_project`：**先验证加载快照**（坏项目绝不毁掉活文档）→ `confirmReplacement`（询问中存盘可能替换正打开的文件 → `save_generation` 变了且路径相同则**重载** :210-215）→ `install_project` |
| `newCanvas()`（:225-231） | `new_canvas()` | — | 挂 workspace → `workspace.new_canvas()`；无 workspace 路径 R1 不需要 |
| `close(_:)`（:233-244） | `close()` | — | 挂 workspace → `workspace.close(current.id)` |
| `confirmQuit()`（:246-250） | `confirm_quit()` | `on_quit_choice(choice)` | `begin()` + `confirmReplacement` |
| `confirmReplacement()`（:252-263） | `request_unsaved_choice(tab)` | `on_unsaved_choice` | `!is_modified \|\| document==None` → 直接 true；文案 **"Save changes to {title}?"** / "Your changes will be lost if you don't save them."；按钮序 **Save / Cancel / Don't Save**：Save→`save_current`（可能再弹保存面板）结果即结论；Cancel→false；Don't Save→true |
| `receive(_:at:)`（:287-322） | `receive(urls, at)` | drain 步进 | 队列 + `processing` 防重入；.comp **一次最多一个**（多个 → 错误 "Open one project at a time"）；有 .comp → `open(project)` 决定 proceed，再 `import_images(非.comp, at: 无项目时才传 point)` |
| `showError(_:error:)`（:265-272） | `show_error(title, msg)` | — | 警告样式、messageText + informativeText + OK，复用现有 sheet/alert 通道 |

Rust 里 controller 永远挂 workspace（单窗口应用），Swift 的"无 workspace 回退路径"不移植；
`startAccessingSecurityScopedResource`（macOS 沙盒）N/A；`NSDocumentController` 最近文档列表 v1 不做
（偏差 #8）。`waitForFileRequest` 对应 Rust 会话导入管线的 pending-imports 门，实现时核对名称。

## 5. FloatingPanel / 面板系统

```rust
pub enum PanelPlacement { Automatic, DockedToMainWindowRight }

pub struct PanelState { pub name: &'static str, pub title: SharedString, pub placement: PanelPlacement }

pub struct FloatingPanels {
    open: Vec<PanelState>,                            // 序 = 叠放序（最后者最上）
    positions: HashMap<&'static str, Point<Pixels>>,  // 记忆位置
}
pub const DOCKED_WIDTH: f32 = 440.0;                  // Camera Raw 三轮最宽适配（FloatingPanel.swift:125）

impl FloatingPanels {
    pub fn show(&mut self, name, title, placement);        // 已开 → 置顶并更新标题；未开 → 入栈
    pub fn close(&mut self, name) -> bool;                 // 关闭并路由 onClose = Cancel
    pub fn is_visible(&self, name) -> bool;
    pub fn position(&self, name, canvas_center, window_size) -> Point<Pixels>;
    pub fn remember_position(&mut self, name, p);          // 拖动结束记；Docked 恒不记（:100-103）
    pub fn refocus(&self, name, window, cx);               // 画布点击后焦点回面板（:171-173）
}
```

效果（对齐 FloatingPanel.swift）：
- **Automatic**：首次无记忆位置 → 开在**画布区中心**（对应 `canvasCenter(in:)` :106-116，取画布视图
  bounds 的屏心 → Rust 取画布元素 bounds 中心）；有记忆 → 开在上次左上角；内容尺寸变化不挪面板（:59-61）。
- **DockedToMainWindowRight**：宽 440、贴主窗口右缘、随窗口 resize/move 跟随（:127-157 的 frame
  observer → Rust 每帧按窗口 bounds 重算即可）；**不记忆位置**（:100-103 注释明说：否则下次 Gaussian
  Blur 会开在右缘）。
- 关闭按钮 = `onClose` → 调用方按 Cancel 处理（sheet 语义不变）；Esc 走 `cancel_active` 现有路由。
- 面板名常量（ContentView.swift:11-15、KeyboardShortcuts.swift:133、ColorPickerSheet.swift:167）：
  `levelsPanel` / `adjustmentPanel` / `selectionAmountPanel` / `filterPanel` / `effectsPanel` /
  `keyboardShortcuts` / `colorPickerPanel`。
- 容器元素按 Dialog 配方（gpui-component dialog/dialog.rs:565-594）：
  `anchored().position(pos).snap_to_window()` + `.occlude()` + `v_flex` 外壳 + 标题行
  （`h_flex` 拖动区 + ghost `Button` 关闭）+ 内容槽；Docked 面板高度 = 主窗口内容高。
- 内容视图复用 sheets_ui 现成组件；面板内控件一律 gpui-component `Input`/`Slider`/`Select`/`Button`/`Icon`。

## 6. 标签条（TabBar 接线）

- `TabBar::new("project-tabs").pill()`，`Tab` 的 `prefix` 放未保存圆点（`session.is_modified()`），
  `suffix` 放关闭按钮（`Button` ghost + 9pt x 图标语义），`on_click` → `workspace.select(id)`；
  `menu(true)` 处理溢出（Swift 的边缘渐隐 v2 再做，偏差 #10）；tab 宽度按 Swift 公式
  `min(155, max(35, ceil(标题宽) + 未保存点10px)) + 40`（ProjectTabs.swift:59-68）。
- 位置：自绘 TitleBar 下一行 34px。
- 标签条整体与每个 tab 都是拖放目标：图层拖放 → `receive`/`copy_layer`，文件拖放 → `receive`；
  "New" 虚线槽（ProjectTabs.swift:200-216）= 拖放目标 `into: None`。gpui 外部文件拖放支持待验证，
  未验证前 v1 用 Open 菜单（偏差 #3）。
- can_switch 为假时 tab 与关闭按钮禁用（ProjectTabs.swift:236,242）。

## 7. core 增补

```rust
// session.rs
pub project_url: Option<PathBuf>,   // ProjectController.saveCurrent 写；title/open 去重用
pub fn install_project(&mut self, snapshot: Document, url: &Path);
    // open 成功后整体替换文档 + project_url + history.mark_saved()（Swift installProject）
pub fn clear_project(&mut self);    // Swift clearProject（R1 的 workspace 路径用不到，先核对是否存在）

// project.rs
pub fn copy_layers_between(source: &EditorSession, target: &mut EditorSession,
                           root_id: Uuid, at: Option<Point<f64>>) -> Result<(), String>;
```

`copy_layers_between` 效果（ProjectWorkspace.swift:157-209 逐行）：
1. 集合 = `root_id` 及其后代；从 `source` 取快照与图层子集。
2. 预算：目标已用像素 + 新增像素 ≤ 100,000,000，超限返回
   `"The copied layers exceed this project's 100-megapixel limit."`。
3. `mask_source_id` 不在集合内的图层：调整层直接置 None；像素层先 `LiveMaskBaker.bake` 成位图蒙版
   再置 None（跨页拷贝时活蒙版落袋为安）。
4. UUID 全量重映射（uniqueKeysWithValues 语义，冲突即 panic 降级为错误）；`parent_id`/`mask_source_id`
   随映射改写，出集合的置 None。
5. 平移：`center − anchor`（anchor = 根图层中心，无根则源文档中心；目标点 = `at` ?? 目标文档中心）；
   蒙版 placement 同步平移。
6. 目标无文档 → 以 `目标尺寸 ?? 源尺寸` `create_document`；`begin_edit("Copy Layers from Project")`
   → 追加 → `active = mapping[root]` → `end_edit`（一条 undo）。

## 8. 接线点清单

- **AppState**：`workspace: ProjectWorkspace`、`panels: FloatingPanels`、`fit_for` 归零标记；
  frame 缓存键改 `(tab_id, revision)`。
- **窗口关闭**：`on_window_should_close` → `workspace.close_window` 两阶段（询问时返回 false 否决关闭）。
- **菜单/快捷键改投 workspace**：New ⌘N→`new_canvas`、Open ⌘O→`open(None)`（多选）、
  Close ⌘W→`close(current)`、Save ⌘S→`save(false)`、Save As ⇧⌘S→`save(true)`、Quit→`confirm_quit`。
- **标签条行**插在 TitleBar 与工具选项条之间；高度 34px。
- **面板迁移顺序**：Levels（最现成）做样板 → Adjustment/SelectionAmount/Filter/Effects →
  ColorPicker → Shortcuts → Camera Raw（Docked）。每迁一个，对应 `SheetKind` 删除。

## 9. 与 Swift 的偏差清单（审阅重点）

| # | 偏差 | 理由 |
| --- | --- | --- |
| 1 | `async` 对话流程 → 两阶段状态机（`Pending` + 完成回调） | gpui 对话框是声明式 sheet；`is_managing`/`can_switch` 守卫语义不变 |
| 2 | 面板位置：屏幕坐标 → **窗口坐标**；无 `hidesOnDeactivate`；拖不出主窗口 | 窗口内浮层路线（gpui Windows 端无真 NSPanel 语义，已验证 `WindowKind::Floating` 落 WS_SYSMENU 分支） |
| 3 | 拖放来源：粘贴板嗅探 → 拖拽项携带 `(tab_id, layer_id)`；外部文件拖放 v1 缺 | gpui 拖放模型不同；外部拖放支持待验证 |
| 4 | `confirm_quit` 逐 tab await 循环 → 逐 tab 推进的状态机 | 同 #1；任一时刻只有一个未决询问，quitOrder 顺序不变 |
| 5 | `receive` 的 30ms 轮询等待 canSwitch → 事件驱动 drain | 无 async 运行时可轮询；队列语义等价 |
| 6 | 切页视口：跟随 Swift `.id()` 重建 = 重置为 fit（`fit_for` 归零）；frame 缓存键加 tab 维度 | 与 Swift 行为一致，且避免陈旧帧 |
| 7 | `skipsInitialClipboardCanvasSize` 待核对（Rust 疑无启动剪贴板探测） | 实现时确认后标 N/A 或补行为 |
| 8 | `NSDocumentController` 最近文档 v1 不做；security-scoped resource N/A | 平台特有 |
| 9 | 无 workspace 的回退路径不移植（controller 恒挂 workspace） | 单窗口应用，回退路径死代码 |
| 10 | 标签条溢出：v1 用 TabBar 溢出菜单，Swift 的滚动 + 边缘渐隐 v2 | TabBar 现成能力优先，1:1 视觉后续追 |

## 10. 验收

- **移植 `ProjectWorkspaceTests.swift`**：can_switch 守卫、空 tab 复用、quitOrder 顺序、坏项目不落
  tab、select 的 commit_transform、remove_tab 选中回退。模型方法 gpui-free，直接断言。
- **移植 `FloatingPanelTests.swift`**：位置记忆/首开居中/Docked 不记忆/440 宽。
- **core 新增**：`copy_layers_between` 用例（预算、烘焙分支、重映射、平移、空目标建文档、undo 名）。
- **app 集成**：两阶段流（save 面板续接、confirm_quit 中止、receive 队列 drain）各一例。
- 全量 `cargo test` 绿；`docs/rust-vs-swift-diff-report.md` 顶部进度节随本轮更新。
