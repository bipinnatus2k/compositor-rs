# 07 — App 层状态与画布实现审查（state / canvas / tabs / tools / keys / 入口）

- 审查日期：2026-09-28
- 分支：dev-ui-fix
- 方法：逐文件完整读取（非抽样）+ 全工程 grep 调用点验证 + `cargo check` 编译验证。所有结论来自亲自读取的代码，注释仅作对照物。
- **快照声明（重要）**：审查期间工作树正被**并发修改**（同一批文件在审查过程中多次变化，例如 state.rs 先后出现 1759 / 1816 / 1824 行三个版本，编译错误从 7 个被并发修复到 1 个）。本报告全部行号固定于以下快照：
  - `crates/app/src/state.rs` 1824 行，md5 `2c02296c881b8dcd4f15705d920b1ca8`
  - `crates/app/src/canvas.rs` 1880 行，md5 `df4e95acec768c7bf7cfb9a719d0caf9`
  - `crates/app/src/tools.rs` 1103 行，md5 `b756f980901b400638a99ec567d5ba18`
  - `crates/app/src/tabs.rs` 1221 行，md5 `6cc836721592f8448d6ffee93e88b746`
  - `crates/app/src/keys.rs` 222 行，md5 `e8412f2e2d03badd168e1be26e9b5580`
  - 后续代码继续演进时，±1~10 行的漂移是可能的，但各发现的调用点均已用内容级 grep 复核。

## 总览表

| 文件 | 主要功能块 | 实际状态 |
| --- | --- | --- |
| state.rs | 坐标换算（doc↔screen / zoom clamp） | 完整实现（含 12 个单元测试，state.rs:1698-1817） |
| state.rs | 帧缓存 / 缩略图缓存 | 基本完整但有缺陷（按 revision 失效，缩放/平移也触发全量重编码，见 #11） |
| state.rs | 视口 fit（zoom_fit / request_refit / fit_tab） | 部分（`request_refit` 死代码、`fit_tab` 只写不读，tab 切换不 refit，见 #8） |
| state.rs | Space 手型 / 数字键透明度 / 文本间距 / ⌘回车 | 完整实现（state.rs:519-686），真实接到 core |
| state.rs | 滑杆↔session 同步 | 完整实现（state.rs:690-748） |
| state.rs | 内容识别填充 / Camera Raw 三件（吸管/读数/自动白平衡） | 部分：填充真实；**吸管链路不可达、读数与自动白平衡为死代码**（#5、#6、#7） |
| state.rs | JPEG 导出实时预览 | 完整实现但同步编码阻塞 UI（#12） |
| state.rs | Confirm/Alert sheet + pending_save（未保存确认流） | **stub/半成品**：枚举与字段已加、`close_window` 已接，但 `present_pending` 未定义导致整个 crate 编译失败（#1、#2） |
| canvas.rs | 场景构建与绘制（棋盘格/网格/参考线/选区/裁剪/变换框/标尺） | 基本完整（标尺无数字标签，#19；蚂蚁线无动画，#20） |
| canvas.rs | 指针交互（平移/缩放/绘制/渐变/形状/移动/裁剪/参考线/像素搬移/选区搬移） | 基本完整但有缺陷（双击吞事件 #13、裁剪手柄容差不随 zoom #14、吸附容差 #15、滚轮丢横向分量 #16） |
| canvas.rs | 键盘交互（install_key_events → state.canvas_key_*） | 完整实现，真实接到 core 调用 |
| canvas.rs | Warp（涂抹/液化） | 基本完整但**忽略蒙版**，与其注释相悖（#17） |
| canvas.rs | Type 工具 | 部分：无画布文本输入，点击即落默认内容（#18） |
| tabs.rs | ProjectWorkspace 模型（标签/打开/关闭/退出/接收/跨项目拷贝） | 模型层完整且测试充分（773-1221 行约 20 个测试），**但除 `close_window` 外与 UI 完全未接线**（#3） |
| tools.rs | 工具栏 / 每工具选项头（含 Transform 检查器、Type 头） | 基本完整；形状头无线宽/圆角滑杆（低） |
| keys.rs | 动作目录 + 键位绑定（shortcuts 注册表驱动） | 完整实现；大量 action 有 handler/菜单但无默认键（低，见 #22） |
| main.rs (crates/app) | 窗口装配 | 完整实现（单窗口） |
| lib.rs | 模块声明 | 完整 |
| src/main.rs（根） | — | **孤儿文件**："Hello, world!"，不属于 workspace 任何成员，从不构建（#23） |

---

## 发现清单

### 高严重度

- **[高] 工作树无法编译：调用未定义的 `present_pending`** — `crates/app/src/state.rs:503`（调用点）、`state.rs:491-508`（on_window_should_close 闭包）。`cargo check` 实测：`error[E0599]: no method named 'present_pending' found for mutable reference '&mut AppState'`，`compositor`（app lib）编译失败。该方法在整个 crate 中只有这一处调用、没有定义。审查早前快照还曾同时存在 canvas.rs:411/448/451 的 E0499 借用冲突、layers_panel.rs:54/230 的 E0500/E0596、sheets.rs:134 的 E0004（`SheetKind::Confirm`/`Alert` 未覆盖），这些在审查过程中被并发编辑陆续修掉，但截至快照仍剩此 1 个错误。**当前工作树上的画布交互与全部 UI 无法运行验证**。根因是"未保存确认流"（SheetKind::Confirm/Alert + pending_save/SaveAfter，state.rs:104-108、152-156、366-381）只加了一半。
- **[高] 未保存确认流是半成品，退出/关闭流程无法完成一次确认** — `crates/app/src/state.rs:496`（`workspace.close_window()` 已接）、`crates/app/src/tabs.rs:466-478`（close_window → advance_quit）、`tabs.rs:524/550`（`resolve_quit`/`quit_after_save`）、`tabs.rs:364/392`（`resolve_close`/`close_after_save`）。grep 全工程：`resolve_quit`、`quit_after_save`、`resolve_close`、`close_after_save` 在 tabs.rs 之外**没有任何调用者**——弹出的确认框没有任何代码能把它的事件送回模型（本应由缺失的 `present_pending` + Confirm sheet 解析逻辑承担）。即窗口关闭时若触发未保存确认，流程将挂死在 Pending 态。
- **[高] tabs.rs 的标签页模型与 UI 完全脱节（约 1100 行孤岛）** — `crates/app/src/tabs.rs:232`（`select`）、`:258`（`open`）、`:289`（`load_project`）、`:337`（`close`）、`:449`（`confirm_quit` 菜单退出路径）、`:581`（`receive`）、`:598`（`drain_incoming`）、`:651`（`copy_layer`）、`:731`（`can_receive_drag`）。grep 全工程生产代码：上述方法**零调用**（仅在 tabs.rs 自身测试中出现）；`ProjectWorkspace::select()` 没有任何 UI 调用点，workspace.rs/panels.rs 中不存在标签条渲染（grep `tabs()`/tab_bar 无命中）。唯一接进 UI 的是 `close_window`（state.rs:496）。文件的打开/保存实际由 sheets_ui.rs:1452 直接调用 `compositor_core::project::save`，完全绕过 `ProjectController`。tabs.rs 头部注释（tabs.rs:6-9）声称"the app presents and drains them"，实际 app 侧的接收/呈现层不存在。
- **[高] Camera Raw 去边吸管（Defringe Eyedropper）链路不可达** — `crates/app/src/state.rs:322`（`camera_raw_samples_defringe` 声明）、`:461`（初始化 false）、`:1589`（唯一写点，置 false）、`crates/app/src/canvas.rs:231`（唯一读取点）。该标志**从未被置为 true**（没有任何打开 Camera Raw 吸管的代码），因此 canvas.rs:231-235 的吸管采样分支永远不执行，`sample_camera_raw_defringe`（state.rs:1570-1583）事实死代码。整个"点画布中心化去边色域"功能不存在。
- **[高] `update_camera_raw_readout` 与 `camera_raw_readout` 双端死代码** — `crates/app/src/state.rs:1595-1616`（定义，全工程无调用者）、`:324`（字段声明，全工程无读取者）。注释声称实现 `updateCameraRawReadout`（指针下 RGB 读数），实际该功能完全没有接进事件流，也没有任何 UI 显示这个值。注意 canvas.rs:389-391 在 sheet 打开时直接 return，即使接线也不会在 Camera Raw 模态下收到 move 事件，接线时还需绕开该守卫。
- **[高] `apply_camera_raw_auto_white_balance` 死代码** — `crates/app/src/state.rs:1621-1635`。函数体完整（调 core 的 `CameraRawSettings::auto_balance`），但全工程无调用者：Camera Raw 面板上没有"自动白平衡"按钮/菜单项，功能声明存在、入口缺失。

### 中严重度

- **[中] tab 切换不重置视口：fit 机制只实现了一半** — `crates/app/src/state.rs:312-316`（注释声称 "a tab switch refits the viewport too"）、`:1262-1268`（`request_refit`，**全工程无调用者**）、`:316`/`:1256`/`:1264`（`fit_tab` 只写不读）。画布的自动 fit 条件是 `dims != state.fit_for`（canvas.rs:85），只比较文档尺寸。两个文档尺寸相同的 tab 之间切换时，新 tab 继承前一个 tab 的 zoom/pan，与注释及 Swift 的 `.id(tab.id)` 重建行为不符。修复件（request_refit）已写好但没有接到任何 tab 切换路径上——因为 UI 侧根本没有 tab 切换（见高 #3）。
- **[中] 帧缓存与缩略图缓存按 revision 全量失效，交互期每帧重编码** — `crates/app/src/state.rs:1376-1390`（`frame_image`：`frame_image_rev != self.revision` 即重编码整幅 BGRA）、`:1391-1428`（`thumbnail_image`：rev 不匹配即重算 canvas 缩略图并重建 RenderImage）。`zoom_at`/`zoom_fit`/任何 `touch()` 都 bump revision（state.rs:1234-1242、1149-1152），因此**平移和缩放会让像素完全不变的合成图反复重新编码**，多图层文档的每次指针拖动还会同步重渲染所有图层行缩略图。缓存实际只省"静止帧重复绘制"，拖动交互期的开销是 O(文档像素 × 图层数)。frame_image 的 `(tab_id, revision)` 键（state.rs:1376-1387）本身是正确的。
- **[中] 双击吞掉所有工具的第二次点击** — `crates/app/src/canvas.rs:238-248`。`event.click_count > 1` 一律提前 return（仅多边形套索收尾例外）。后果：Zoom 工具快速连点时每隔一次点击被吞（2x 放大变成 1x、0.5x、1x…），Marquee/Brush 等工具的连击首拖也会漏拍。Swift 版只在多边形套索上特判双击。
- **[中] 裁剪手柄命中容差用文档像素且不随 zoom 换算** — `crates/app/src/canvas.rs:1267`（`const HANDLE: f64 = 8.0`，用于 `begin_crop` 的 `handle_at`，canvas.rs:1005-1018）。zoom 0.1 时手柄命中区约 0.8 屏幕像素（几乎无法命中），zoom 32 时约 256 屏幕像素（大面积误命中）。同一文件里参考线命中已经正确做了 `GUIDE_HIT_DISTANCE / zoom` 换算（canvas.rs:29-30、271），两套逻辑自相矛盾。
- **[中] Move 拖动吸附容差 10.0 为文档像素，未随 zoom 折算** — `crates/app/src/canvas.rs:444`（`snapped_move(value, &moving, 10.0)`）。低倍率下 10 doc px 屏幕上极小（吸附几乎不触发），高倍率下过大（乱吸附）。与参考线的处理方式不一致。
- **[中] 滚轮事件丢弃横向分量，且任何滚动都解释为缩放** — `crates/app/src/canvas.rs:524-548`。`ScrollDelta::Pixels/Lines` 只取 `pt.y`（540-543），触控板双指横滑、横向滚轮完全无效；同时没有"滚动=平移"的模式（Swift/Photoshop 常规是无修饰键滚动平移、修饰键缩放）。另外 532-539 的命中测试不排除标尺条带（`RULER_SIZE`），在标尺上滚动也会缩放。
- **[中] 涂抹/液化（Warp）忽略蒙版，与其自身注释相悖** — `crates/app/src/canvas.rs:755-757`（注释："the stroke works on the active layer **as the canvas shows it**"）vs `:769-771`（`draw_layer_pixels(&image, &layer.transform, &mut surface)`，直接画原始像素，未乘蒙版覆盖）。对比同文件 `sample_for_clone`（canvas.rs:689-707）对克隆取样认真做了蒙版coverage 乘法。后果：在带启用蒙版的图层上涂抹/液化，会把被蒙版遮住的内容一并显示、并把整幅未蒙版结果烙进图层像素（`finish_warp` canvas.rs:783-826 直接 commit 层像素）。
- **[中] Type 工具无画布文本输入：点击即落地默认内容** — `crates/app/src/canvas.rs:1021-1039`。`create_text_layer` 在 `begin_text` 后立刻用 `text_defaults.content` 填充空草稿并 `apply_text`。注释自认 "Without a text input UI a click lays down the default sample content"。用户唯一能改内容的方式是 `Edit Text` 复用草稿（tools.rs:777-789）——但同样没有画布输入框，正文内容事实上只能保持默认样例。与旧报告 H 时代相比排版引擎是真字体（tools.rs:878-916 字体目录），但"输入文本"这一核心交互缺失。
- **[中] Space 临时手型没有失焦兜底** — `crates/app/src/state.rs:519-537`（space_down）、`:678-686`（canvas_key_up）。keyup 依赖窗口级 key 事件（canvas.rs:1861-1881 在 paint 期间注册）；若按住空格时窗口失焦/切走，keyup 丢失，`space_saved_tool` 永不清除，工具卡在 Hand。无超时或 focus-lost 清理。
- **[中] JPEG 导出预览在 UI 线程同步编码** — `crates/app/src/state.rs:1159-1204`（`refresh_jpeg_preview`：每次质量/背景选项变化即 `draw_live_composite` 全图合成 + `jpeg_data_with_dpi` 编码 + 缩略图，全部同步）。大文档上拖动质量滑杆会明显卡顿。正确性方面实现是完整的（`jpeg_ready` 与选项比对禁用 OK、`jpeg_data` 复用避免二次编码，state.rs:1171-1201），仅性能问题。
- **[中] Ctrl 拖拽像素搬移失败时静默吞掉手势** — `crates/app/src/canvas.rs:314-320`。`begin_pixel_move(alt)` 返回 false 时直接 `return`，既不回退到普通选区拖拽也不给状态提示，用户按住 Ctrl 在选区内拖动会偶发"没反应"。

### 低严重度

- **[低] `ruler_label` 死代码；标尺只有刻度没有数字** — `crates/app/src/canvas.rs:1631-1639`（定义 + `#[cfg(test)]` 引用 1644/1710-1713），`paint_rulers`（canvas.rs:1569-1615）只画 6px 刻度线，从不调用 `ruler_label`，也没有任何文本绘制。函数及其 doc 注释（"Whole-pixel tick label (`CanvasRulers.label`)"）描述了一个不存在的 UI 能力。
- **[低] 选区"蚂蚁线"是静态虚线，无行进动画** — `crates/app/src/canvas.rs:1326-1333`（dash 4/4，无相位参数）。注释自称 "marching ants"，实际蚂蚁不跑。选区/套索的实时预览本身是真实绘制（build_scene canvas.rs:151-168 接 core `displayed_selection()`），此条仅动画缺失。
- **[低] `cancel_rename` 死代码（诚实标注）** — `crates/app/src/state.rs:1470-1478`，带 `#[allow(dead_code)]`（1470 行）。Escape 链（canvas.rs:1080-1129 的 `cancel_active`）只 `take()` rename 状态，不走此函数。
- **[低] 画布视口无旋转** — state.rs:294-296 仅有 `zoom`/`pan`，全文件无视口 rotation 概念（旋转只存在于图层 transform）。任务关注点中"旋转"在视口层面不存在（Swift 是否有视口旋转未核实，仅记录缺失事实）。
- **[低] Marquee 拖动中 Option 居中生长不生效** — `crates/app/src/canvas.rs:422`：`drag_marquee(doc, shift, false)`，from_center 硬编码 false；`alt` 在 395 行取了但未传。
- **[低] 100% 缩放按钮不保持视图中心** — `crates/app/src/tools.rs:1091-1095`：直接 `zoom = 1.0; revision += 1`，pan 不动，视图以文档左上角为锚跳变。
- **[低] 根目录 `src/main.rs` 是孤儿 "Hello, world!"** — `src/main.rs:1-3`。根 `Cargo.toml` 的 `workspace.members = ["crates/core", "crates/app"]`，根目录没有 package 清单，此文件不属于任何编译目标、从不构建，纯遗留误导。
- **[低] keys.rs 大量 action 无默认键位（但非死代码）** — `crates/app/src/keys.rs:17-62` 声明的动作中，TrimSheet/ImportImages/AddMask/FlipLayer*/MoveOutOfFolder/RenameLayer/ToggleLayerVisibility/EditAdjustment/ClearSelectionPixels/LoadLayerSelection/LoadMaskSelection/FilterBlackWhite/FilterColorBalance/FilterExposure/FilterGradientMap/FilterGrain/RemoveBackground/ClearGuides/AddAdjustment*（12 个）/FilterGaussianBlur/FilterMotionBlur/FilterAddNoise/FilterLensCorrection/FilterCameraRaw/TogglePixelGrid/SnapGuides/SnapGrid/SnapLayers/SnapBounds/CropToSelection/FlipCanvas*/NewGroup/ResetPanels/ShowShortcuts/About 未出现在 `bindings_tracked`（keys.rs:86-221）的绑定表中；但 grep 证实它们全部有 handler（workspace.rs:377-955）且经菜单分发（如 menu.rs:81 的 Import Images），可触发。keys.rs:64-66 的 doc（"Every key binding, bound globally"）措辞容易让人以为所有 action 都有键。
- **[低] `receive` 被守卫阻塞时返回值与"全部成功"不可区分** — `crates/app/src/tabs.rs:581-593`：入队时返回 `Vec::new()`，调用方无法区分"无失败"与"被排队"。当前无 UI 调用者，影响为 0，属 API 设计隐患。
- **[低] 文档漂移：`transform_auto_select` 注释过时** — `crates/app/src/state.rs:348-352` 声称 "carries it until it is wired into canvas picking"，实际 canvas.rs:963-1001（`begin_move`）与 1172-1224（`transform_press_layer`）已经完整接线（含 Cmd 拾取、auto-select 取上层、空画布回退）。行为存在，注释否认它。
- **[低] 冗余乘法与可疑性能写法（记录性）** — `crates/app/src/canvas.rs:1582/1597`（`value * scene.zoom <= scene.doc.0 * scene.zoom` 两侧同乘 zoom，等价于不乘，虽无害但令人误解）；`paint_checkerboard`（canvas.rs:1434-1464）最多 500×500 格逐 quad 绘制，极端视口下单帧 25 万个 quad。

### 明确澄清（检查过、非问题）

- `screen_to_doc_at`/`doc_to_screen_at`（state.rs:35-46）互为逆运算，测试覆盖到 MIN/MAX_ZOOM 极端（state.rs:1735-1751），无坐标系换算错误。
- 参考线创建/拖动/释放删除链路真实接 core：canvas.rs:259-267（标尺按下 `begin_guide_creation`）、457-464（拖动 `move_guide_drag`）、509-515（释放 `finish_guide_drag(delete)`），且命中容差正确按 zoom 换算（canvas.rs:271）。
- 眼管取色真实接 core `sample_composite_color`（canvas.rs:360-370），像素级取色落在 `brush_settings` 并写状态栏。
- 像素搬移/选区轮廓搬移/裁剪/变换拖拽的 mouse-down→move→up 全链路真实接到 core（canvas.rs:294-328、427-464、470-520），非 UI 空转。
- 空格手型、数字键透明度（`type_opacity_digit`，state.rs:540-573 + 664-671）、Option 方向键文本间距（state.rs:576-601、640-665）、⌘回车完成文本（state.rs:601-609、624-628）全部真实实现并有 core 支撑。
- keys.rs 未绑定的动作均有 handler 且可经菜单触发（见低 #22），不是死代码；`bindings_tracked` 机制（keys.rs:86-101）专用于防目录漂移。

---

## 与旧报告 H（docs/agent-reports/H-ui-app.md，2026-09-24）的矛盾点

共 22 条实质矛盾（旧报告论断 vs 当前代码）。最重要的一条总体判断：**旧报告列出的"缺失"清单中约七成已在当前代码落地**，旧报告只能作为 2026-09-24 时点快照，不可再作为事实来源。

| # | 旧报告论断（位置） | 当前代码事实（行号） |
| --- | --- | --- |
| 1 | §1.1：不透明度数字 0–9「缺失（app 中无任何数字键处理，已 grep 验证）」（H:40） | `state.rs:540-573` `opacity_digit`（含两位精确输入节奏）、`state.rs:664-671` 数字键分发 + 滑杆回同步 |
| 2 | §1.1：临时手型 Space「缺失（keys.rs/canvas.rs 均无 space 处理）」（H:45） | `state.rs:519-537`（space_down/up）、`state.rs:614-686`（canvas_key_down/up）、`canvas.rs:1861-1881`（install_key_events） |
| 3 | §1.1：剪贴板四件套「无任何剪贴板动作/键」（H:25） | `keys.rs:21`（Cut/CopyPixels/CopyMerged/PastePixels 声明）、`keys.rs:138-141`（四条绑定） |
| 4 | §1.1：Export JPEG「完全缺失（无动作无 UI）」（H:22） | `keys.rs:17/131` + `state.rs:1159-1204` 完整实时预览编码（质量记忆由 shortcuts/设置层承担，见 tools/sheets） |
| 5 | §1.1：Content-Aware Fill「完全缺失」（H:28） | `keys.rs:145` + `state.rs:782-824` `run_content_aware_fill`（真调 `FilterKind::ContentAwareFill` 并入 undo） |
| 6 | §1.1：Curves「缺失（无入口）」（H:30） | `keys.rs:150` + `state.rs:164-187` CurvesEditor + `state.rs:842-849` apply_curves_preview |
| 7 | §1.1：⌘⏎ 完成文本「缺失（Rust 文本层无编辑状态）」（H:42） | `state.rs:601-609` finish_text_edit、`state.rs:624-628` 键分发、tools.rs:752-789 Done/Edit Text 按钮 |
| 8 | §1.1：Tracking/Leading ⌥方向「缺失」（H:43） | `state.rs:576-601`（adjust_text_tracking/leading）+ `state.rs:640-665`（含 Shift ×10） |
| 9 | §1.1：⌥P Levels 预览「缺失」（H:44） | `state.rs:827-836` toggle_levels_preview + `keys.rs:217` 绑定 |
| 10 | §1.1：画笔硬度 ⇧[/⇧]「缺失」（H:37） | `state.rs:766-778` change_brush_hardness（25% 步进）+ `keys.rs:209-210` |
| 11 | §1.1：混合模式/形状循环「未绑定/缺失」（H:38-39） | `keys.rs:214-215`（blend.next/shape.cycle）+ `canvas.rs:1043-1078` cycle_tool_mode（含 Shape 分支） |
| 12 | §1.1：⌘方向移动选中像素 8 条「缺失」（H:41） | `keys.rs:199-206`（NudgePixels* 8 条绑定） |
| 13 | §1.1：Move Layer Up/Down「面板按钮有但无键」（H:33） | `keys.rs:153-154`（layer.move-down/move-up 绑定） |
| 14 | §1.1：Lock Guides「缺失（无 lock 概念）」（H:35） | `keys.rs:159`（view.lock-guides 绑定）；SnapToggle 亦已绑定（keys.rs:158） |
| 15 | §1.3：快捷键自定义子系统整个「缺失」（H:54-56） | `keys.rs:64-101`：shortcuts 注册表 + `shortcuts.json` 用户覆盖 + `bindings_with` 注入点 + 漂移测试钩子（`bindings_tracked`） |
| 16 | §3.3：Move「只有一个不做事的 Auto-select 勾选框（handler 为空）」（H:125） | `tools.rs:583-595`（handler 写状态并持久化）+ `canvas.rs:963-1001` + `canvas.rs:1172-1224`（Cmd 拾取/auto-select/空画布回退，带 5 个单元测试 canvas.rs:1652-1696） |
| 17 | §3.3：Type「只有一行提示文字……5×7 点阵字体」（H:128） | `tools.rs:680-790` 完整 Type 头（字体菜单 882-916 真字体目录、字号/字距/行距输入、色块、对齐、Done/Edit） |
| 18 | §4：标尺拖拽创建参考线「app 无任何调用（grep 验证）」；参考线只读绘制（H:154） | `canvas.rs:259-267`（标尺按下建参考线）、`canvas.rs:457-464`（拖动）、`canvas.rs:509-515`（拖上标尺删除） |
| 19 | §4：标尺「无 label、候选集 12 个、目标 50px」（H:153） | `canvas.rs:1618-1629`：候选集 18 个、目标 70px（label 至今仍缺——该半句仍成立，见低 #16） |
| 20 | §3.2：ColorPickerSheet「完全缺失」（H:113） | `crates/app/src/color_picker_ui.rs` 存在 + `state.rs:913-982` build_picker_ui（HSB 三滑杆 + Hex 输入，经订阅写回 picker） |
| 21 | §5：多项目「整体缺失……无关闭确认……CloseDocument 直接清空 document」（H:167） | `tabs.rs:107-737` 完整 ProjectWorkspace 模型（未保存确认/quit 顺序/去重打开/跨项目拷贝）+ `state.rs:491-508` 两阶段关闭确认（on_window_should_close 拦截）。**半矛盾**：模型与关闭确认已存在，但标签条 UI 与 select/receive 接线仍缺（本报告高 #3） |
| 22 | §7：app crate「0 个 #[test]」（H:7、H:175） | `state.rs:1685-1817`（12 个）、`canvas.rs:1641-1827`（11 个）、`tabs.rs:773-1221`（约 20 个）、`tools.rs:70-114`（3 个） |

另有一条**部分仍成立**的旧论断需保留：H §3.3 形状头「只有形状下拉」——当前 tools.rs:791-832 仍无线宽/圆角滑杆（尽管 session 有 `shape_corner_radius`/`shape_line_width` 且被 canvas.rs:830-843 消费）；H §3.3 Zoom 头无缩放百分比输入框——tools.rs:503-509 仍只有 4 个按钮。H §3.3 画笔直径上限 300 vs Swift 2000 —— 仍成立（state.rs:751-760 clamp 1..300）。

---

## 总结论

1. **当前工作树不可编译**（state.rs:503 引用未定义的 `present_pending`），且这是"未保存确认流"半成品的一部分；该流的事件回传（resolve_quit/resolve_close/quit_after_save/close_after_save）也没有任何 UI 接线。在此修复前，任何关于画布交互行为的运行时验证都不可信。
2. **画布交互主链路（平移/缩放/绘制/渐变/形状/移动/裁剪/参考线/像素与选区搬移/取色/空格手型/数字键透明度）是真实实现**，全部落到 core session/工具调用，不是 UI 空转；坐标系换算正确并有测试。缺陷集中在：双击吞事件、裁剪手柄与吸附容差不随 zoom、滚轮丢横向分量、Warp 忽略蒙版、蚂蚁线无动画、标尺无数字。
3. **Camera Raw 的三个画布联动功能（去边吸管/RGB 读数/自动白平衡）全是死代码或不可达**：状态与函数都在，但既没有触发入口（`camera_raw_samples_defringe` 永远 false）也没有调用者，读数甚至没有显示端。
4. **tabs.rs 是高质量的模型层移植（含约 20 个测试），但除 `close_window` 外与 UI 零接线**：无标签条、无 tab 切换、无文件接收、无跨项目拷贝入口；实际打开/保存绕过 ProjectController 直调 core。同时 `request_refit`/`fit_tab` 半成品导致同尺寸 tab 切换不 refit。
5. 性能方面，帧/缩略图缓存按 revision 全量失效的设计使平移、缩放与滑杆拖动期间产生大量同步重编码（含 JPEG 导出预览的 UI 线程编码），是交互流畅度的最大隐患。
6. 旧报告 H 与当前代码有 22 条实质矛盾，其"缺失"清单已大面积过时；引用它时必须逐条对代码复核。
