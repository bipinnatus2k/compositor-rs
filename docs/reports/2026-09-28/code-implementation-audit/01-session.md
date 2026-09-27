# session.rs 实现情况审计（2026-09-28）

- 审计对象：`crates/core/src/session.rs`（8580 行，含约 2770 行内联测试），分支 `dev-ui-fix`
- 审计方法：全文逐行通读（Read 分 9 段读完 1-8580），所有结论附实测行号；关键行为差异与 Swift 原版（`Compositor/Compositor/Document/*.swift`）逐段对照；死代码用 grep 在 `crates/app/src`、`crates/core/src`、`crates/core/tests` 全量验证调用点。未采信任何注释/doc/旧报告作为事实来源；旧报告 A 仅用于交叉核对。
- 注：`session.rs:7` 有 `#![allow(dead_code)]`，编译器不会报告下述死代码。

## 〇、总览表（按功能块）

| 功能块 | 主要证据行号 | 实际状态 |
|---|---|---|
| 文档创建 / 项目安装与清空（create/install/clear_project） | 545-689 | 完整实现（与 Swift 逐条对照一致；Swift 同样不重置 `isMaskSelected`，非移植缺陷） |
| 历史 / undo-redo | 581-645 | 基本完整但有缺陷（见 高-4、中-1） |
| 图层操作（增删/改名/显隐/多选/分组/拖放/重排/眼睛滑动/复制） | 691-941, 2887-3326 | 基本完整但有缺陷（守卫缺失、删除不走活蒙版 bake 询问、select_layers 缺 resolve_gradient） |
| 选区会话（lasso/marquee/wand/修改/羽化/反选/载入蒙版与图层选区） | 1195-1693, 1480-1602 | 完整实现（含 NaN 防护、错误上报路径） |
| 选区像素编辑（fill/clear/像素搬移/剪贴板/层经复制） | 1921-2860 | 完整实现（Cmd-drag、Cmd-arrow、duplicate、浮动选区分支均有测试覆盖） |
| 浮动选区变换（Cmd-T） | 4689-4820, 5654-5718 | 完整实现（不变草稿精确还原、merge 失败回滚、Escape 恢复都有实现与测试） |
| 蒙版会话（add/delete/toggle/copy/link/displayed placement） | 1695-1919 | 基本完整（`displayed_mask_placement` 的 group 分支不可达，见 中-3） |
| 活蒙版 / 剪贴蒙版（link/release/toggle/adopt/release/bake） | 3490-3810, 5720-5773 | 基本完整（bake 管线完整，但删除询问路径有 bug 且未接线，见 高-4、高-5） |
| 变换会话（begin/preview/commit/cancel/nudge/distort/mask 变换/翻转） | 4043-4245, 4578-5007 | 基本完整但有缺陷（多选整体变换的 group 管线是死代码，见 中-3） |
| 调整层（add/update/编辑会话 begin/preview/finish） | 2437-2608 | 完整实现（预览写入元数据、Cancel 精确还原、单条 undo 折叠，均有测试） |
| 文本工具（begin/apply/finish/recolor/样式） | 4247-4576 | 完整实现（live_text 身份共享、box 文本、空文本静默、样式回退均有实现与测试） |
| 渐变（begin/move/refresh/apply/resolve） | 3895-4041 | 基本完整但有缺陷（双 sqrt 阈值、提交目标跟随活动层、缺 tool 守卫，见 中-2、低-1、低-2） |
| 参考线 / 吸附 / 裁剪 / 画布与图像尺寸 / Trim | 3812-3893, 5202-5430, 5432-5598 | 完整实现（小瑕疵：重复语句、测试调试残留） |
| 导入管线（位图 / RAW / PSD / 尺寸预算） | 3328-3488, 5775-5797 | 完整实现（同步解码；预算守卫与失败逐文件回报均有） |
| 合并（merge_plan / can / title / merge_layers） | 5009-5200 | 完整实现（三守卫 + `hierarchy_is_valid` 回退都在） |
| 不透明度快捷键 / 画笔硬度 / 调色板（含蒙版黑白） | 1058-1193, 5600-5647 | 完整实现（双键组合 0.6s 窗口、25% 硬度步进与 Swift 一致） |

---

## 一、发现清单

### 高严重度（错误结果 / 数据写错目标）

- **[高] 删除图层绕过活蒙版 bake 询问，依赖层被静默解除剪贴** — `crates/core/src/session.rs:766-768`（`delete_layer`）、`crates/core/src/session.rs:3264-3318`（`delete_selected_layers`，3288-3296 把 `mask_source_id` 直接置 `None`）。Swift 侧 `EditorSession.swift:635-639` 与 `:647-654` 的 `deleteLayer`/`deleteSelectedLayers` 都先走 `deleteWithLiveMaskChoice(ids)`（弹 bake/unlink 询问）；Rust 的主删除路径不做任何检查直接删。app 已接线 `crates/app/src/workspace.rs:515` 调 `delete_selected_layers()`，因此删除剪贴基底时依赖层静默失去剪贴（Swift 会弹窗）。核心虽另备 `delete_with_live_mask_choice`（3756），但全工程无生产调用点。
- **[高] `delete_selected_layers_with_baked` 多选路径删错对象，且 bake 像素落在 undo 步之外** — `crates/core/src/session.rs:3791-3810`。`ids.len() > 1` 时调用 `self.delete_selected_layers()`，而后者删的是 `self.selected_layer_ids`（3271-3276），不是传入的 `ids`——与 Swift `LiveLayerMask.swift:149-153`（逐个 `finishDeletingLayer(id, baked:)` 删传入 ids）语义相反；且 3803-3809 把 baked 像素写在 `delete_selected_layers` 的 `end_edit()` 之后，baked 不在 "Delete Layers" 条目的 after 快照里，undo→redo 后 baked 像素凭空消失。单 id 路径（3796-3801）走 `delete_layer_with_baked` 是对的（baked 在 783-798 的 undo 步内），所以现有测试（7917-7942，只测单 id）测不出。当前 `delete_with_live_mask_choice` 未接线，属潜伏缺陷。
- **[高] 待定渐变可能提交到错误的图层** — `crates/core/src/session.rs:3987-4027`（`apply_gradient` 4003/4006 用 `raster_edit_for_active` 取当前活动层栅格，4022 `commit_pixel_edit` 写入 `self.active_layer_id`），而草稿里记录的 `GradientDraft.layer_id`（152-157, 3918-3923）提交时完全没用。Swift `Gradient.swift:88-100` 的 `commitGradient` 提交的是 begin 时捕获的 `edit.raster`（绑定原图层）。可触发序列：对图层 A 拖渐变 → `delete_layer(A)`（`delete_layer`/`delete_selected_layers` 都不调 `resolve_gradient`）→ 活动层移到邻居 B → 任何一次 `select_layer`/`commit_transform` 触发 `resolve_gradient`（4037-4041）→ 渐变写进 B。另外 `install_project`（653-673）不清理 `gradient_edit`，跨项目加载后同样的错误提交也能发生。
- **[高] 撤销/重做缺少 Swift 的 busy 守卫，可在变换/像素搬移进行中换掉整个文档** — `crates/core/src/session.rs:593-598`（`can_undo`）、`604-614`（`undo`/`redo`）、`620-637`（`restore`）。Swift `EditorSession.swift:563-567` 的 `canUseHistory` 要求 `transformEdit == nil && textDraft == nil && levels == nil && selectionAmountOperation == nil && brushStroke == nil …`。Rust：普通持久变换（`begin_transform`，4824-4862）不开 history 事务，`can_undo()` 在 Cmd-T 打开时仍为 true；`undo()` 会直接 `restore()` 换文档且 `restore` 不取消 `transform_edit`、不取消 `pixel_move`（只取消 crop/gradient，621-622）——之后 `commit_transform` 会把陈旧 draft 以 "Transform Layer" 写进被恢复的文档（4623-4642），`end_pixel_move` 同理把旧的抬升像素写进恢复后的活动层（2194-2224）。浮动选区变换因 `begin_selection_transform` 开了事务（4703）而被 depth 挡住，普通变换与像素搬移没有。

### 中严重度（边界行为错误 / 与声明不符）

- **[中] `undo()` 没有 Swift 的"第一次撤销先丢弃待定渐变"** — `crates/core/src/session.rs:604-608` 对比 Swift `EditorSession.swift:569-571`（`if gradientEdit != nil { cancelGradient(); return }`）。Rust `can_undo`（597）因 `gradient_edit.is_some()` 声称可撤销，但 `undo()` 直接弹一条真实历史条目并经 `restore` 顺带取消渐变：一次 Cmd-Z 同时丢渐变+回退一步，Swift 需要两次按键。菜单状态与实际消耗不一致。
- **[中] 渐变"成线"阈值被双重开方削弱** — `crates/core/src/session.rs:3941`、`crates/core/src/session.rs:3996`：`((end.x-start.x).hypot(end.y-start.y)).sqrt() >= 0.5`。`hypot` 已是欧氏长度，再 `.sqrt()` 把有效阈值降到 0.25px，与 Swift `Gradient.swift:32`（`hypot(...) >= 0.5`）不符；0.25-0.5px 的"点击"会提交成渐变而不是取消。
- **[中] 多选整体变换（group transform）管线是死管道，功能实际缺失** — `crates/core/src/session.rs:4833-4862`（`begin_transform_persistent` 只变换活动层，无 Swift `EditorSession.swift:378-384` 的 `transformsAsGroup` 分支）；全工程（app+core+tests）无任何 `group: Some(...)` 构造（grep 验证，`session.rs` 内 4734/4860/6474 三处均为 `group: None`）。由此 4099-4118 `commit_distort` 的 group 分支（4100-4104，undo 名 "Distort Layers"）和 1885-1900 `displayed_mask_placement` 的 group 分支永远不可达；`TransformGroup`（transform.rs:212）只有类型。测试 6467-6475 是手工构造 `TransformEdit` 才走到该路径的。
- **[中] `select_layers` 缺 Swift 的三件事：清 effect_selection、finish_text、resolve_gradient** — `crates/core/src/session.rs:2891-2913` 对比 Swift `LayerGroups.swift:96-105`（`effectSelection = nil`；`if ids != selectedLayerIDs, !finishText() { return }`；`commitTransform(); resolveGradient()`）。Rust 仅在多选集合变化时 `commit_transform`（2905-2907），不清 `effect_selection`、不应用待定文本、不解析待定渐变——后者同样是"渐变写错层"的入口之一。单选 `select_layer`（693-703）有 commit+resolve，但同样不清 `effect_selection`（只有 `select_layer_target` 1768 清）。
- **[中] `delete_with_live_mask_choice` 未接线（生产死代码）** — `crates/core/src/session.rs:3756-3787`。grep 全工程仅 session.rs 自身测试（7928/7941）调用；`bake_live_mask`（3692）另有真实调用点 `crates/core/src/project/workspace.rs:85`。即 bake 管线活着，但"删除前询问 bake/unlink"的会话入口在 Rust 应用里不可达（与 高-1 叠加）。
- **[中] `begin_gradient` 缺 `tool == Gradient` 守卫** — `crates/core/src/session.rs:3899-3924` 只查 `can_edit_pixels`；Swift `Gradient.swift:37` 首条 guard 是 `tool == .gradient`。核心层面任何工具都能开待定渐变，正确性完全依赖 app 侧路由。
- **[中] `rename_layer` / `toggle_layer_visibility` / `move_active_layer` 缺 `can_edit_layers` 守卫** — `crates/core/src/session.rs:822-834`、`836-849`、`852-883`。Swift 对应 `EditorSession.swift:646`（rename）、`:657`（toggleVisibility）都有 `canEditLayers` guard，`moveActiveLayer` 经 `canMoveActiveLayer`（EditorSession.swift:710-715）同样要求。Rust 里裁剪框或变换打开期间这些操作照常执行并各自落 undo 步。
- **[中] `add_mask_from_selection` 的 doc comment 与实现（及 Swift）相反** — `crates/core/src/session.rs:2359-2361` 注释称"the mask shows (or hides) just the selected pixels"；实现 2389-2417 是 base（revealing→255）铺满、选区内 blend 向反色 `fill`（`fill = 255 - base`），即 revealing=true 时选区被敲黑、其余全白。这与 Swift `LayerMask.swift:233-260`（"so a white mask hides the selection"）和 Rust 测试 6817-6834（`mask_from_selection_knocks_out_the_selection`）一致——行为对、注释错，读注释会得出相反预期。

### 低严重度（死代码 / 文档漂移 / 风格 / 微小数值偏差）

- **[低] 全工程零调用的 pub 方法（grep 验证，含测试也无人调用）** — `cancel_guide_drag`（`crates/core/src/session.rs:5596-5598`）、`extend_selection`（2917-2933，Cmd-Shift-click 多选未在 app 接线）、`move_lasso_cursor`（1273-1277）、`remove_last_lasso_point`（1279-1286）。另有约 44 个 pub 方法在 session.rs 之外无调用点（如 `delete_with_live_mask_choice`、`set_selection`、`render_selected_pixels`、`floating_selection_transform` 等——它们被 session.rs 内部或内联测试使用，属"过度 pub"而非死逻辑）；`#![allow(dead_code)]`（第 7 行）把这些全部静音。
- **[低] 渐变行长判断重复且语义绕** — `crates/core/src/session.rs:3941` 与 `3996` 两处重复同一表达式而非提取（Swift 是 `hasLine` 计算属性，Gradient.swift:32），叠加 双 sqrt 问题（中-2）。
- **[低] `group_selected_layers` 构造了从不使用的 `names` 集合** — `crates/core/src/session.rs:2999-3000` 收集全部图层名，3052 用 `let _ = &mut names;` 压警告；"Folder N" 编号实际发生在 3001-3004 的另一份收集上。纯冗余。
- **[低] `commit_crop` 重复语句** — `crates/core/src/session.rs:5340-5341` 连续两行 `self.crop_rect = None;`，复制粘贴残留。
- **[低] 测试内调试残留** — `crates/core/src/session.rs:7690-7703`：`shape_layers_rerender_on_transform_commit` 里除 `SHP_DBG` 环境变量块外，7700-7703 还有一条**无条件** `eprintln!("DBG shape test: …")`，每次 `cargo test` 都会打印；`5267-5270` 的 `CROP_DBG` 块把 `updated.min_x()` 同时当作 in/out 打印（`in={:?} out={:?}` 两参数相同），调试输出本身是错的；`7498-7505` 另有 `GRAD_DBG` 残留。
- **[低] `change_text_style` 对草稿的非法修改不回滚** — `crates/core/src/session.rs:4564-4568`：`change(&mut draft.style); if !draft.style.is_valid() { return; }`，闭包已把非法值写进 draft；而 defaults 分支（4569-4575）是先改克隆、验证通过才赋值。两条路径行为不对称，非法样式会留在打开的文本草稿里。
- **[低] 蒙版/选区逐像素混合用 `as u8` 截断而非四舍五入** — `crates/core/src/session.rs:2412`（`add_mask_from_selection`）、`2708`（`render_selected_pixels` 软边乘法）、`2738`（`render_merged_pixels`）。与 Swift CoreGraphics 的舍入存在 ±1 灰度偏差，无累积危害。
- **[低] 选区驱动的逐像素路径为 O(w·h) 且每像素做一次仿射变换** — `crates/core/src/session.rs:2320-2343`（`invert_pixels`：每像素 `applying(&to_doc)` + `alpha_at`）、`2400-2417`（`add_mask_from_selection`）。在 100MP 预算的图层上约为 1 亿次矩阵乘，单步操作会卡顿；线性复杂度、无 O(n²)，不算陷阱但值得注意。
- **[低] `distort_at` 中蒙版裁剪的防御式表达式可产生 0×0 蒙版** — `crates/core/src/session.rs:4157`：`crop_w.min(warped_mask.width - crop_x.min(warped_mask.width))`，若 `crop_x >= width` 会得到 0×0 位图并替换原蒙版。按 `warp_trimmed` 的裁剪语义应不可达，但写法脆弱、无断言。
- **[低] `create_document` 不重置 `is_mask_selected`** — `crates/core/src/session.rs:545-566`。旧文档蒙版目标会带进新画布（新层无蒙版）。经与 Swift `EditorSession.swift:893-906` 对照，原版同样不重置，属继承缺口而非移植错误，故仅列低。
- **[低]（相邻文件）history 的蒙版字节按 1 字节/像素计，实际 Bitmap 是 RGBA** — `crates/core/src/history.rs:191-196` 对 mask 计 `width*height`，而 Rust `LayerMask.asset.image` 是 4 通道 `Bitmap`（对照 session.rs:1410-1414 的用法），真实驻留内存是该值的 4 倍。与 Swift 语义一致（Swift 蒙版真是 DeviceGray），但 Rust 实现里 256MB 上限对蒙版多的文档会放行约 4 倍内存。

---

## 二、与旧报告 A 的矛盾点（docs/agent-reports/A-document-session-history.md，2026-09-24）

旧报告 A 基于 1951 行的旧版 session.rs。现行 8580 行版本已使其绝大多数"缺失"论断失效。逐条核对结果：

**已失效（代码现已在场，引用旧报告原文位置 → 现行代码行号）：**

1. A§一.1「多选 `selectLayers`/`extendSelection` 缺失，`selected_layer_ids` 从未赋多元素」→ `session.rs:2891-2933`（另有 `selected_layer_ids` 多元素赋值遍布 930、3047、3484 等）。
2. A§一.2「`deleteSelectedLayers` 无对应」→ `session.rs:3264-3318`（undo 名 "Delete Layers"，测试 7170-7182）。
3. A§一.3「`reorderLayers` 缺失」→ `session.rs:3198-3234`。
4. A§一.4「眼睛滑动三件套缺失」→ `session.rs:3238-3260`。
5. A§一.5「导入管线完全无入口」→ `session.rs:3333-3488`（`insert_image`/`import_images`/`import_photoshop_data`）。
6. A§一.8「`commitTransform` 无 floating/corners/mask 分支」→ `session.rs:4590-4612`（三分支俱全）。
7. A§一.13「`textDraft`/`finishText` 缺失」→ `session.rs:4247-4576` 全套。
8. A§一.14-16（LayerGroups：entries/groupSelectedLayers/addGroup/toggleGroupExpansion/canPlaceLayer/placeLayer/moveActiveLayerOutOfGroup）→ `session.rs:2938-3194, 3321-3326`。
9. A§一.17「mergePlan 无菜单可用性查询」→ `session.rs:5018-5114`（`merge_plan`/`can_merge_layers`/`merge_title`）。
10. A§一.18-25（LayerMask 全组缺失）→ `add_mask_from_selection` 2362、`select_layer_target` 1767、`can_copy_mask`/`copy_mask` 1778/1802、`toggle_mask_link` 1845、`displayed_mask_placement` 1880、`commit_mask_transform` 4647、`commit_mask_distort` 4206。
11. A§一.26-28（LiveLayerMask 全组缺失）→ 3534-3687（link/release/toggle）、`adopt_clipping` 5722、`release_detached_clipping` 5745、`bake_live_mask` 3692、`delete_with_live_mask_choice` 3756。
12. A§一.29-31（LayerAppearance）→ `set_preview_blend_mode` 1033、`can_edit_appearance` 含 canEditLayers 1013、`set_selected_layers_opacity` 1127。
13. A§一.32-33（调整层全套）→ `add_adjustment` 2439、`update_adjustment` 2489、AdjustmentEditing 四件套 2507-2608。
14. A§一.35「无 installProject/clearProject」→ `session.rs:653-689`。
15. A§一.36（ColorPalette 会话态）→ `ColorPickerTarget/State` 210/300，方法在 color_picker.rs；`ColorPickerTarget` 甚至带 GradientMap 变体。
16. A§一.37「FloatingSelection 整文件无对应、无 session 代码构造或消费」→ `begin_selection_transform` 4689、`floating_selection_transform` 4740、merge/cancel 4754-4810、`floating_merge` 5654。
17. A§一.38-39（Guides 拖拽/吸附全套缺失）→ `GuideDrag` 140、`hit_guide` 5436、guide 拖拽三件套 5510-5593、`alignment_snap_targets` 3816、`snapped_guide_position` 5457。
18. A§一.40-43（Brush 会话：makeRasterEdit/commitRasterEdit/透明度键/硬度）→ `raster_edit_for_active` 1931、`commit_pixel_edit` 1971、`uses_opacity_keys`/`type_opacity_digit` 1162/1172、`change_brush_hardness` 1061。
19. A§一.44（SelectionClipboard）→ `duplicate_active_layer`（含 descendants 重映射与 10000 上限）887-941、`add_pixel_layer` 2822、`next_layer_name` 2849、copy/cut/paste/layer-via-copy 2748-2818。
20. A§二.3「删除活动层后选中置 None」→ 现按 `layers[min(index, count-1)]` 选邻居（`session.rs:802-811`，测试 5876-5899）。
21. A§二.4「select_layer 不提交变换」→ 现 commit+resolve（`session.rs:696-698`）。
22. A§二.5「restore 不取消裁剪、is_mask_selected 变陈旧」→ `session.rs:620-637` 与 Swift 逐条对应（keepMaskTarget 在 625-633）。
23. A§二.6「can_undo 无渐变分支」→ `session.rs:597` 已有（但见本文 高-1：undo() 行为仍与 Swift 不同）。
24. A§二.7「蒙版单独变换提交走错路径」→ `commit_mask_transform` 4647-4673（undo 名 "Transform Layer Mask"，测试 6509-6537）。
25. A§二.8「merge 守卫不全、无 validate 回退」→ `merge_plan` 三守卫 5018-5102 + `hierarchy_is_valid` 回退 5189-5191。
26. A§二.9「duplicate 只复制单层丢子层」→ 887-941 完整 descendants 复制与 id 重映射。
27. A§二.11-12「can_edit_appearance/can_edit_mask/toggle/delete 蒙版缺守卫」→ 1013-1017、1698-1700、1723、1744 全部补上。
28. A§二.13「app 的 add_group 语义不同」→ core `add_group`（3057-3097，"Folder N"/空组/兄弟父级/"New Folder"）与 Swift 一致；app 现调用 core。
29. A§二.18「beginTransform 无 persistent 参数」→ `begin_transform_persistent` 4833 + `nudge_layer` 4883 用 transient。
30. A§三.5「add_blank_layer 插入点简化、collapsed_group_ids 是死字段」→ 文件夹顶部插入 728-753；`collapsed_group_ids` 被 `layer_rows`/`toggle_group_expansion`/group/place 多处读写（3108-3119、3048-3050、3175-3177）。
31. A§三.6「pixelClipboard/colorPicker/effectsEditing/filterEdit 等 core 无对应」→ 均已是 session 字段（386、351、346-348、429）。
32. A§三.7「selectedLayerIDs 降级 Vec、无有效性过滤」→ `select_layers` 2895-2902 做交集+去重。
33. A§二.2「快照相等语义与 Swift 相反（值相等 vs 身份）」→ `crates/core/src/raster.rs:226-231` `ImportedImage::eq` 用 `Arc::ptr_eq` 身份比较，history.rs:123 的未变判定与 Swift 一致（session.rs:6346-6348 注释与测试 6959-6973 也证实）。
34. A§一.15/§三.4「全仓库无 effective_opacity」→ `crates/core/src/render.rs:352` 已实现并在合成中使用（render.rs:261）。
35. A§四「history.rs:381 `transform_not_saved_blocks_nothing` 是无断言空测试」→ 该测试已不存在（现行 history.rs 测试区 240-470 无此名）。
36. A§五「crates/core/tests 为空」→ 现有 9 个集成测试文件（adjustment_layer/brush_session/clipboard_group/downsample/export/glow/selection_edit/tool_flow/transform_session/type_tool），大量驱动 session 公共 API。

**仍然成立（现行代码证实旧报告未过时的部分）：**

1. A§一.6「群组变换缺失」→ 仍缺：`begin_transform_persistent`（4833-4862）无 group 分支，`TransformEdit.group` 全工程无人构造为 Some（见本文 中-3）。
2. A§一.7「`beginDuplicateTransform` 缺失」→ session.rs 仍无对应（grep 无 begin_duplicate/transform_duplicate）。
3. A§一.10「`parkedBrushTips`/`tipFamily` 缺失」→ session.rs 无对应。
4. A§二.17「`WandSettings` 用 `sample_radius: usize` 而非 Swift 的 `WandSampleSize` 枚举」→ `session.rs:61` 仍是 `pub sample_radius: usize`。
5. A§三.1/三.2「LayerMask 为 RGBA Bitmap 降级、`mask_clip_image` 无缓存」→ document.rs 蒙版仍为 RGBA Bitmap；render.rs:146 `mask_clip_image` 每次重算（仅加了个 `mask_clip_image_limited` 尺寸上限包装，157）。
6. A§一.12「ToolDefaults 持久化缺失」→ core 仍只硬编码 fallback（Default impl 457-534）；持久化移到了 app 层（`crates/app/src/workspace.rs:288` `settings::persist_session`），core 内确无。
7. A§一.9 的后半「busy 状态机（isProjectBusy 等）不在此层」→ 属实且按设计留给 app；但本文 高-1 指出：Swift 靠 `canUseHistory` 挡住的部分场景（transformEdit==nil 等）在 Rust core 无等效闸门。

（数量：矛盾/失效 36 条；仍成立 7 条。）

---

## 三、总结论

1. **实现完成度高，不是 stub 集合。** 8580 行中未发现任何"返回固定值/空函数体"式占位实现；约 60 个功能子系统（从浮动选区到调整层编辑会话到 PSD 导入）都有真实逻辑，且内联测试 + `crates/core/tests` 的 9 个集成文件对绝大多数路径有断言。注释宣称的行为与实现抽查一致率高（抽核 15 处注释 vs Swift，仅 1 处注释错误：`add_mask_from_selection` 的 doc comment，见 中-8）。
2. **真正的风险集中在"状态机缝隙"而非"假实现"**：(a) 渐变提交目标跟随活动层而非草稿记录的图层，配合 `delete_layer`/`install_project` 不解析待定渐变，可把像素写进错误图层（高-3）；(b) undo/redo 缺 Swift `canUseHistory` 的守卫，且 `restore` 不撤 `pixel_move`/普通 `transform_edit`，可在拖拽中换底文档（高-4）；(c) undo() 未实现"先弃渐变"（中-1）；(d) 双 sqrt 阈值（中-2）。
3. **两条删除路径相对 Swift 是功能性回退**（不走 bake 询问 + `delete_selected_layers_with_baked` 多选路径删错对象且 bake 在 undo 步外，高-4/高-5），当前因 `delete_with_live_mask_choice` 未接线而潜伏，一旦接线即触发。
4. **多选整体变换（group transform）是"管道铺好、源头缺失"**：commit/preview/display 侧 group 代码齐全但无任何构造入口（中-3），旧报告 A 对此的判断在 2026-09-28 仍然成立，这也是旧报告少数未失效的"缺失"论断之一（另有 beginDuplicateTransform、parkedBrushTips、WandSettings 枚举）。
5. **死代码规模可控但被 `#![allow(dead_code)]`（第 7 行）掩盖**：4 个全工程零调用的 pub 方法 + 约 44 个仅在 session.rs 内部/测试使用的过度 pub 方法，其中 `delete_with_live_mask_choice` 属"实现了关键安全询问却没接线"的那类，建议优先接线或删除。
