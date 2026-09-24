# compositor-rs 与 Swift 原版 Compositor 差异报表

- 对比基线：`Compositor/`（Swift/C 原版，约 39,300 行，193 个源文件）vs `crates/core` + `crates/app`（Rust 重写，约 30,700 行，52 个 `.rs` 文件）。
- 对比方法：按 8 个功能域逐文件通读两侧代码，逐公式/常量/守卫核对；关键结论均经 grep/Read 复核行号。
- 日期：2026-09-24。所有路径为仓库根相对路径。

## 修复进度（2026-09-24 第二轮，按 §11 优先级推进）

以下报表缺陷已在本轮修复并有测试覆盖（`cargo test -p compositor-core` 338 全绿）：

**§11-2 确认的公式 bug（全部完成）**
- §6.1-3 HardMix 改为 PDF 公式 `Cb+Cs < 1 → 0`（旧 VividLight 阈值在 cs>0.5 时错位）。
- §6.1-4 `composite_pixel` 改为 PDF 方程 `αs(1-αb)·Cs + αsαb·B + αb(1-αs)·Cb`，半透明 backdrop 不再丢失源色。
- §6.1-5 Divide 零源改回 PDF 的 `Cs=0 → 1`（白）。
- §5.1-1 heal 行扫描 clamp 由 `wh` 改回 `ww`（HealPixels.c:143），RING 不再偏薄。
- §5.1-2 `coverage_bounds` 只在完全为空时返回 None，单行/单列/单像素 spot 恢复可修。
- §5.1-3 渐变「前景→透明」端点改为 `[fg α=1, fg α=0]`（app 传参）。
- §8.1-1 coverage 边界 inset 改为 `ceil(feather*2)`。

**§11-1 渲染语义三件套（全部完成，render.rs 重写）**
- §6.1-1 调整图层改作用于**下方已合成内容**（LiveMaskRenderer.adjust 语义：非 Normal 混合模式全覆盖混合、有效不透明度混合、自身蒙版裁剪作用域、is_valid 守卫）；自身像素不再绘制。
- §6.1-2 「蒙版+效果」只烘焙一次蒙版（效果以 `mask: nil` 绘制），并修复 effects placed_transform 以 grown 网格为基准的错位。
- §6.1-6 剪贴蒙版 coverage 改为对源层完整 drawOwn 渲染（含效果/有效不透明度/自身蒙版/递归剪贴、与可见性无关），带缓存与防环。
- §6.1-7 剪贴组成组语义：连续同源同父层共享 base 覆盖率（prepareStacks/drawComposite），整组以 base 混合模式上画；组内 adjustment 子层改写组表面。
- §6.1-8 文件夹有效不透明度 `effective_opacity` 逐层乘祖先（LayerOpacity）。
- §3.1-10 渲染顺序改为深度优先层级树（renderLayers），悬空父级不可见。
- 新发现并修复：`mask_clip_image` 的 `mask.placement?` 使**无独立 placement 的蒙版整体失效**（§6.1-2 的根因之一，旧 folder 蒙版测试因图层未超出蒙版范围而空通过，已同步加强）。

**§11-6 小修（全部完成）**
- §7.1-3 CameraRaw/Curves/Exposure/GradientMap/BlackWhite/ColorBalance 的 `apply` 均先 `is_valid` 守卫（Grain 原有）。
- §4.2-7 `LayerSampling` serde/derive 默认统一为 High（LayerTransform.swift:24）。
- §7.1-9 colorize 饱和度 `(saturation/100).clamp(0,1)`，0 不再被抬到 1%。
- §7.1-10 `visualize_point_color` 默认 -1。
- §3.1-15 笔画 undo 名按工具区分（Brush Stroke/Clone Stamp/Blur/Erase/Paint Mask/Spot Healing）。
- §5.2-9 heal 种子每次随机（时间源）。
- §3.1-3 删除活动层后选中邻层 `layers[min(index, count-1)]`（LayerTests.swift:27）。
- §3.1-4 `select_layer` 切层先 `commit_transform()`。
- §3.1-7 蒙版单独变换提交走 `commit_mask_transform`：只改 placement，undo 名 "Transform Layer Mask"。
- §3.1-11/12 新增 `can_edit_layers()`；`can_edit_appearance`/`can_edit_mask` 加合取；`toggle_layer_mask`/`delete_layer_mask` 加 `can_edit_mask` 守卫。
- §3.1-16 `NavigationTool::Idle` label 落到 "Zoom (Z)"（与 Swift 直落一致）。
- §8.1-8 wand 空白图层/文件夹读作透明画布 → 全画布命中（selectionSample）。
- §8.1-9 wand 轮廓追踪失败写入 `brush_error`（不再静默）。

**§11-3 app 接线（部分完成）**
- §9.1-1 PSD 导入已接入 `open_document`：`PsdReader::read_with_budget` → assets → `make_import_with_assets`，无文档时作新文档、有文档时包组导入（insertPhotoshop 语义），万层上限、转换说明计入状态栏。
- §9.1-2 JPEG 导出落地：core 新模块 `export.rs`（`check_export_size` 100MP/30000 守卫、`flatten_on_matte` 哑光铺底、`png_data`/`jpeg_data`（质量 0…1，JpegEncoder）、`preview_thumbnail` ≤1000px）+ app `Export JPEG…` 菜单（alt-shift-s）与 ExportJpeg sheet（路径、质量滑杆默认 85%、哑光色 White/Black）。DPI 元数据（pHYs/JFIF density）image crate 不支持写入，仍缺。

**§11-4 会话操作补齐（部分完成）**
- §7.2 Camera Raw 吸管流程落地：`adjust/camera_raw.rs` 新增 `DEFRINGE_PURPLE_CENTER=290.0`/`DEFRINGE_GREEN_CENTER=90.0`/`DEFRINGE_SPAN=25.0` 常量、`hue_degrees`（RGB→色相 0…360）、`straight_pixel_hue`（透明像素返回 None）、`apply_defringe_sample`（较近的紫/绿中心 ±25° 归一化，amount 为 0 时起步 50）、`apply_auto_white_balance`（选 Auto + `auto_balance` 求温度/色调）；app `state.rs` 加 `camera_raw_samples_defringe`/`camera_raw_readout` 状态与 `sample_camera_raw_defringe`/`update_camera_raw_readout`/`apply_camera_raw_auto_white_balance` 方法，canvas 点击/移动在 Camera Raw sheet 打开时路由到吸管。
- §4.1-3 变换吸附接入：session `alignment_snap_targets`/`transform_snap_targets`（画布边 + 可见图层边与中心，`snapping_enabled` 门控）、`snapped_move`（TransformSnap::offset 求偏移，落点写入 `snap_guides` 供画布画参考线）；app 的 `MoveLayer` 拖拽已路由 `snapped_move`（TransformSnap.distance=10 文档像素）——TransformSnap 自此有了消费点。
- §5.2-3 GPU 连续沉积模型落地（MetalBrushCoverage.swift → `tools/brush.rs`）：`render_continuous_tile`/`ContinuousTile`/`ContinuousParams`——coverage = 1−exp(−∫tipDensity)，8 点 Gauss-Legendre 沿线积分（裁剪到 tip support），permanent 与 tail 分离（tail 预览永不进入 permanent，含自交）、硬笔刷保留像素边抗锯齿、optical density 封顶 20；`BrushStroke` 记录指针事件段（append，单击为零长段），`result()`/`heal` 的 coverage 改由连续引擎产出（heal 后清空事件段以熄灭湿预览）；spacing 无关性有测试（长短事件同点沉积相等）。
- §4.1-2 CropSnap 裁剪边吸附落地（`core/canvas.rs`）：`CropSnap::apply`——move 吸最近双边保尺寸、create/resize 只吸被拖边（ratio 固定时仅 move 吸）、symmetric 镜像半宽；session `crop_snap_targets`（画布边 + 可见图层边界，无中心）与 `drag_crop` 集成（8 文档像素容差，snapping_enabled 门控）。
- §5.3 TypeTool 真字体排版落地：新模块 `tools/text.rs`（fontdue 光栅化 + 系统字体查找与替身表，Windows/macOS/Linux 字体目录）；`text_box_size`（固定框直用、点文本按行测量 + padding + 光标宽度）、`text_image`（词换行、左/中/右对齐、tracking、leading/Auto 120%、样式颜色 premultiplied 写入）、`layer_name`（首词一行、40 字上限）；session 流程：`begin_text`/`begin_text_in`（点文本 vs 框文本）、`edit_active_text`、`apply_text`（空白草稿静默、编辑现有层保持变换锚点与缩放/旋转/翻转、新层 "New Text Layer"、textDefaults 更新、asset 与 live text 共享同一图像）、`finish_text`/`cancel_text`、`recolor_text`（"Fill Text" 保持可编辑）、`change_text_style`/`current_text_style`；app 的 5×7 点阵字体（text_bitmap.rs）已删除，create_text_layer 改走 core 流程。
- §2 活蒙版整文件落地（LiveLayerMask.swift）：`can_link_mask`/`link_mask`（LiveMaskGraph.validate 链校验：环、组/调整层不可作源）、`remove_live_mask`（释放 base 连带上连续同源子层，undo "Release Clipping Mask"）、`can_toggle_clipping_mask`/`toggle_clipping_mask`（Option-click 剪贴到较低兄弟/共享其 base；已剪贴则释放）；`adopt_clipping`/`release_detached_clipping` 自由函数（placeLayer 已接入）；`bake_live_mask`（LiveMaskBaker.bake：源链烘进目标像素，蒙版与外观保持 live）；`delete_with_live_mask_choice(ids, bake)` + `delete_layer`/`delete_selected_layers` 的链接解除与 baked 像素写入（finishDeletingLayer 语义；bake/unlink 对话框属 app UI 仍缺）。
- §2 Distort 整文件落地（Distort.swift → `tools/distort.rs`）：`corners_of`/`is_usable`/`is_convex`、`Homography`（单位方角→四角透视 + 解析逆）、`affine_between`、`warp`（凸形透视双线性重采样 + 折叠形两三角仿射、1×1 蒙版直通、尺寸守卫、limit 预览）、`warp_trimmed`（alpha_bounds 裁剪）、`carried`（放置蒙版随透视）、`warp_mask`（形状外铺 background）、`map_path`（选区轮廓跟随）；session 流程：`begin_distort`/`preview_corners`（无效形状拒绝）、`commit_transform` 的 corners 分支 → `commit_distort`（"Distort"，像素+链接蒙版重采样进形状、放置蒙版随行、未链接蒙版原地）与 `commit_mask_distort`（蒙版单独扭曲，"Distort Layer Mask"）。
- §3.2 多选批量落地（LayerGroups.swift）：`select_layers`（有效性交集 + 切换先提交变换）、`extend_selection`（Cmd-Shift-click）、`group_selected_layers`（根集合/公共父级/包裹层落在最顶分支处/选中文件夹带子树/选中后代不拉出，"Folder N"，undo "Group Layers"）、`add_group`（空 "Folder N" 插活动层上方、挂活动层父级/活动组，undo "New Folder"）、`toggle_group_expansion`（折叠带走活动选中）、`can_place_layer`/`place_layer`（防环、目标同父校验、adoptClipping/releaseDetachedClipping 剪贴链接维护、undo "Move Layer"）、`move_active_layer_out_of_group`、`reorder_layers`（面板 top-first 序移动，Array.move 语义）、眼睛滑动 `begin/set/end_visibility_swipe`（一次 undo）、`delete_selected_layers`（一条 "Delete Layers"）、`layer_rows`（LayerHierarchy.entries 移植：top-first + depth + 折叠，`collapsed_group_ids` 由死字段转为 HashSet 真正生效）。
- §3.2 导入管线落地：`import_images`（PNG/JPEG 解码、30000/边 + 100MP 文档像素预算守卫、无文档时首图定画布、有文档居中放置、失败清单、一条 "Import Images" undo）、`insert_image`（`insert(_:centeredAt:)` 语义）、`import_photoshop_data`（PSD 路由，open_document 已改为复用此函数）；app 增加 `Import Images…` 菜单与 Import sheet（分号分隔多路径）。
- §2 浮动变换整文件落地（FloatingSelection.swift）：`can_transform_selection`/`begin_selection_transform`（选区像素 lift 到临时 "Floating Selection" 层、源层清空、父级/不透明度/混合继承、一条 "Transform Selection" 事务）、`floating_selection_transform`（选区跟随映射）、`merge_floating_transform` + `floating_merge`（FloatingMerge.merge：合成回源层网格、图层按 extent 长大、无 placement 蒙版同步生长 reveal）、`cancel_floating_transform`（Escape 精确恢复）；`commit_transform`/`cancel_transform` 接入 floating 分支（未改动 commit 精确还原）。
- §3.2 选区编辑：`fill_selection`/`clear_selected_pixels`（蒙版目标走 Fill Mask/背景色）、`can_invert`/`invert_pixels`（含 1×1 均匀蒙版扩展）、`add_mask_from_selection`（revealing 语义对齐 addMask）、`add_adjustment`/`update_adjustment`（Gradient Map 前景→背景、Grain/AddNoise 随机种子）、`can_edit_pixels`、`raster_edit_for_active`/`commit_pixel_edit` 辅助。
- §2/§3.1-9 剪贴板整文件落地：`selection_copy_region`/`can_copy_pixels`/`render_selected_pixels`（蒙版作不透明灰、软选区边缘保留）/`render_merged_pixels`/`copy_selection`/`copy_merged_selection`/`cut_selection`/`paste_pixels`/`layer_via_copy`/`add_pixel_layer`/`next_layer_name` + `PixelClipboard`；`duplicate_active_layer` 重写为连后代复制 + parent/maskSource 重映射 + 万层上限 + " copy" 命名。系统粘贴板（PNG 互通）属 app 层仍缺。浮动变换（Cmd-T）、多选批量、导入管线、Distort 仍缺。

**测试**：核心 321 → 397（+76：混合公式、heal、渲染语义 6 项、蒙版/删除/选区编辑/调整图层/剪贴板/复制文件夹/浮动变换 4 项/导出 5 项/多选批量与折叠 5 项/重排/眼睛滑动/批量删除/导入 3 项/DistortWarp 6 项/活蒙版剪贴与释放 3 项/烘焙 1 项/Distort 会话 2 项/真字体排版 6 项/文本会话 5 项/连续沉积 3 项/裁剪吸附 4 项/吸管色相与 defringe 4 项/变换吸附 2 项）。

## 总体结论

| 功能域 | 对等度 | 一句话结论 |
| --- | --- | --- |
| 变换/几何 (`transform.rs`/`geometry.rs`) | **≈1:1** | 公式逐式一致，`concat` 已修为 CoreGraphics 语义 |
| 像素内核（adjust/levels/curves/filters/heal/content_fill/wand 的算法本体） | **≈1:1（有例外）** | 数值常数逐项一致；heal 有 1 处确认 bug，Blend 有 2 处公式偏差 |
| 项目格式 (`project/`)、PSD 读写 (`psd/`) | **高** | manifest v1–v9 校验逐条对应；PSD 解析逐行对齐，但 **app 未接线 PSD 导入** |
| 文档模型/历史 (`document.rs`/`history.rs`) | **高** | 模型与历史内核忠实；若干相等语义/字节估算不同 |
| 会话操作 (`session.rs`) | **中低** | Swift EditorSession 六个扩展约 2900 行，Rust 只覆盖一小部分操作 |
| 选区高层操作、图层效果 UI、剪贴板、浮动变换、Distort、TypeTool 排版、SubjectRemoval | **大量缺失** | Swift 整文件无对应 |
| 渲染合成 (`render.rs`) | **语义有偏差** | 调整图层作用域、剪贴蒙版语义、半透明 backdrop 合成、双重蒙版等与 Swift 不同 |
| UI (`crates/app`) | **骨架级** | 快捷键/菜单/sheet/面板大面积缺失，app crate 零测试 |
| 测试 | **约 6 成移植** | Swift XCTest 约 300 用例中相当一部分（尤其会话级/UI 级）无 Rust 对应 |

图例：**【缺失】**= Swift 有 Rust 无；**【行为差异】**= 两边都有但结果不同；**【简化】**= 移植但降级；**【多出】**= Rust 独有。

---

## 1. 确认为等价/高质量移植的部分（抽查核对过）

这些部分逐公式核对一致，可以放心：

- `transform.rs` / `geometry.rs`：`point/contains/placing/following/rounded`、`TransformDrag`（镜像翻转、比例锁定投影）、`TransformSnap`、`concat` 的 CoreGraphics 语义（`fix3.py` 修过，现已正确）。
- C 内核常数：`AdjustPixels.c` 全部 12 个内核与 `camera_raw.rs`/`image_adjustments.rs` 逐常数一致（sRGB↔linear、Rec709、exposure 2^stops、vibrance 肤色保护、dehaze、vignette 椭圆、glow/halation、混色器中心、detail 半径、锐化 mask、色差/暗角、校准版本系数）；`LevelsPixels.c` ↔ `levels.rs`；grain hash（`AdjustPixels.c:31-73` ↔ `image_adjustments.rs:610-707`）；Curves Hermite 样条（`Curves.swift:18-37` ↔ `curves.rs:61-103`）。
- `heal.rs` 主体（膜求解迭代 300/40、omega=1.8、patch 搜索 `{1.05…2.8}`×24 角度±3 精调、Box-Muller 纹理）与 `content_fill.rs`（LCG、28 候选、64→1 折半细化）、`warp.rs`（Smudge/Liquify 权重、间距、margin、`finish_warp_into_layer`）——但 heal 有 1 处 bug 见 §5.1。
- `wand.rs` flood-fill/轮廓追踪与 `WandPixels.c` 逐行等价（"hash" 一词两边都不存在，均为逐通道 ±tolerance）。
- 混合模式 24 种枚举/serde 名与 PDF 公式主体、效果展开顺序（shadow→outer glow→外描边→像素→color overlay→inner glow→inner shadow→内描边）、`extreme()` 滑窗形态学、Add Noise hash、Lens Correction 畸变模型——例外见 §6。
- `project/store.rs`：manifest v1–**9**（README 写 v1–8 已过时）全部校验规则逐条对应（16 项规则对照核实无误），含层级校验、live-mask 图校验、预算上限。
- `psd/`：reader 的段解析/压缩/blend 签名 24 项映射/kind 判定/adjustment 段（levl/curv/hue2）逐行对齐；fixture（写侧）RLE `packBits` 字节级一致。
- 历史内核：条目上限 100、事务嵌套折叠、undo 名列表基本一致。

---

## 2. 整文件级缺失（Swift 文件在 Rust 无任何功能对应）

| Swift 文件 | 行数规模 | 内容 | 备注 |
| --- | --- | --- | --- |
| `Compositor/Compositor/Document/FloatingSelection.swift` | ~160 | Cmd-T 浮动选区变换（begin/merge/cancel、FloatingMerge.merge） | Rust 仅存数据壳 `selection.rs:605-612` `FloatingTransform` + `transform.rs:228` 字段，`session.rs:1180` 恒置 `None`，无任何流程构造 |
| `Compositor/Compositor/Document/SelectionClipboard.swift` | ~220 | 剪切/复制/粘贴/Copy Merged/Layer via Copy、`duplicateLayer` 连后代复制 | Rust 仅有 `session.rs:491` `duplicate_active_layer`（且为浅拷贝，见 §3.9） |
| `Compositor/Compositor/Document/SelectionEdits.swift` | ~200 | 填充/清除选区像素、反相、像素移动（Cmd 拖动 PixelMove）、`expandedUniformMask` | Rust 无 `fill_selection`/`clear_selected_pixels`/`invert_pixels` 会话入口；`session.rs:163` `selection_move_origin` 为死字段 |
| `Compositor/Compositor/Document/MaskTracing.swift` | ~90 | Cmd 点击蒙版/图层载入选区（darkPixels/whitePixels/opaquePixels/trace） | 无 |
| `Compositor/Compositor/Document/ObjectSelection.swift` | ~300 | Vision 物体选择（边缘保留二值化/腐蚀膨胀/Chaikin 平滑/简化） | `tools/mod.rs:3` 注释声称移植但只有注释；app 的 Object 模式（`crates/app/src/canvas.rs:792-804`）点击仍走 `magic_wand`（`canvas.rs:239`），是无效开关 |
| `Compositor/Compositor/Document/SubjectRemoval.swift` | ~143 | Vision 前景分割 + GuidedMatte 抠边 + 移边 | `tools/subject_removal.rs:9-11` 是返回 `image.clone()` 的 TODO stub；`guided_refine`（`adjust/pixel.rs:233`）无生产调用方 |
| `Compositor/Compositor/Document/AdjustmentEditing.swift` | ~120 | 调整图层编辑会话（begin/preview/finish/cancel，"Edit X Adjustment"） | 无；Rust 无法创建调整图层（见 §3.32） |
| `Compositor/Compositor/Document/ProjectWorkspace.swift` + `EditorSession+Projects.swift` | ~350 | 多标签多项目工作区、跨项目复制图层、projectSnapshot/installProject | `crates/app` 是单文档布局；core 无 session 级安装/重置入口 |
| `Compositor/Compositor/Document/ToolDefaults.swift` | ~30 | UserDefaults 持久化工具开关 | `session.rs:252-260` 只硬编码 fallback 值 |
| `Compositor/Compositor/Document/LevelsAutomatic.swift` | ~90 | 自动色阶三模式 + 吸管校准 | core 无；app 层 `crates/app/src/sheets.rs:382-422` 的 `levels_auto` 是**算法不同的重写**（见 §7.1） |
| `Compositor/Compositor/Document/Distort.swift` | ~392 | DistortWarp/homography/warp 全套 + session distort 流程 | ✅ 已移植（`tools/distort.rs` + session distort 流程，见顶部修复进度；效果预览扭曲缓存属渲染端未移植） |
| `Compositor/Compositor/Rendering/TiledLayerRenderer.swift` + `RasterSnapshot.swift` + `DownsampleCache.swift` + `LiveMaskRenderer.swift` + `AdjustmentSurface.swift` + `LayerEffectsSurface.swift` + `EffectsPreviewCache.swift` | ~1100 | 瓦片渲染/稀疏快照/降采样缓存/剪贴蒙版运行时/效果预览缓存 | `render.rs:1-4` 注释声称"复刻语义"，实际是整图单次仿射绘制；无任何 tile/patch/cache 结构。语义后果见 §6 |
| `Compositor/Compositor/Rendering/InlineTextEditor.swift` | ~700 | 行内文字编辑器 | 无 |
| `Compositor/Compositor/IO/RawImporter.swift` | ~123 | RAW 导入（CIRAWFilter 队列/asShot/develop） | 无（`camera_raw.rs` 是作用于位图的滤镜，与 RAW 解码无关） |
| `Compositor/Compositor/UI/` 下约 25 个面板/控件文件 | ~7000 | CurvesControls、EffectsSheet、ColorPickerSheet、PSDConversionSheet、RawDevelopSheet、FloatingPanel 系统、TransformInspector、CanvasRulers 交互、ProjectTabs、KeyboardShortcuts 自定义系统等 | 详见 §9 |
| Swift 侧 WarpStroke 的 GPU/交互壳、`MetalBrushCoverage.swift` | ~160 | GPU 连续沉积模型（coverage = 1−exp(−∫tipDensity)，8 点 Gauss-Legendre） | Rust `tools/brush.rs` 复刻的是 CPU 回退路径（逐 dab 盖章），无 continuous 路径 |（✅ CPU 连续路径已移植进 `tools/brush.rs` 并成为 `result()` 的默认覆盖来源，见顶部修复进度）

---

## 3. 文档模型 / 会话 / 历史（A 域）

### 3.1 行为差异

| # | 差异 | Swift 位置 | Rust 位置 |
| --- | --- | --- | --- |
| 1 | 历史字节估算：Swift 按位图实际 bytesPerRow（灰度蒙版 1 B/px）；Rust 一律 `w*h*4`，蒙版多的文档更早触发 256MB 上限裁剪 | `Document/DocumentHistory.swift:103` | `core/src/history.rs:186` |
| 2 | 快照相等语义：Swift 对图像用 `===` 身份比较；Rust derive 值相等（含像素数据），"内容相同的不同图像对象"场景下"文档未变不记历史"判定相反 | `Document/EditorSession.swift:4-7` | `core/src/document.rs:501`、`raster.rs:207`、`history.rs:123` |
| 3 | 删除活动层后选中层：Swift 移到邻层 `layers[min(index,count-1)]`；Rust 置 `None`（LayerTests.swift:27 专门测此行为） | `Document/LiveLayerMask.swift:142-145` | `core/src/session.rs:414-417` |
| 4 | `select_layer` 不提交进行中的变换、无守卫；Swift `selectLayer` 先 `commitTransform()`+`resolveGradient()` 且被笔刷/levels 守卫 | `EditorSession.swift:325-331` | `core/src/session.rs:362-368` |
| 5 | undo/redo 恢复：Swift 恢复 `isMaskSelected`（按 keepMaskTarget）、取消裁剪、适配视口；Rust 只换 document/active_layer_id，`is_mask_selected` 变陈旧 | `EditorSession.swift:570-591` | `core/src/session.rs:332-350` |
| 6 | `canUndo`：Swift 含 `gradientEdit != nil` 分支；Rust 只有 history 条件 | `EditorSession.swift:567` | `core/src/session.rs:324` |
| 7 | 蒙版单独变换提交走错路径：Swift 对 `edit.mask` 走 `commitMaskTransform`（只改 mask.placement，undo 名 "Transform Layer Mask"）；Rust 把 draft 写进 `layer.transform`，undo 名恒 "Transform Layer" | `EditorSession.swift:432`、`LayerMask.swift:341-365` | `core/src/session.rs:1127-1150` |
| 8 | merge 守卫不全：Swift `mergePlan` 要求 canEditLayers、多选含非组层、组内含像素层 + `LayerHierarchy.validate` 失败回退；Rust `merge_layers` 三守卫皆无 | `Document/LayerMerge.swift:8-30,65` | `core/src/session.rs:1288-1340` |
| 9 | 复制图层：Swift `duplicateActiveLayer` 连 descendants 一起复制、重映射 parent/maskSource、上限 10000、同步折叠状态；Rust 浅拷贝单层（文件夹复制丢全部子层） | `Document/SelectionClipboard.swift:147-170` | `core/src/session.rs:491-513` |
| 10 | 渲染顺序：Swift 按 `renderLayers` 层级树深度优先，悬空 parentID 的层不可见；Rust `composite_onto` 按数组顺序，悬空父级视为可见 | `LayerGroups.swift:11-26,89-92` | `core/src/render.rs:86-97`、`document.rs:648-675` |
| 11 | `can_edit_appearance`/`can_edit_mask` 丢 `canEditLayers` 合取项 | `LayerAppearance.swift:83`、`LayerMask.swift:221` | `core/src/session.rs:579-582,1068-1070` |
| 12 | `toggle_layer_mask`/`delete_layer_mask` 无 `canEditMask` 守卫 | `LayerMask.swift:272,280` | `core/src/session.rs:1092,1110` |
| 13 | app 层建组语义不同：Swift 命名 "Folder N"、建**空**组插活动层上方、组挂到活动层父级、undo 名 "New Folder"；Rust app 命名 "Group N"、**把活动层移进新组**、组挂根、undo 名 "New Group" | `LayerGroups.swift:177-194` | `app/src/layers_panel.rs:351-384` |
| 14 | `LayerAdjustment.is_valid` 缺 HSV 细分设置校验 | `LayerAdjustment.swift:128-141` | `core/src/adjustment.rs:280-303` |
| 15 | 笔画 undo 名：Swift 按工具区分（"Brush Stroke"/"Clone Stamp"/"Blur"/"Paint Mask"…）；Rust app 只有 "Spot Healing"/"Erase"/"Paint" | `EditorSession+Brush.swift:141` | `app/src/canvas.rs:519-527` |
| 16 | `NavigationTool`：Swift `allCases` 含 `idle`（label 落到 "Zoom (Z)"）；Rust 15 个不含 Idle，另给 Idle 定义专属 label "No tool (A)" | `EditorSession.swift:89-98` | `core/src/document.rs:723-739,777` |
| 17 | `WandSettings.sampleSize`：Swift 是枚举；Rust 用 `sample_radius: usize` | `Document/MagicWand.swift:14` | `core/src/session.rs:59` |
| 18 | `beginTransform(persistent:)` 参数：Swift 供 nudge/duplicate 传 false；Rust 恒 true，`TransformEdit.persistent` 无人读 | `EditorSession.swift:375` | `core/src/session.rs:1156` |
| 19 | 画笔提交缺 `canPaint` 守卫（可见性/蒙版启用/调整层禁画）与 100MP 像素预算 | `EditorSession+Brush.swift:5-27` | `app/src/canvas.rs:427-439` |

### 3.2 缺失的会话操作（Swift 有 Rust 无，均给出 Swift 位置；Rust 侧为"无对应"）

多选状态机与批量删除（`LayerGroups.swift:96/107`、`EditorSession.swift:647`）；面板重排 `reorderLayers`（`EditorSession.swift:687`）；眼睛滑动批量显隐（`:673-685`）；导入管线 `importImages/drainImports/insert/insertPhotoshop`（`:720-855`，Rust core 无任何导入入口）；群组变换 `transformsAsGroup` 全套（`:301-448`）；Option 拖拽复制变换（`:396-470`）；`commitTransform` 的 floating/corners/mask 三分支（`:426-433`）；操作守卫 `canStartProjectOperation/canUseHistory/canEditLayers` 与 busy 状态机（`:118-169,563,600`）；工具族笔尖记忆 `parkedBrushTips`（`:223-344`）；`cycleToolMode` 的 Shape/Blur/SpotHealing/CloneStamp 分支（`:354-373`，app 只实现 4 种工具且缺 `toggleShapeKind` 等，`app/src/canvas.rs:789-816`）；ToolDefaults 持久化（`:198-279`）；文本/形状草稿守卫 `textDraft/shapeDraft`（`:233-241`）；`LayerHierarchy.entries/visibleLayers/validate` 面板行模型（`LayerGroups.swift:3-47`）；**文件夹有效透明度** `effectiveOpacity`（`LayerGroups.swift:53-83`，Rust 全仓库无 `effective_opacity`，合成只用自身透明度 `render.rs:233`）；`groupSelectedLayers/addGroup/toggleGroupExpansion/canPlaceLayer/placeLayer/moveActiveLayerOutOfGroup`（`LayerGroups.swift:119-231`）；蒙版：从选区建蒙版 `addMask(revealing:)`（`LayerMask.swift:233-260`，Rust `add_layer_mask` 只能建纯色 `session.rs:1072`）、`selectLayerTarget`（`:222-228`）、跨层复制蒙版 `canCopyMask/copyMask`（`:290-298`）、链接开关 `toggleMaskLink`（`:312`，Rust `is_linked` 字段存在但无操作改它）、`displayedMaskPlacement`（`:324-338`）、蒙版扭曲预览缓存（`:369-394`）；活蒙版（剪贴蒙版）：`linkMask/removeLiveMask/toggleClippingMask/adoptClipping` 全套（`LiveLayerMask.swift:23-225`，Rust 只有加载校验与渲染期直乘 `render.rs:276`）、删除时 bake 询问 `deleteWithLiveMaskChoice`（`:104-149`）、`LiveMaskBaker.bake`（`:74-97`）；外观：`previewBlendMode` 设置/清除方法（`LayerAppearance.swift:78`）、`setSelectedLayersOpacity` 多选一步（`:108-119`）；调整图层：`addAdjustment/updateAdjustment`（`LayerAdjustment.swift:184-206`，**Rust 造不出调整图层**，渲染端 `render.rs:129-131` 会用但无从创建）；参考线交互全套 `GuideDrag/hitGuide/beginGuideCreation/beginGuideMove/finishGuideDrag` 与吸附 `snappedGuidePosition`（`Guides.swift:58-249`）；调色板/取色器会话 `openColorPicker` 全套（`ColorPalette.swift:60-180`）；`duplicateLayer(in:above:)/addPixelLayer/nextLayerName`（`SelectionClipboard.swift:174-203`）。

### 3.3 简化

- `LayerMask` 表示：Swift 单通道 DeviceGray + `isValid` 校验 + 96px 缩略图（`LayerMask.swift:6,23,35`）；Rust 是 RGBA `Bitmap`（`document.rs:242-311`）。
- `clipImage` 的 LRU 缓存（8 项/64MP，`LayerMask.swift:110-166`）→ Rust `render.rs:27-57` 每次重算。
- `add_blank_layer` 插入点：Swift 选中文件夹时插入文件夹内容顶部并展开（`EditorSession.swift:614-628`）；Rust 只做"活动层上方"，`collapsed_group_ids`（`session.rs:132`）是从未读写的死字段。
- `selectedLayerIDs`：`Set<UUID>` 带有效性交集 → `Vec<Uuid>` 无去重过滤（`session.rs:130`）。
- EditorSession 的 UI/异步状态（viewport、effectsPreviews、filterEdit、pixelClipboard、colorPicker、renamingLayerID 等，`EditorSession.swift:104-292`）无 core 对应，部分由 app 临时状态替代。

---

## 4. 变换 / 画布 / 项目格式（B 域）

### 4.1 缺失

| # | 差异 | Swift 位置 | Rust 位置 |
| --- | --- | --- | --- |
| 1 | DistortWarp 全套（homography/warp/carried/warpMask/mapPath + session distort 流程） | `Document/Distort.swift:8-238,260-392` | ✅ 已移植（`tools/distort.rs`；corners 由 `commit_transform` 的 distort 分支消费） |
| 2 | CropSnap 裁剪边吸附（move 吸双边/resize 吸被拖边/symmetric 镜像半宽） | `Document/Crop.swift:49-112` | ✅ 已移植（`core/canvas.rs` `CropSnap` + `drag_crop` 集成，见顶部修复进度） |
| 3 | 变换/裁剪对齐吸附目标 `transformSnapTargets/snappedMove/cropSnapTargets/alignmentSnapTargets` | `Crop.swift:116-142`、`Guides.swift:182-220` | ✅ 已移植并接入（session `alignment_snap_targets`/`transform_snap_targets`/`snapped_move`/`crop_snap_targets`；MoveLayer 拖拽与 drag_crop 已消费） |
| 4 | `flip_canvas` 不镜像选区（Swift 用仿射镜像 `selection.path`） | `Document/LayerFlip.swift:69-77` | `core/src/session.rs:1266-1281` |
| 5 | `flip_layers` 不支持多选/组翻转 | `LayerFlip.swift:24-51` | `core/src/session.rs:1219-1246` |
| 6 | Canvas Size 的 fill（"Canvas Extension" 有色扩展层）+ 像素/图层数预算 + 颜色校验 | `IO/CanvasResizer.swift:8-15,23,36-69` | `core/src/canvas.rs:142-158` 完全忽略 `options.fill`（`CanvasExtensionColor` 定义于 `canvas.rs:96-101` 无使用者）；`app/src/sheets_ui.rs:695` 也不传 |
| 7 | CanvasResizer 防御校验（1…30000、同尺寸早退、逐层 transform.isValid） | `IO/CanvasResizer.swift:8-23` | `core/src/canvas.rs:142-158` 无校验（同尺寸时还会清空 selection） |
| 8 | 参考线拖拽/标尺交互全套 | `Guides.swift:91-179` | `core/src/session.rs:1409-1431` 仅 `clear_guides/add_guide` |

### 4.2 行为差异

| # | 差异 | Swift 位置 | Rust 位置 |
| --- | --- | --- | --- |
| 1 | ImageResizer 旋转层：Swift 取 4 角投影后缩放的包围盒，输出 transform 的 rotation/flip **归零**（ImageSizeTests.swift:56 断言）；Rust 线性缩放 origin/size 并保留 rotation/flip → 旋转/非均匀缩放层尺寸与位置不一致 | `IO/ImageResizer.swift:36-43` | `core/src/canvas.rs:179-182` |
| 2 | 1×1 均匀 mask：Swift 原样保留（分辨率无关，`ImageResizer.swift:77`）；Rust 无条件重采样放大。mask placement 缩放公式同样不同（Swift 用 `placing(unitToDocument ∘ scale)`，`ImageResizer.swift:97`；Rust 线性缩放，`canvas.rs:198-201`） | 同上 | `core/src/canvas.rs:191-201` |
| 3 | 原子保存强度：Swift `NSFileCoordinator(.forReplacing)` + FileWrapper atomic（旧包在交换前完好）；Rust 先 `remove_dir_all` 旧包再 rename 暂存包，两步间崩溃丢包 | `IO/ProjectStore.swift:111-118` | `core/src/project/store.rs:512-528` |
| 4 | PNG 严格性：Swift 拒多帧 APNG 且逐属性查 depth≤8（`ProjectStore.swift:155-161`）；Rust 只拒 16-bit 色型，多帧按首帧接受 | 同上 | `core/src/project/store.rs:299-307` |
| 5 | `resize_image_to` 缺 resolution（1…9600）与 100MP 总像素预算校验；Swift 同尺寸时只改 resolution 早退 | `IO/ImageResizer.swift:15-25` | `core/src/session.rs:1514-1523` |
| 6 | Image Size 无 sampling 选项（Swift `ImageSizeOptions.sampling`，测试用 `.nearest`）；Rust 硬编码 `LayerSampling::High` | `IO/ImageResizer.swift:4-9` | `core/src/canvas.rs:187,195` |
| 7 | `LayerSampling` serde 默认值内部不一致：enum 级 `#[default] Smooth`（`transform.rs:10-11`），manifest 路径默认 High（`project/manifest.rs:202-207`）；Swift 字段默认 `.high`（`LayerTransform.swift:24`）——若走 CanvasDocument serde 路径持久化将与 Swift 分叉 | `Document/LayerTransform.swift:24` | `core/src/transform.rs:8-11`、`project/manifest.rs:202-207` |

### 4.3 简化/多出

- `TransformGroup.originals` 用 `Vec<(Uuid, LayerTransform)>` 代替字典（`transform.rs:212`，语义等价）。
- trim 渲染入口：Swift 经 `ImageExporter.render`（含 effects）；Rust 用 `draw_live_composite`（`canvas.rs:438`）。
- 多出：`transform.rs:197-204` `LayerTransform::corners()`（公式正确但无人调用）；`canvas.rs:260-269` `standardize()`。

---

## 5. 绘画 / 修复 / 变形工具（E 域）

### 5.1 确认的 bug（两边都实现但 Rust 结果错误）

| # | 差异 | Swift 位置 | Rust 位置 |
| --- | --- | --- | --- |
| 1 | **heal ring 膨胀行扫描 clamp 维度错误**：C 行扫描 `hi` 钳到行宽 `ww`；Rust 误写 `.min(wh)`。工作盒 `wh>ww` 且 `x+ring+1>ww` 时读prefix陈旧单元，RING 集合偏薄，修复结果偏离。非方盒 + ring≥2 必现 | `Rendering/HealPixels.c:143` | `core/src/tools/heal.rs:249`（列扫描 `heal.rs:264` 用 `wh` 是对的） |
| 2 | **heal 拒修单行/单列 spot**：C 的 `heal_coverage_bounds` 只在全空时清零；Rust `coverage_bounds` 把宽或高为 1 的 coverage 判 `None` → `spot_heal` 返回 false | `Rendering/HealPixels.c:8-22` | `core/src/raster.rs:675`、`tools/heal.rs:201-204` |
| 3 | **渐变"前景色→透明"（默认样式）损坏**：Swift `[fg α=1, fg α=0]`（`Gradient.swift:78-80`）；Rust app 传 `[fg, fg]` 且 straight 两端 α=255 → 退化为不透明纯色填充（core `fill_gradient` 本身支持 alpha 插值，是 app 传参错） | `Document/Gradient.swift:78-80` | `app/src/state.rs:430-439,603-610` |

### 5.2 行为差异

| # | 差异 | Swift 位置 | Rust 位置 |
| --- | --- | --- | --- |
| 1 | heal 修复区域：Swift 限制在 `painted.insetBy(-reach)`（`reach=(max(w,h)+32)*3.2`）内搜索 patch；Rust 全图层网格搜索 → 大图层上选中的源 patch 可不同 | `Document/BrushStroke.swift:776-784` | `core/src/tools/heal.rs:192-224` |
| 2 | 笔画 provisional tail 的 coverage 撤销：Swift `drawTail` 保存 tail 可达 tile 的 coverage 图，`removeTail` 精确恢复；Rust 只恢复间距状态，tail 已画的 dab 保留（软笔刷下每次 pointer 事件留直线 tail 多余覆盖） | `BrushStroke.swift:354-387` | `core/src/tools/brush.rs:284-293` |
| 3 | GPU 连续沉积模型缺失：Swift 默认 `useGPU=true` 走 `appendContinuous`（coverage=1−exp(−∫tipDensity)，0.2px 自适应细分）；Rust 是 CPU dab 路径 | `Rendering/MetalBrushCoverage.swift:70-161`、`BrushStroke.swift:149-150,224,281-309` | `core/src/tools/brush.rs`（无 continuous 实现） |
| 4 | 旋转/非均匀缩放图层上的笔刷形状**语义相反**：Swift CPU dab 在文档空间画圆（保证图层上仍是圆）；Rust 只映射圆心、tip 在图层像素空间盖章 → 旋转图层上画出文档空间椭圆 | `BrushStroke.swift:546-547,559-560`（测试 BrushTests.swift:197） | `core/src/tools/brush.rs:360-374` |
| 5 | Clone Stamp aligned 语义缺失：Swift aligned 时复用首笔 `cloneOffset`（取整）；Rust 每次起笔重算（恒 unaligned），`session.rs:148` `clone_offset` 从未写入，`CloneSettings.aligned` 是死配置 | `Document/CloneStamp.swift:23-27`、`EditorSession+Brush.swift:39` | `app/src/canvas.rs:465-472` |
| 6 | Gradient 预览/提交：Swift `GradientEdit` 挂起可重画、切工具自动提交、支持蒙版渐变；Rust 拖完即提交、`mask:false`（蒙版渐变不可用） | `Document/Gradient.swift:36-108` | `app/src/canvas.rs:682-707`（`:699`） |
| 7 | Blur 采样对象：Swift 画的是 displayedTransform 后的画布显示（含蒙版分支）；Rust 直接对 `layer.asset` 原图做高斯，无变换/蒙版 | `Document/BlurTool.swift:8-40` | `app/src/canvas.rs:475-482` |
| 8 | Clone/Blur 采样源：Swift 经 displayedTransform 画到文档尺寸（含 sampleAllLayers 合成）；Rust 单图层时直取原始 asset（变换≠单位阵时错位） | `Document/CloneStamp.swift:39-48` | `app/src/canvas.rs:488-496` |
| 9 | heal 随机种子：Swift 每次 `UInt32.random`；Rust 固定 `0x6d2b79f5` | `BrushStroke.swift:793` | `app/src/canvas.rs:510` |
| 10 | `replaces_with_clone` 的 copy 混合：Swift `.copy` blend 结果 = src×(cov×opacity) 不保留 dst；Rust `out*(1-a)+s*a` 保留 dst（半覆盖像素不同，影响 finishWarp/Blur 软边） | `BrushStroke.swift:464` | `core/src/tools/brush.rs:461-468` |
| 11 | `fill()` 是否吃 opacity：Swift 不吃（直接 fill）；Rust 乘 `settings.opacity`（改过笔刷不透明度后填充变淡） | `BrushStroke.swift:626-631` | `core/src/tools/brush.rs:607-638` |
| 12 | 形状工具：Swift `finishShape` 后 uncommitted 时实时重渲（缩放=重绘）；Rust `add_shape_layer` 立即写 asset → 形状缩放是重采样拉伸非重绘；缺 Option 中心放大、直线 45° 吸附、`toggleShapeKind`/`redrawShape` | `Document/ShapeTool.swift:84-100,161-194` | `core/src/session.rs:517-556`、`app/src/canvas.rs:316-322` |
| 13 | warp finish：Swift 校验图层未被并发修改且经选区剪裁；Rust 无守卫、`selection_clip: None`（结果可越界涂写） | `Document/SmudgeLiquify.swift:192-201` | `core/src/tools/warp.rs:248-283` |
| 14 | move_lifted 采样：Swift 整像素 `.none`/分数 `.high` 插值；Rust 恒 Nearest，punch-out 用截断 `as u8` 非四舍五入 | `BrushStroke.swift:737` | `core/src/tools/brush.rs:719,724-729` |

### 5.3 缺失（本域内）

TypeTool 排版整体：Swift `textAttributes/textBoxSize/textImage`（字体/字距/行距/换行/对齐/padding/100k UTF-16 等校验，`TypeTool.swift:205-255`）+ 会话 `beginText/editActiveText/applyText`（`:67-196`）；Rust 实际渲染是 `app/src/text_bitmap.rs:1-134` 的 **5×7 点阵字体（76 字形）**，`font_name/alignment/tracking/leading/box_size` 全部被忽略（数据模型 `document.rs:331-427` 已移植但不被渲染消费）。（✅ 已移植：core `tools/text.rs` 真字体排版 + session 全流程；app 点阵字体已删除，见顶部修复进度。遗留：富文本样式面板 TypeControls 的 UI。）笔刷平滑 smoothing（`EditorSession+Brush.swift:77-87`）UI 有滑杆但路径不消费；笔刷参数校验与 100MP 预算（`BrushStroke.swift:173-177`）无；笔刷快捷键（`[`/`]`/Shift-[`/`]`/数字键不透明度，`EditorSession+Brush.swift:191-226`）无；蒙版绘画在 app 不可达（`app/src/canvas.rs:459` 硬编码 `is_mask: false`，core 支持）；`BrushCommit` 后台提交 actor（`BrushStroke.swift:844-902`）无。

### 5.4 简化

256px 瓦片引擎 → 整网缓冲（`brush.rs:5-10` 自述）；tip 渲染近似（CG 解析抗锯齿/25 站径向渐变 → 逐像素解析，非 bit-exact）；tip 两级缓存（stamp ≤160px / gridTip ≤3000px）→ 每次重算；clone 采样 CG `.medium` → 最近邻（`brush.rs:456-460`）。

### 5.5 多出

`ShapeKind::RoundedRectangle` 作为独立种类（Swift 圆角是 rectangle 的 `cornerRadius` 属性，`ShapeTool.swift:3-15`；存档 kind 枚举与 Swift manifest 不一致，`document.rs:470-480`）；`LayerShape.filled` 空心形状（Swift 恒填充）；`LayerShape.start/end` 非 Optional（Swift 是 `CGPoint?` 带 legacy 内缩语义，Rust 无 legacy 回退，`render.rs:337-344`）。

---

## 6. 渲染 / 效果 / 滤镜 / 混合（F 域）

### 6.1 行为差异（按影响排序）

| # | 差异 | Swift 位置 | Rust 位置 |
| --- | --- | --- | --- |
| 1 | **调整图层作用域语义完全不同**：Swift 由 LiveMaskRenderer 拦截，对**画布上已合成的下方内容**应用调整，支持自身蒙版裁剪作用域与有效不透明度作用强度；Rust 对**该层自己的像素**应用后上画（`render.rs` 测试固化了这种"自己改自己"行为） | `Rendering/LiveMaskRenderer.swift:21-60,84-90`、`EditorCanvas.swift:954-966` | `core/src/render.rs:129-131`（测试 `render.rs:600-623`） |
| 2 | **带"蒙版+效果"的图层被双重蒙版且效果越界采样错位**：Swift 把 mask 烘进 shown 后以 `mask: nil` 绘制效果图（效果不被蒙版裁剪）；Rust 先烘进 shown、又用 own_mask 对 placed alpha 再乘一次（coverage 被平方），且 doc_to_grid 映射的是 grown 网格而 own_mask 是原始尺寸 → 阴影/辉光/描边被 mask 边缘值错误调制或裁掉 | `Document/LayerEffects.swift:445`、`EditorCanvas.swift:878-901` | `core/src/render.rs:143-146,177-185,206-229,254-271` |
| 3 | **HardMix 公式错误**：Rust 用 `VividLight(cb,cs) < 0.5`；PDF/Photoshop 是 `Cb ≤ 1−Cs`。cs>0.5 时阈值错位（如 cb=0.3, cs=0.9：Rust→0，PS→1）；cb+cs=1 时两边相反。现有测试 (0.6,0.7) 恰好落在同值区域未暴露 | `Rendering/SeparableBlend.swift`（PDF 公式） | `core/src/blend.rs:224-231`（测试 `blend.rs:443-445`） |
| 4 | **`composite_pixel` 半透明 backdrop 偏离 PDF**：Rust `co = B·αs + Cb·αb·(1−αs)`；PDF 为 `co = αs(1−αb)·Cs + αsαb·B + αb(1−αs)·Cb`。αb∈(0,1) 时 Rust 用混合色 B 替代了应显示的源色 Cs（50% 不透明底上叠 Multiply 白层：PDF 白，Rust 偏灰黑） | `Rendering/SeparableBlend.swift:22-46` | `core/src/blend.rs:373` |
| 5 | Divide 零源特例：Rust `cs≤0 && cb≤0 → 0`（黑）；PDF `Cs=0 → 1`（白） | PDF 公式 | `core/src/blend.rs:235-245` |
| 6 | 剪贴蒙版 source 的 coverage：Swift 经完整 `drawOwn` 渲染源层（含效果/有效不透明度/自身蒙版/递归剪贴）；Rust 只取 source.asset 原始像素 alpha 与 transform | `Rendering/LiveMaskRenderer.swift:129-145` | `core/src/render.rs:276-304` |
| 7 | 剪贴组成组语义：Swift 连续剪贴子层在共享 group surface 内先互相合成、整组以 base 的 blend mode 上画、基于相邻性；Rust 每层各自乘 source alpha 后以自己的 blend mode 对整画布合成，`mask_source_id` 不要求相邻/同父 | `LiveMaskRenderer.swift:63-111` | `core/src/render.rs:234-250` |
| 8 | 文件夹不透明度不参与合成：Swift `effectiveOpacity` 逐层乘祖先 folder opacity；Rust 只用自身 opacity | `LayerGroups.swift:63-71`、`EditorCanvas.swift:817` | `core/src/render.rs:233` |
| 9 | Gaussian Blur 滤镜边界：Swift 刻意不 clamp（边缘向透明扩散）+ grow→blur→trim 流程；Rust 边缘复制 + app 无 pad/trim → 层边缘结果方向相反 | `Document/Filters.swift:115-133,146-148,164-166,297-337,502-517` | `core/src/raster.rs:550-589,623-634`、`app/src/state.rs:564-587` |
| 10 | Motion Blur profile：Swift `CIMotionBlur` 锥形衰减；Rust 均匀 box + floor 整像素采样（半径系数 1/√12 一致） | `Filters.swift:135-138,167-173` | `core/src/filters.rs:343,401-441` |
| 11 | halving 算法：Swift vImage **Lanczos** + 8px padding + 尺寸向上取整 `(w+1)/2`；Rust 2×2 box + 向下取整 `w/2` | `Rendering/DownsampleCache.swift:82-111` | `core/src/raster.rs:389-411` |
| 12 | 预缩小阈值差一档：Rust `scale ≤ 0.5` 即 halve；Swift `factor < 0.5` 才启用（scale 恰 0.5 时 Rust 多缩一次）；且 Swift 缩小时非 Nearest 一律 Low、Rust 无 Low/Medium 概念 | `DownsampleCache.swift:32-35`、`Rendering/LayerRenderer.swift:42-44` | `core/src/raster.rs:502-518,293-297` |
| 13 | Add Noise seed：Swift 每次编辑随机（`Filters.swift:223`）；Rust app 硬编码 7（`state.rs:579`） | `Filters.swift:223` | `app/src/state.rs:579` |
| 14 | 选区相关渲染差异（与 D 域交叉）：蚂蚁线颜色相反且无动画（Swift 白实线+黑虚线相位动画 0.12s；Rust 黑实线+白虚线静止） | `Rendering/TransformOverlay.swift:196-210`、`EditorCanvas.swift:2001-2015` | `app/src/canvas.rs:914-921,1101-1106` |

### 6.2 缺失/简化

- `run_filter` 在 core 内无调用方（`filters.rs:350`），由 app 直接对原尺寸图层像素调用，缺 blur padding 与 trim（`session.rs:1368` `trimmed` 只用于 Merge Down）。
- `render_effects` 无结果缓存（Swift `LayerEffects.swift:375-408` cache + EffectsPreviewCache）；无 100MP/80M 尺寸 guard（`LayerEffects.swift:441`、`MetalLayerEffects.swift:61`）；GPU 路径 inner glow 的 `size>0` gate（`MetalLayerEffects.swift:155`）Rust 无（`effects.rs:713`）。
- `mask_clip_image`（`render.rs:27-57`）无 limit 交互降采样与 placement 缓存。
- effects coverage 的 Gaussian 4σ 截断 vs CI 约 3σ（`raster.rs:554,596`）。

---

## 7. 调整（C 域）

### 7.1 行为差异/简化

| # | 差异 | Swift 位置 | Rust 位置 |
| --- | --- | --- | --- |
| 1 | **app 层 `levels_auto` 与原版算法不同**：Swift 三模式（Contrast/Color/Neutral）、单通道 256 bin、clip 0.001、Neutral gamma=log(mean)/log(0.5)；Rust 对 1024 个混合 bin 求 0.5%/99.5% 分位、无三模式、无 gamma | `Document/LevelsAutomatic.swift:4-37` | `app/src/sheets.rs:382-422` |
| 2 | HueSaturation 渲染路径：Swift 构建 33³ CIColorCube（三线性 + f32 量化）应用；Rust 逐像素 f64 精确计算 HSL——语义等价但输出非 1:1（`dimension=33`、`previewLimit=8000` 无对应） | `Document/HueSaturation.swift:234-243,383-405` | `core/src/adjust/hue_saturation.rs:409-448`、`adjust/mod.rs:27-47` |
| 3 | 各 `apply` 缺 `is_valid` 检查：Swift 在 CameraRaw/Curves/Exposure/GradientMap/BlackWhite/ColorBalance/Grain 的 `apply` 均先 `guard isValid else throw`；Rust 全部静默处理 | `Document/CameraRaw.swift:224-225`、`Curves.swift:34`、`ImageAdjustments.swift:65,89,133,166,202` | `core/src/adjust/camera_raw.rs:1584-1692`（`is_valid` 定义于 `:1426` 未被 apply 调用）、`curves.rs:92-103`、`image_adjustments.rs:123-595` |
| 4 | 选区 coverage 生成：Swift 把路径经逆变换后重新光栅化（亚像素精确）；Rust 对预生成 coverage 位图做 floor 最近邻查找（旋转/缩放图层边缘不一致），且 `pixel.rs:25-33` 有一段无效死代码 | `Document/PixelAdjust.swift:23-34` | `core/src/adjust/pixel.rs:10-52` |
| 5 | 选区回混精度：Swift `CIBlendWithMask`（浮点）；Rust 整数 `(adj·a+orig·(255−a)+127)/255`（可差 1） | `PixelAdjust.swift:38-46` | `core/src/adjust/pixel.rs:58-82` |
| 6 | GuidedMatte 重采样：Swift CGContext `.high` 一步缩放；Rust halve+resample 组合，滤波器不同非位等（guided_filter 本体 epsilon=1e-4 一致） | `GuidedMatte.swift:68-117` | `core/src/adjust/pixel.rs:257-281` |
| 7 | 几何 warp：Swift 用 `CIPerspectiveTransform`（越界透明）；Rust 手写单应 + bilinear（越界 clamp 到边缘）→ 边缘像素外推行为不同 | `CameraRawGeometryCalibration.swift:77-83` | `core/src/adjust/camera_raw.rs:3130-3182`、`raster.rs:305-334` |
| 8 | App 层 Levels/HueSaturation 提交绕过选区：Swift 两滤镜都经选区混合；Rust 直接整层应用 | `Levels.swift:78-83`、`HueSaturation.swift:244-250` | `app/src/sheets.rs:262-300`、`state.rs:537-561` |
| 9 | colorize 饱和度下限 bug：Rust `saturation.max(1.0)`（滑块原始 −100..100 取 max(1.0)）抬到 1%；Swift 是 `settings.saturation/100` 后 clamp 0…1（可为 0） | `HueSaturation.swift:301` | `app/src/sheets.rs:290` |
| 10 | `FilterJob.visualize_point_color` 默认 0 而非 −1 → 每次滤镜预览都进 visualize 分支（无视觉影响但属偏差） | `CameraRaw.swift:221` | `app/src/state.rs:586`、`camera_raw.rs:1604-1609` |

### 7.2 缺失（本域）

- **LevelsAutomatic 整文件**（见 §2 表）。
- CameraRaw 会话交互：白平衡吸管流程、自动白平衡扫描、defringe 吸管（**purpleCenter=290.0 / greenCenter=90.0 / span=25.0 在 Rust 任何位置不存在**，`CameraRaw.swift:462-507`）、RGB 读数（`:510-527`）、定向拖拽（`:529-560`）、点颜色采样（`:562-576,609-637`）、几何参考线（`:578-607`）、`CameraRawScope.preview`（`:400-415`）——底层数学（neutralize/autoBalance，`camera_raw.rs:1808-1859`）已移植。（✅ 常量与 defringe/自动白平衡/RGB 读数已移植，见顶部修复进度；白平衡吸管的 CameraRawScope.preview 与定向拖拽属渲染/视图层未移植。）
- Levels 直方图的选区 coverage 加权：core 有参数位（`levels.rs:145`）但 app 永传 `None`（`app/src/sheets.rs:146-151`）。

---

## 8. 选区（D 域）

### 8.1 行为差异

| # | 差异 | Swift 位置 | Rust 位置 |
| --- | --- | --- | --- |
| 1 | `coverage_bounds` ceil 次序：Swift `ceil(feather*2)`；Rust `(ceil(feather))*2`（feather 非整数时外扩不同：如 feather=4.2426 → Swift 9，Rust 10） | `Document/Selection.swift:35` | `core/src/selection.rs:432` |
| 2 | contains/rasterize 用偶奇规则；Swift 用 winding（自交套索结果不同） | `Selection.swift:22,259` | `core/src/selection.rs:134-155,502-529,544-562` |
| 3 | 椭圆：Swift `addEllipse` 4 段 Bézier；Rust 96 段折线（边界像素覆盖细微差） | `Selection.swift:218` | `core/src/selection.rs:81-93` |
| 4 | 环→多边形配洞按外接矩形取第一个命中：三层嵌套（岛中湖）配错父级、语义反转；CGPath 原生支持任意嵌套 | （CGPath 原生） | `core/src/selection.rs:320-353`（`:339-346` 的 break） |
| 5 | expand/contract 描边带：Swift 整条路径一次 `strokingWithWidth`；Rust 逐环 offset+作差再并，凸角圆弧 ≤32 段、极小环跳过 → 极小环上 contract 不收缩 | `Selection.swift:336` | `core/src/selection.rs:173-194,233-303`（`:283`） |
| 6 | coverage 光栅化：Swift CG 解析抗锯齿；Rust 4×4 超采样（16 级灰度）——README "scanline coverage" 名不符实 | `Selection.swift:19-22` | `core/src/selection.rs:505-529`（`SS=4` 于 `:511`） |
| 7 | 羽化模糊：σ=feather/2 一致，但 Swift 走 CI（工作色彩空间/自有核截断），Rust 4σ 截断 + 边缘复制（`raster.rs:623-634` 实为边缘复制非镜像） | `Selection.swift:27`、`PixelAdjust.swift:48-54` | `core/src/selection.rs:444`、`raster.rs:592-649` |
| 8 | magic_wand 空白图层行为：Swift 返回全透明画布图 → 全画布命中（MagicWandTests.swift:101-103 断言 200px 全选）；Rust 返回 None → 直接不选 | `MagicWand.swift:128-137` | `core/src/session.rs:1054-1064`、`:998-1000` |
| 9 | wand 守卫与错误：Swift `tooDetailed`（800 万边上限）写入 `brushError` 提示；Rust `Err(_) => return` **静默失败**；C 的 OOM 错误类别被折叠 | `MagicWand.swift:63,99,109`、`WandPixels.c:7` | `core/src/session.rs:987-997,1037` |
| 10 | 选区守卫削弱：`begin_lasso` 缺 canEditSelection/selectionMoveOrigin 检查；`can_modify_selection` 只查非空（**画套索草稿期间也能 Expand/Contract/Feather**）；`nudge_selection` 不取整、无来源原点、空选区也能移动；`drag_marquee`/`extend_lasso` 无 isFinite 检查 | `Selection.swift:143,157,166,272-277,293` | `core/src/session.rs:672-675,697-725,885-900` |

---

## 9. IO / PSD / UI 集成（G+H 域）

### 9.1 集成层缺失（影响最大的几条）

| # | 差异 | Swift 位置 | Rust 位置 |
| --- | --- | --- | --- |
| 1 | **PSD 导入未接入 app**：`open_document` 对 `.psd` 走 `image::open`（workspace 只启用 png/jpeg feature）→ 必然失败只显示 "Open failed"。README "Photoshop PSD files are imported" 目前不成立 | `Document/EditorSession.swift:741,764-776`、`IO/PSD/PSDReader.swift:10-27` | `app/src/sheets_ui.rs:829-897`（无任何 `PsdReader` 引用） |
| 2 | JPEG 导出整体缺失（`JPEGOptions`/质量/哑光色/DPI/预览） | `IO/ImageExporter.swift:95-154`、`UI/JPEGExportSheet.swift` | 无 |
| 3 | EXIF 方向、色彩管理（ICC→sRGB）、导入格式白名单（heic/tiff）、导入尺寸预算全缺 | `IO/ImageImporter.swift:37-52` | `app/src/sheets_ui.rs:855-866` |
| 4 | 导出无尺寸 guard、无 DPI 元数据（pHYs）、非原子写 | `ImageExporter.swift:22-23,80-82,111-138` | `app/src/sheets_ui.rs:783-807` |
| 5 | 拖放导入（ImageFileDrop）、"Open With"/Dock 打开、RAW 导入 | `IO/ImageFileDrop.swift:7-33`、`IO/CompositorApplicationDelegate.swift:14-22`、`IO/RawImporter.swift` | 无 |

### 9.2 PSD 细节差异

| # | 差异 | Swift 位置 | Rust 位置 |
| --- | --- | --- | --- |
| 1 | 图层名 Pascal 串解码：Swift macOSRoman（Latin-1 兜底）；Rust `from_utf8_lossy`（非 ASCII 单字节名变 U+FFFD） | `IO/PSD/PSDReader.swift:154` | `core/src/psd/reader.rs:184-186` |
| 2 | 闭合路径收口角：CG 对 closeSubpath 计算收口 miter join；Rust 重复首点按开放折线描边（无 joint） | `IO/PSD/PSDVector.swift:62-79` | `core/src/psd/vector.rs:160-169,620-722` |
| 3 | 矢量光栅化：CG 解析抗锯齿 winding → Rust 自写 8 样本 scanline（已声明替换） | 同上 | `core/src/psd/vector.rs:726-826` |
| 4 | fixture 全局 composite：Swift 经 context 重绘；Rust 尺寸不符时 resample 近似 | `CompositorTests/PSDFixture.swift:230-245` | `core/src/psd/fixture.rs:333-340` |
| 5 | live shape 模型：Swift `LayerShape = style + CGImage`（像素身份）；Rust 纯几何模型 + 合成默认 `line_width:4.0, filled:true` | `Document/ShapeTool.swift:37-53` | `core/src/psd/types.rs:246-258` |
| 6 | 蒙版转换失败报告消息不可达（Rust `layer_mask_asset` 无失败路径，可接受） | `IO/PSD/PSDDocumentBuilder.swift:95-99` | `core/src/psd/document_builder.rs:190-199` |

### 9.3 UI / 应用壳

**快捷键缺失**（`UI/KeyboardShortcuts.swift:74-124` vs `app/src/keys.rs:45-123`）：Save As(⇧⌘S)、Export JPEG(⌥⇧⌘S)、Close(⌘W 无键)、Show Transform Controls(⌘H)、剪贴板四件套(⌘X/⌘C/⇧⌘C/⌘V)、Fill(⌥⌫/⌘⌫/⇧⌫)、Select Subject(⌥⌘A)、Curves(⌘M)、Invert(⌘I)、Clipping Mask(⌥⌘G)、Move Layer Up/Down(⌘]/⌘[)、Snap 总开关(⇧⌘;)、Lock Guides(⌥⌘;)、Select 工具(A)、笔刷硬度(⇧[/⇧])、混合模式步进(⇧−/⇧=)、循环形状(⇧U)、不透明度数字键、移动选中像素(⌘方向 8 条)、完成文本(⌘⏎)、Tracking/Leading(⌥方向 8 条)、Levels 预览(⌥P)、临时手型(Space)。**键位不同**：Export PNG Rust 用 `ctrl-shift-s`（Swift ⇧⌘E，`keys.rs:72`）、Show Guides Rust `ctrl-shift-r`（Swift ⌘;，`keys.rs:100`）。**整个快捷键自定义系统缺失**（`KeyboardShortcuts.swift:129-319`）。

**菜单缺失**（`CompositorApp.swift:47-318` vs `app/src/menu.rs:34-117`）：Import Images、Save As、Export JPEG；Fit/Actual Pixels/Zoom 组；总 Snap 开关、Transform Controls 开关、Lock/Clear Guides；Cut/Copy/Copy Merged/Paste/Clear Selection Pixels/Content-Aware Fill；Select 菜单的 Layer's Pixels/Subject/Mask's Black Areas；Image 菜单的 Curves/Levels/Hue-Saturation（**sheet 存在但菜单无入口**）及 Black & White/Color Balance/Exposure/Gradient Map/Grain/Invert；Filter 菜单缺 Remove Background（core `filters.rs:26` 有 case）；Layer 菜单的 New Adjustment Layer 子菜单/Edit Adjustment/Transform Layer/Selection/Layer via Copy/Create-Release Clipping Mask/Move Out of Folder/Rename/Show-Hide/Move Up-Down/动态 Merge 与 Delete 标题。菜单项禁用逻辑基本未用（`ui.rs:85-88` 机制存在）。

**面板/控件**：完全缺失——ColorPickerSheet、EffectsSheet（core `effects.rs` 有内核无 UI）、CurvesControls、PSDConversionSheet、RawDevelopSheet、CameraRaw 的 Curve/Mixer/Grading/Detail/Optics/Geometry/Calibration 分区 + 直方图/示波器/剪裁指示/白平衡吸管（app 仅 14 滑块，`app/src/sheets.rs:189-213`）、浮动面板系统（全部为模态居中 overlay，`app/src/sheets_ui.rs:123-145`）、TransformInspector（Rust Move 头只有一个**空 handler** 的 Auto-select 勾选框，`app/src/tools.rs:500-507`）、TypeControls（Rust 只有一行提示文字，`tools.rs:512-514`）。**行为差异**：图层面板缩略图（Swift 整画布形状+棋盘格+蒙版边缘灰度，`UI/CanvasThumbnail.swift:10-62`；Rust 内容裁剪式无棋盘格，`core/src/render.rs:459-472`）；面板宽度固定 248（Swift 202–352 可拖记忆）；混合模式无悬停预览；标尺刻度候选集/目标间距不同且无 label（`UI/CanvasRulers.swift:174-184` vs `app/src/canvas.rs:1205-1214`）；标尺拖拽建参考线无调用；Camera Raw 默认值差异（Gaussian Blur 默认 4 vs Swift 1、Motion Blur 20 vs 10、Add Noise 12 vs 10，`app/src/sheets.rs:170-182` vs `Filters.swift:39-45`）；色板点击=交换而非打开取色器。（已修：app 建组原为 "Group N"+把活动层移进组+挂根，现改为调 core `add_group` 的 "Folder N" 语义，见 §3.1-13。）**多窗口**：单窗口固定 1560×920（`app/src/main.rs:30-49`），无标签条/关闭确认/未保存标记 UI（core 有 `is_modified`/`mark_saved` 未接 UI），CloseDocument 直接清空（`workspace.rs:236-244`）。**持久化**：无任何 UI 状态持久化。

**SwiftUI 内嵌逻辑的 Rust 对应**：`PickerHSB`/hex 换算已移植（`document.rs:49-115`，有测试）；CanvasThumbnail/SliderSnap/ArrowStepper/PanelResizeEdge/ProjectTab 宽度计算无对应。

---

## 10. 测试覆盖对比

Rust 全部测试为内联 `#[cfg(test)]`（`crates/core/tests/` 为空目录）；**crates/app 零测试**。约 300 个 Swift 用例中已移植约六成（像素内核/几何/PSD 覆盖最好），缺口集中在会话级与 UI 级：

| Swift 测试文件 | 状态 |
| --- | --- |
| PSDRoundTripTests (21) | 19 移植，2 缺（依赖 session 导入/确认回调） |
| CameraRawTests (20) | 大部分移植（数学），会话流程用例缺 |
| SelectionTests (23) | 约 1/3 有对应；MaskTracing/像素移动/轮廓拖动类全缺 |
| TransformTests (13) | 缺 9（duplicate/autoSelect/hover 命中/逐 handle 遍历等） |
| BrushTests (19) | 缺 16（tail 精确替换、旋转图层圆形、蒙版预览等） |
| HueSaturationTests (12) | 数学覆盖，预览/会话/色域独立调整缺 |
| SelectionEditTests (16) | 大部分缺（fill/clear/pixelMove/选区建蒙版） |
| AdjustmentLayerTests (8) | 全缺（Rust 无法创建调整图层） |
| TypeToolTests (8)、ShapeToolTests (4)、CloneStampTests (2)、SelectionClipboardTests (9)、GroupingSelectionTests (3)、GroupTests (4)、DistortTests (3)、ProjectWorkspaceTests (5)、TiledLayerTests (6)、RasterSnapshotTests (5)、DownsampleTests (4)、InnerGlow/OuterGlowTests (18) | **全部缺** |
| ExportTests (3)、JPEGExportTests (2)、ImageImportTests (8) | 全缺（app 无测试；JPEG/HEIC/TIFF 功能本身缺失） |
| UI 相关 7 文件 (30 用例) | 仅 ColorPicker 的 3 个数学用例移植（`document.rs:787-816`），其余缺 |

另有空测试：`history.rs:381` `transform_not_saved_blocks_nothing` 无有效断言。git 未跟踪的 `fix1.py`–`fix4.py` 是开发期用脚本改测试/修 bug 的痕迹（fix3 修 `geometry.rs` concat、fix4 改 `transform.rs` 测试），建议处理后删除或入库。

---

## 11. 修复优先级建议（按影响×面广度）

1. **渲染语义三件套**：调整图层作用域（§6.1-1）、"蒙版+效果"双重蒙版/错位（§6.1-2）、剪贴蒙版 coverage/成组语义（§6.1-6/7）——影响所有含调整层/效果/剪贴组的文档渲染正确性。
2. **确认的公式 bug**：HardMix（§6.1-3）、composite_pixel 半透明 backdrop（§6.1-4/5）、heal clamp（§5.1-1）、heal 单行/列拒修（§5.1-2）、渐变前景→透明（§5.1-3）、coverage ceil 次序（§8.1-1）。
3. **app 接线缺失**：PSD 导入分发、JPEG 导出、剪贴板四件套、菜单快捷键批量补齐、EXIF/导入预算。
4. **会话操作补齐**：多选/成组/重排、导入管线、选区编辑（fill/clear/invert/pixelMove）、浮动变换、蒙版操作（从选区建/复制/链接/单独变换）、调整图层创建与编辑、Distort。
5. **算法对齐**：levels_auto 重写为原版三模式、ImageResizer 旋转层/1×1 mask、TypeTool 真字体排版、笔刷文档空间圆形/tail 撤销/aligned clone。
6. **小修**：is_valid 守卫、serde 默认值统一、colorize 饱和度、undo 名、删除层后选中邻层、`fix*.py` 清理。

## 12. README 更正建议

- `README.md:40` "manifest JSON v1–8 semantics" → 实际两边均为 **v1–9**。
- `README.md:23` "antialiased scanline coverage" → 实为 4×4 超采样；wand "same hash" 中 hash 不存在。
- `README.md:33` brush 行 "clone/erase/heal wash, gradients" → aligned clone 缺失、前景→透明渐变损坏；"pixel moves" 属 RasterEdit 半成品无会话入口。
- `README.md:34` heal "kernel-exact" → 有 §5.1-1/2 两处偏差。
- `README.md:26` "LayerRenderer/TiledLayerRenderer semantics" → 无 tiling，且调整图层/剪贴蒙版语义与 Swift 不同（§6.1-1/6/7）。
- `README.md:41,63` "PSD … imported" → app 未接线（§9.1-1）。
- `README.md:27` canvas.rs "复刻 ImageResizer" → 旋转层与 1×1 mask 行为不同（§4.2-1/2），CanvasResizer fill 缺失。
