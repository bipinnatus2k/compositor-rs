# 08 · App 层界面与工作区实现审计（sheets / workspace / project / shortcuts / settings / picker / panels / layers / menu / ui）

- 审计日期：2026-09-28
- 分支：`dev-ui-fix`
- **行号基准：commit `d945a6d`（HEAD）+ 当时磁盘内容**。审计进行中（00:49–01:01）有另一进程对 `state.rs / workspace.rs / sheets.rs / sheets_ui.rs / layers_panel.rs / color_picker_ui.rs / tools.rs / canvas.rs / menu.rs` 做 `self.session → self.tab().session` 的多标签机械迁移，已逐 hunk 核对：`sheets_ui.rs` 1528 行以前、`workspace.rs` 936 行以前的行号与迁移后完全一致（迁移为等量替换），其后区域迁移后约 **-6 / -5 行偏移**。`project.rs / tabs.rs / shortcuts.rs / settings.rs / panels.rs / ui.rs / menu.rs(HEAD)` 未被迁移改动，行号即当前磁盘行号。`state.rs` 引用一律标注为工作树行号（最后写入 00:52:49）。
- 方法：11 个目标文件逐行全文阅读；`project.rs` 的对端 `tabs.rs`、辅助的 `state.rs / keys.rs / main.rs / canvas.rs(cancel_active) / tools.rs(局部)` 亦已阅读；所有死代码结论均经 `grep` 全 crate 验证调用点；旧报告 `docs/agent-reports/H-ui-app.md`（2026-09-24）仅作交叉核对。

---

## 总览表

| 文件 | 主要功能块 | 实际状态 |
| --- | --- | --- |
| sheets_ui.rs (2491) | 模态表单渲染 + OK/Cancel + PNG/JPEG 导出 + .comp 存开 + PSD 打开 + 曲线画布 + 快捷键编辑表 | **基本完整但有缺陷**：全部表单确认路径真实写回 session/core；存在静默回退、JPEG 预览死端、死代码块 |
| sheets.rs (869) | 表单打开/滑杆接线/实时预览 | **完整实现**（预览经 core filter 管线 + 选区混合，commit 走 undo；仅低级别问题） |
| workspace.rs (1083) | 全部菜单/快捷键 action 分发 + 状态栏 | **基本完整但有缺陷**：action 全部接到 core 调用；但模态键盘漏 gating、blend-mode 快捷键不重绘画布、Close 无未保存确认 |
| project.rs (1222) | ProjectController 两阶段文件流（存/开/导出/确认/导入队列） | **完整实现但（HEAD 时）UI 不可达**：模型层+测试完备，AppState 未接；工作树迁移正在接线 |
| tabs.rs (1195，补充阅读) | ProjectWorkspace 多标签模型 | 同上：完整、经测试、HEAD 时未接 UI |
| shortcuts.rs (1180) | 快捷键注册表 + 校验 + JSON 持久化 | **完整实现**（经 keys.rs:73-74 真实驱动 keymap，启动加载 `%APPDATA%\compositor\shortcuts.json`） |
| settings.rs (597) | 工具开关持久化 tool-defaults.json | **完整实现**（首帧 apply_startup_defaults + 每次切换 persist_session + JPEG 质量持久化） |
| color_picker_ui.rs (952) | SV 场 + 色相条 + hex 取色器 | **基本完整**（真实读写 session、效果色实时预览；3 个幽灵滑杆构造但永不渲染） |
| panels.rs (652) | 浮动面板状态模型 + anchored 渲染壳 | **stub/未接线**：全模块无任何生产调用点（仅自身测试）；文档声称的 "wiring batch" 不存在 |
| layers_panel.rs (417) | 图层面板 | **基本完整但有缺陷**（cmd 点缩略图载入选区等真实；编辑态禁用逻辑被丢弃、Del 按钮语义与 Delete 键不一致） |
| menu.rs (229) | 菜单栏 | **完整实现**：逐项核对，每个菜单项都 dispatch 到 workspace 已注册的 action handler；但无任何禁用逻辑、缺 Save As 入口 |
| ui.rs (194) | 共享 UI helper | **完整实现**（下拉/复选/滑杆均回调 AppState；有陈旧 `#[allow(dead_code)]`） |

浮层原语核对（背景知识验证）：sheet 遮罩 `absolute().inset_0().occlude()`（sheets_ui.rs:204-208）、取色器同（color_picker_ui.rs:344-351）、浮动面板 `anchored().position().snap_to_window()` + `occlude()`（panels.rs:332-336）、下拉菜单走 gpui-component `dropdown_menu`（menu.rs:219-229, ui.rs:92-106）、窗口挂 `Root::new`（main.rs:47-49）。**均遵守 gpui-component 层原语，无裸 div 浮层**。

---

## 逐条发现

### 高

- **[高] 工作树处于未完成迁移，当前快照无法编译** — `crates/app/src/menu.rs:16` 与 `crates/app/src/state.rs:503`。工作树迁移把 `AppState.session` 字段替换为 `pub workspace: ProjectWorkspace`（state.rs:291），但 `menu.rs:16` 仍写 `let session = &state.session;`（字段已不存在）；`state.rs:503` 调用 `state.present_pending(pending, window, cx)`，而 `fn present_pending` 全 crate grep 无定义。迁移仍在进行（menu.rs 最后写入 01:01:53），以磁盘快照为准时本条为编译破坏；以 HEAD `d945a6d` 为准则不存在此问题（HEAD 的 state.rs 仍是 `pub session`）。
- **[高] 模态表单打开时约 40 个 action handler 未做键盘 gating，可在遮罩后修改文档** — `crates/app/src/workspace.rs:978-988`（`hotkey()` 是唯一检查点）+ `state.rs:1633-1638`（工作树；`canvas_keys_active` 在 `sheet.is_some()` 或输入框聚焦时返回 false）。只有经 `hotkey()` 包装的 handler（工具切换 workspace.rs:292-339、删除层 502-519、剪贴板 868-900、nudge 等）受 gating；`on_undo/on_redo`（392-405）、`on_fill_foreground/background`（417-424）、`on_free_transform`（425-433）、`on_select_all/on_deselect/on_invert_selection`（644-655）、`on_new_layer/on_merge_down/on_add_mask`（493-530）、`on_invert_pixels`（850-853）、View 全组（686-752）、`on_remove_background`（915-954）等都不检查。代码自己的注释（state.rs:1631-1633「Single-letter shortcuts and nudges are ignored while a text input or an open sheet owns the keyboard」）证明作者知道绑定会在 sheet 下触发，但只保护了一半 handler。后果实例：Levels 表单打开并预览时按 Ctrl-Z，`session.undo()` 在遮罩后执行，随后 OK 的 `commit_preview`（state.rs 工作树:1494-1518）把基于 undo 前像素的预览写到 undo 后的图层上。Esc/Enter 有专门处理（canvas.rs:1080-1089 `cancel_active` 先行路由到 `cancel_sheet`），不受此条影响。
- **[高] blend-mode 快捷键切换后画布不重绘（revision 未 bump）** — `crates/app/src/workspace.rs:833-840`。`on_blend_mode_previous/next` 只调 `session.cycle_blend_mode(..)` + `cx.notify()`，不调 `touch()`；帧缓存以 `revision` 为键（state.rs 工作树:1379-1390 `frame_image_rev != self.revision` 才重建），因此 Shift+= / Shift+- 改混合模式后面板标签更新、但画布合成图保持旧缓存，直到下一次任何 bump revision 的操作。同模式：`on_cycle_shape_kind`（841-844）同样只 `cx.notify()`（它不改当前画布，影响较小）。对比 `on_zoom_100`（748-752）正确地手动 `self.revision += 1`，说明是遗漏而非设计。

### 中

- **[中] Ctrl-W / File>Close 无未保存确认，直接丢弃文档** — `crates/app/src/workspace.rs:380-388`。`on_close_document` 直接清空 `document/active_layer_id/selected_layer_ids`，不问 `is_modified`。Swift 的 close 流程有确认（project.rs 已移植 `ConfirmStep::Close` 全套，project.rs:45-52、tabs.rs:337-416），HEAD 时 UI 不走它。工作树正在接（state.rs:491-508 仅接了窗口关闭 `on_window_should_close`，Ctrl-W 路径尚未接）。
- **[中] Save/Save As 永远弹路径表单；`save_comp` 不回写 `project_url`，"再次保存"无法原地写盘** — `crates/app/src/workspace.rs:365-370` 两个 handler 都只是 `open_sheet(SheetKind::SaveComp)`；`sheets_ui.rs:1433-1450` `save_comp` 落盘后 `mark_saved()` 但从不设置 `session.project_url`。ProjectController 已实现的「已知 URL 直接写盘不再弹窗」（project.rs:303-324 `save_current`）在 UI 路径完全不可达。连带 `is_modified`/标签标题（tabs.rs:94-101）无法按文件跟踪。
- **[中] JPEG 导出对超大文档形成永久死端：OK 永远禁用且无错误提示** — `state.rs:1178-1180`（工作树）`refresh_jpeg_preview` 在 `check_export_size` 失败时静默 `return`，不写 `jpeg_ready`、不报错；`sheets_ui.rs:133-138` OK 按钮 `disabled = jpeg_ready != (quality, matte)` → 按钮永远禁用，状态行停留 "Updating preview…"（sheets_ui.rs:851-855）。同函数内编码失败/无文档也静默返回（state.rs:1174-1176, 1187-1191）。用户没有任何失败反馈。
- **[中] 表单数值静默回退/静默失败** — `sheets_ui.rs:1172-1173` NewCanvas 宽高 parse 失败静默按 1920×1080 建档；`sheets_ui.rs:1190-1191` + `1201-1206` ImageSize parse 失败得 0，`resize_image_to(0,0,…)` 返回 false 被忽略（core 侧 `resize_image_to` 对非法尺寸拒绝并返回 bool，core/session.rs:5375-5387），用户输入垃圾后点 OK 无任何反应也无提示；`sheets_ui.rs:1242` Amount（Feather/Expand/Contract）parse 失败静默按 0 执行。对比 CanvasSize 有 `max(1)`（1225）但同样不校验上限、不报错。
- **[中] panels.rs 整模块（652 行）无任何生产调用点** — `crates/app/src/panels.rs:69`（FloatingPanels）、`:260-337`（render_panel）、`:212-219`（docked_top_left/docked_size）、`:36-44`（panel_names）。grep 全 crate：仅 panels.rs 自身定义与测试引用。模块文档（panels.rs:12-14）称 render_panel 是 "the wiring batch feeds sheet content into" 的容器，该 wiring 从未发生；Swift 浮动面板（非模态、位置记忆、Camera Raw 停靠）在 UI 中不存在，所有编辑器仍是模态居中 overlay。状态模型本身经测试且逻辑正确，但按「界面上存在但点了没效果/根本不在界面上」口径这是最大的悬空 UI 层。
- **[中] `AppState.subscriptions` 只增不减，长会话泄漏** — `state.rs:339`（字段）、push 点 708/721/731/747（常驻滑杆）、971（取色器幽灵滑杆×3）、1117/1141（文本/变换输入）、1455（重命名）；`sheets.rs:90`（每次开表单按滑杆数 1–14 条追加）。全文件无 `clear()/retain()/remove`。每次开表单、开取色器、重命名层都永久留存订阅（及其闭包捕获），无回收路径。
- **[中] 图层行 "Del" 按钮与 Delete 键语义不一致** — `crates/app/src/layers_panel.rs:175-180` 页脚 Del 直接 `delete_active_layer()`；而 Delete 键走 workspace.rs:502-519 的 `deleteLayerOrMask` 语义（蒙版选中且单选→删蒙版，多选→删多图层）。同一界面两个入口行为不同。

### 低

- **[低] NewCanvas 表单内的 preset 回写是 no-op 死代码** — `sheets_ui.rs:327-331`：`preset` 刚从 `sheet.preset` 读出（240），`if preset != 0 { sheet.preset = preset }` 把读到的值原样写回，无任何效果。
- **[低] `AppState::levels_auto` 无调用者（死代码）** — `crates/app/src/sheets.rs:554-556`。UI 实际调 `levels_auto_mode(LevelsAuto::Contrast, …)`（sheets_ui.rs:487-493），包装函数 grep 全 crate 无调用点。
- **[低] project.rs 死代码四件** — `crates/app/src/project.rs:31` `UNSAVED_CHANGES_INFO` 常量无引用；`:247-249` `can_start` 无调用者；`:146-155` `Step::done` 无调用者；`:85-90` `Pending::Confirm.tab` 字段永远为 `None`（构造点 581-587、643-649 均 `tab: None`）。
- **[低] `ShortcutRegistry::conflicts()` 生产路径未用，文档声称的用途不成立** — `crates/app/src/shortcuts.rs:455-478` doc 称 "so a settings sheet can flag each row"，但快捷键表单只用 `validate()`（sheets_ui.rs:1712、141）；`conflicts()` 仅测试调用（shortcuts.rs:787/1002/1021）。
- **[低] TextEditing 组是空目录、对应校验分支不可达** — `crates/app/src/shortcuts.rs:37`（Group::TextEditing 定义）+ `:417-424`（TextEditingNeedsModifier 校验）：CATALOG（70-186）无任何该组成员，快捷键表单按组遍历（sheets_ui.rs:1720-1723）时该组永远不出现；测试已注明是保留规则（shortcuts.rs:1040-1044），属文档化的死分支。
- **[低] 取色器的 3 个滑杆构造+订阅但永不渲染（幽灵控件）** — `crates/app/src/color_picker_ui.rs:79-101`（构造）、`state.rs:940-975`（工作树；`build_picker_ui` 还为它们各建一条订阅）；模块 doc（color_picker_ui.rs:9-13）已声明此取舍。它们是 `PickerUi` 形状兼容的遗留，且是上面 subscriptions 泄漏的来源之一。
- **[低] layers_panel 计算后丢弃 `can_edit`，编辑态控件不禁用** — `crates/app/src/layers_panel.rs:50` 计算 `can_edit_appearance()`、`:195` `let _ = (&app, can_edit);` 显式丢弃；Blend 下拉与 Opacity 滑杆在不可编辑状态仍可操作（靠 session 内部兜底）。菜单层同理：`menu.rs` 全文无一处 `.disabled`，与 Swift 每项精细 disable 不符（旧报告 H 该点仍成立）。
- **[低] 不透明度滑杆只在点击行时回同步** — `crates/app/src/layers_panel.rs:29-36`（sync_opacity_slider）唯一调用点是 `:268`（行点击）。经 打开工程/删除层等路径切换活动层后，滑杆拇指停在旧值（旁边的百分比文本按帧重算所以是对的），出现滑杆与数字不一致直到再点一次行。
- **[低] 陈旧的 `#[allow(dead_code)]`（注释说死但代码活）** — `crates/app/src/ui.rs:84-89` 给 `MenuEntry::disabled` 标 dead_code，但它实际被 `layers_panel.rs:210` 使用；同文件 `BG_CONTROL`（23-24）与 `app_bg()`（29-32）的 allow 则属实（grep 无使用）。
- **[低] 快捷键录制中按 Esc 会被录制成 Esc 快捷键而非取消录制** — `crates/app/src/sheets_ui.rs:1600-1617`（`recorder_capture("escape")` → Finish）+ `:1836-1870`（录制中吞键）。已核对 Swift 原版 `ShortcutRecorder.keyDown`（Compositor/Compositor/UI/KeyboardShortcuts.swift:310-317）：Swift 同样对 key.count==1 的一切键（含 Esc）finish，故为**忠实移植**，仅作为行为备注，不算缺陷。
- **[低] 取色器 hex 无回车提交** — `crates/app/src/color_picker_ui.rs:118-139`：hex 只在点 OK 时读取提交（Swift 是输入框 submit 即 commitHex）；输入合法 hex 后必须点 OK。功能可用，交互差异。
- **[低] 曲线新增点 x 范围注释与代码不一致** — `sheets_ui.rs:1922-1934`：注释说 "x in 2…253"，代码是 `x <= 1.0 || x >= 254.0` 拒绝（即允许 1<x<254，如 1.5）。测试（2332-2340）按代码语义断言。纯文档漂移。
- **[低] `save_comp`/`export_png` 等以相对路径写 cwd** — `sheets.rs:316-346` 默认路径 `export.png`/`./project.comp`/`./image.png`，确认框直接 `std::fs::write`（sheets_ui.rs:1377, 1427, 1440），依赖进程 cwd，无原生文件对话框（旧报告 H 该点仍成立；两阶段控制器里的原生面板方案未接 UI）。

---

## 死 UI 清单（界面上存在/已声明，但点了无实际效果或不可达）

1. **Window > Reset Panel Layout** — `menu.rs:210`（菜单项）→ `workspace.rs:756-758`：handler 只 `set_status("Panel layout is fixed in this build")`，无任何布局操作。空实现。
2. **Move 工具头 Auto Select 勾选框** — `tools.rs:587-591`：写入并持久化 `transform_auto_select`（settings.rs 键 `autoSelect`），但该标志无任何 canvas 拾取代码读取（state.rs 工作树:348-352 注释自认 "until it is wired into canvas picking"）。勾选/取消/重启均无行为效果。
3. **File 菜单无 Save As 入口（反向死 UI）** — action 与键真实存在（`keys::SaveAs`，shortcuts.rs:96 `ctrl-shift-s`，handler workspace.rs:368-370），但菜单 File 组（menu.rs:73-84）没有 "Save As…" 项，鼠标用户不可达。
4. **浮动面板整套 UI（应承载 Levels/Effects/Camera Raw 等的非模态面板）** — `panels.rs` 全模块无生产调用（见中级别条），用户界面上不存在该特性。
5. **取色器 H/S/B 三滑杆** — `color_picker_ui.rs:79-101` 构造、`state.rs:940-975` 订阅，永不渲染（文档化取舍，仍是死控件+死订阅）。
6. **快捷键表单的 "Text Editing" 分组** — `sheets_ui.rs:1720-1723` 遍历 `Group::TextEditing`，但 `shortcuts.rs` CATALOG 无成员，该组标题与行永远不出现（Swift 有该组）。
7. **模型层不可达 UI 流（HEAD 时点）** — 未保存确认三键弹窗（project.rs:654-751 + tabs.rs:337-573 的 Close/Quit 询问）、保存面板直写路径（project.rs:303-324）、跨标签图层拷贝（tabs.rs:651-724）、多标签/New Canvas 新标签流（tabs.rs:215-252）：实现+测试齐全，但 HEAD 的 AppState/UI 不调用；工作树迁移进行中（`SheetKind::Confirm/Alert`、`pending_save` 已加入 state.rs:104-109/369，`on_window_should_close` 已接 state.rs:495-508，`present_pending` 未写完）。

---

## 与旧报告 H（docs/agent-reports/H-ui-app.md，2026-09-24）的矛盾点

H 报告早于近 4 天的大量落地（wave 1/2），以下关键论断与当前代码矛盾（H 位置 → 现状）：

1. **H §1.3（54-56 行）：「快捷键自定义子系统整体缺失，只有静态 SHORTCUTS_TEXT」** → 已完整落地：`shortcuts.rs`（注册表/校验/冲突/JSON 持久化，CATALOG 92 项 shortcuts.rs:70-186，持久化路径 602-604）+ 快捷键编辑表单带录制器（sheets_ui.rs:1560-1870）；`SHORTCUTS_TEXT` 已不存在（grep 0 命中）；keymap 真由注册表生成（keys.rs:73-74 `bindings()` → `lock_global()`，main.rs:19 `bind_keys`）。H 同表内约 20 条「缺失键位」（Save As 21、Export JPEG 22、Close 23、剪贴板 25、Curves 30、Invert 31、Clipping 32、⌘方向 41、⌘⏎ 42、⌥方向 43、⌥P 44、Space 45、数字键 40、混合模式 38、形状 39、硬度 37 等）全部已实现（workspace.rs:292-974 各 handler + shortcuts.rs:70-186 绑定 + state.rs canvas_key_down/space_down/toggle_levels_preview）。
2. **H §2（62-84 行）：菜单大面积缺失** → 现菜单（menu.rs）已含 H 列为缺失的：Import Images(81)、Export JPEG(79)、Curves/Levels/HueSat(104-106)、B&W/ColorBalance/Exposure/GradientMap/Grain(107-111)、Invert(112)、Fit/100%/Zoom In/Out(185-188)、Snap 总开关(196)、Show Transform Controls(190)、Lock/Clear Guides(206-207)、Cut/Copy/CopyMerged/Paste/Clear/Content-Aware(89-97)、Layer's Pixels/Mask's Black Areas(167-168)、New Adjustment Layer 子菜单(124-137)、Edit Adjustment(138)、Clipping(143)、Rename(148)、Show/Hide Layer(149)、Move Up/Down(151-152)、动态 Merge/Delete 标题(154/160，逻辑 48-61)。仅 Save As 菜单项仍缺（见死 UI #3）。
3. **H §3.1（103-105 行）：「Gaussian 默认 4.0 / Motion 默认 20 / Noise 默认 12」** → 现在滑杆初值取 `FilterSettings::default()`（sheets.rs:260-307），且有测试断言 radius=1.0、distance=10.0、amount=10.0（sheets.rs:862-867），与 Swift 一致。
4. **H §3.1（108 行）+ §6：「Export JPEG 缺失、质量记忆缺失、无任何 UI 状态持久化」** → JPEG 导出带实时编码预览/字节数/质量记忆全在（sheets_ui.rs:837-907、1250-1275；state.rs 工作树:1162-1204；质量持久化 sheets_ui.rs:1262-1264 写 / sheets.rs:323-330 读）；工具开关持久化（settings.rs 全文件，启动加载 workspace.rs:21-24）与快捷键持久化均在。
5. **H §3.2（113-115 行）：「ColorPickerSheet / EffectsSheet / CurvesControls 完全没有」** → color_picker_ui.rs 全实现（SV 场 380-424、色相条 426-480、hex+预览 270-332）；Effects 表单（sheets.rs:195-256 + sheets_ui.rs:551-617，每滑杆一次真实 undo）；曲线编辑器（sheets_ui.rs:622-722 + 曲线画布 1963-2087）。
6. **H §3.4（142 行）：「底部 Layer Effects 菜单缺失」** → 已有：layers_panel.rs:162-168（Fx 下拉）+ effects_menu_entries（201-213，含 `can_edit_effects` 禁用）。
7. **H §3.4（141 行）：「⌘点缩略图载入选区缺失」** → 已有：layers_panel.rs:300-338（Ctrl/Cmd+点缩略图载入图层/蒙版像素，Shift 加选、Alt 减选）。
8. **H §7（175 行）：「app crate 0 个 #[test]」** → 现 app 9 个文件含测试模块（sheets.rs:763-868、sheets_ui.rs:2254-2490、shortcuts.rs:663-1180、settings.rs:340-597、project.rs:886-1222、tabs.rs:745-1195、state.rs:1685-1815、color_picker_ui.rs:805-952、panels.rs:412-652）。
9. **H §3.3（125 行）：「Auto-select 勾选框 handler 为空、不做事」** → 部分更正：tools.rs:587-591 现在写入 + 持久化 `transform_auto_select`，但 state.rs:348-352 注释承认仍未接入 canvas 拾取——**行为上仍然无效**（本报告死 UI #2），H 的结论在行为层仍成立，机制层已过时。
10. **H §5（167 行）：「无关闭确认、is_modified/mark_saved 未接 UI」** → HEAD 仍成立（workspace.rs:380-388）；但工作树已在接入（state.rs:491-508 窗口关闭走 `close_window` 确认流、`SheetKind::Confirm/Alert`、`mark_saved` 已在 save_comp/open_document 调用 sheets_ui.rs:1444/1469）。属于「正在矛盾中」的条目。
11. **H §1.3 引用 `sheets_ui.rs:1028`、§8 引用 `sheets_ui.rs:608/166-211` 等**：行号全部漂移（文件已从 H 时的规模长到 2491 行），H 的行号引用整体不可再用。

仍与 H 一致（未反转）的要点：Reset Panel Layout 空实现（H §2:84 ≈ 本报告死 UI #1）；无原生文件对话框、路径文本框方案（H §3.1:108-109）；菜单项无禁用逻辑（H §2:86）；图层列表无多选/拖拽/右键菜单（H §3.4:141）；NewCanvas 无尺寸上限（H §3.1:96，本报告中级条亦覆盖）。

---

## 总结论

1. **接线密度远高于旧报告 H 所描述的「骨架」状态**：11 个文件里没有发现「返回固定值的假实现」；表单提交（sheets_ui.rs `confirm_sheet` 全分支）、快捷键（注册表→keymap）、设置持久化（启动加载+即改即写）都是真写回 session/core 的。H 报告中 20+ 条「完全缺失」已反转，其结论只能当 2026-09-24 的快照看。
2. **当前最大的三处实坑**：① 模态表单对键盘不设防（仅 `hotkey()` 包装的 handler 检查 sheet/输入焦点，Ctrl-Z 可穿透 Levels 预览改文档）；② blend-mode 快捷键因漏 `revision += 1` 导致画布缓存不失效；③ 关闭文档无未保存确认（模型层的确认机已写好并测好，只差接到 Ctrl-W）。
3. **架构悬空**：`project.rs + tabs.rs`（约 2400 行，模型完整、测试充分）在 HEAD 与 UI 完全脱节；`panels.rs`（652 行）无人调用。工作树的多标签迁移正在把前者接上（迁移未完：menu.rs、present_pending 两处编译断点），后者仍无接线迹象——浮动面板特性在 UI 上不存在。
4. **死代码集中在模型层边缘与文档化取舍**：`levels_auto`、`UNSAVED_CHANGES_INFO`、`can_start`、`Step::done`、`Pending::Confirm.tab`、`conflicts()`（生产）、`subscriptions` 无回收、取色器幽灵滑杆。无 panic 风险级别的 unwrap（`expect` 均有不变量保护），错误处理以状态栏文本为主、两处静默吞错（JPEG 预览失败、resize 返回值忽略）已列为中级别。
