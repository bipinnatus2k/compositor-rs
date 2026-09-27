# compositor-rs 与 Swift 原版 Compositor 差异报表

- 对比基线：`Compositor/`（Swift/C 原版，约 39,300 行，193 个源文件）vs `crates/core` + `crates/app`（Rust 重写，约 30,700 行，52 个 `.rs` 文件）。
- 对比方法：按 8 个功能域逐文件通读两侧代码，逐公式/常量/守卫核对；关键结论均经 grep/Read 复核行号。
- 日期：2026-09-24。所有路径为仓库根相对路径。

## 修复进度（2026-09-27 第二十轮：标尺参考线/像素移动/工具默认值/快捷键设置 UI/测试扩容）

三个方向并行（标尺参考线+PixelMove / ToolDefaults+快捷键设置 UI / 会话测试套件），主线完成接线与收尾。测试 659 → **748** 全绿（core lib 575 + 集成 104 + app lib 69）。

**§4.1-8 标尺参考线拖拽（交互闭环）**
- 标尺条按下建参考线（轴/文档坐标换算）、拖动跟随、释放于标尺区删除（`finishGuideDrag(delete:)` 语义）、Move 工具按下 5pt 容差拾取现有线；Escape 取消链插入 guide 分支；画布按 displayedGuides 渲染拖拽中的线。4 个新测试。

**§2/§3.2 像素移动 + 选区轮廓拖动（SelectionEdits 最后缺口）**
- PixelMove 会话：Cmd 拖把选区像素 lift 出栅格，每次拖动重建"原位挖洞 + 偏移贴回"预览（文档在提交前不变），提交一步 "Move Pixels"/"Option→Duplicate Pixels" undo 同时移动轮廓；与 Cmd-T 浮动变换互斥（守卫）。蚂蚁线跟随 `displayed_selection`。
- 选区轮廓拖动（Selection.swift:263-290 整段）：Replace 模式在选区内直接拖动只移动轮廓（整像素取整、不重裁剪、可拖出画布往返）；⌘方向键 8 条入快捷键目录（`nudge_pixels`）。

**§2 ToolDefaults 持久化 + §9.3 快捷键设置 UI**
- `settings.rs`：ToolDefaults.swift 1:1——12 个开关键（autoSelect/transformControls/pixelGrid/grid/guides/rulers/snap×4/lockGuides）版本化 JSON 持久化（平台配置目录、原子写、损坏回退），启动应用 + didSet 式即时保存（含 Auto-select 勾选）。
- Shortcuts sheet 从静态列表升级为 KeyboardShortcuts.swift:129-319 的设置编辑器：分组行、整键捕获录制（`intercept_keystrokes`）、每行 Reset、冲突/非法以 ShortcutProblem 文案显示、Restore Defaults、Done 原子保存（生效需重启，界面注明）。

**§9.1-4 导出 DPI 接线**：PNG 导出经 `png_data_with_dpi`（pHYs）、JPEG 经 `jpeg_data_with_dpi`（JFIF 密度）携带文档分辨率。

**§10 测试扩容（+76 集成用例）**
- SelectionEditTests（14）、SelectionClipboard+Group+GroupingSelection（15）、TransformTests 会话流（9）、BrushTests（15）、AdjustmentLayerTests（9）、Distort/ShapeTool/CloneStamp/TypeTool 会话流（14）——六个新 `crates/core/tests/` 文件。

**仍缺（后续批次）**：~~取色器 SV 场+色相条自绘控件~~ ✅ 已落地（color_picker_ui.rs 重写：SV 场 2x 位图缓存+hue 竖条+双环 marker+hex，滑杆组删除，10 个测试；R/G/B 数字输入待 state.rs 放开 PickerUi 结构后补）、~~RAW 导入~~ ✅ 已落地（`core::raw`，rawler 0.8.0 默认 develop=as-shot 白平衡+相机矩阵→sRGB 8bit，`decode_image` 分发、扩展名白名单 29 格式；端到端测试用手写最小 CFA DNG 走完整 import_images 管线。此前记录的 rayon-core "冲突"实为首次 `cargo add` 半途写入的污染 lock——干净 lock 下 0.12.6 正常解析统一于 1.13.0。develop 滑杆与 RawDevelopSheet 属后续批次）、浮动面板系统、多窗口/标签条、InlineTextEditor、TransformInspector 的 Esc/Return 与把手实时跟随、select_subject 的 Vision 替代源、快捷键即时生效（当前重启生效）、JPEG 导出预览缩略图接线、ToolDefaults 的 f64/其余键扩展。

---

## 修复进度（2026-09-27 第二十二轮：工作区批次 Wave 1 + 快赢收尾）

两个子代理并行（core 工作区增补 / 浮动面板基础设施），主线完成 JPEG 预览等快赢。测试 761 → **781** 全绿（core lib 586 + 集成 104 + app lib 91）。设计蓝图：`docs/workspace-api-design.md`。

**§2 FloatingPanel 整文件落地（panels.rs）**
- 面板注册表：叠放序（re-show 置顶+改标题）、窗口坐标位置记忆（**Docked 恒不记忆**，:99-104）、首次 Automatic 开在画布中心且**面板中心对齐画布中心**（:110-115，设计稿的 window_size 参数经原文核对改为 panel_size）、440 宽右停靠逐帧重算（:127-136）、拖动 plumbing（Global 拖动状态 + 窗口级事件 + 双重窗口钳制）。渲染壳暴露内容槽，sheet 内容可直接搬入。10 个测试（FloatingPanelTests 状态侧）。

**§2 ProjectWorkspace core 增补（§7）**
- `session.project_url` + `install_project`（document_from_project 再校验后整体替换、清编辑会话、**history.reset()**——设计稿误写 mark_saved，按 Swift EditorSession+Projects.swift:38 修正）+ `clear_project`。
- `project::copy_layers_between`（ProjectWorkspace.swift:157-209 逐行）：根子树拷贝、预算=图层 asset 像素 ≤100MP（弯撇号文案按原文）、出集合的活蒙版烘焙/调整层摘链、UUID 全量重映射（parent/maskSource 出集合置 None）、锚点平移含蒙版 placement、空目标按源尺寸建文档、一条 "Copy Layers from Project" undo。7 个测试。

**§9.1-2 JPEG 导出对齐 JPEGExportSheet.swift**
- 实时编码预览：打开/质量/哑光变更即重编码（选项未变跳过），预览图适配显示 + ByteCountFormatter 风格字节数行 + "Updating preview…"，OK 在预览落后于选项时禁用（readyOptions 语义），Export 写预览自身的字节（finish(result?.data)）。
- 质量持久化：ToolDefaults 新增数值段（serde 向后兼容），`jpegExportQuality` 读写对齐 Swift UserDefaults。
- Return（canvas.apply）现也提交挂起的变换（TransformInspector Apply）。

**仍缺（本批次后续 Wave）**：~~tabs.rs/project.rs 模型层~~ ✅ Wave 2 → ~~AppState 重组 + TabBar + 菜单改投~~ ✅ Wave 3 → ~~面板迁移~~ ✅ Wave 4（全部 8 面板入 FloatingPanel 壳：Levels/ HueSaturation/ SelectionAmount/ Filter+Camera Raw Docked/ Effects/ ColorPicker/ Shortcuts；7 个 SheetKind 变体删除，pending 对话框保持模态对齐 Swift）。**批次收尾后的余项**：InlineTextEditor；TransformInspector 把手实时跟随；快捷键改键即时生效；面板内控件与 Swift 的微小形态差异（Effects Stroke 分段控件、SelectionAmount 旁滑杆）。

**审计修复（2026-09-28，`docs/reports/2026-09-28/code-implementation-audit/`）**：11 高 + 21 中严重度项已修（undo 门控/渐变错层/删除活蒙版 bake/笔刷选区文档坐标/1×1 蒙版 panic/PSD 越界等，9 个新回归测试），5 项经 Swift 原文核实为误报不改，未修项留痕（Vision 依赖、app 接线、性能重构、需算法设计）。
- **中低项与测试质量批次**（2026-09-28）：warp 增加四边形像素 coverage（Sutherland–Hodgman 解析面积，distort trim 边界与 Core Image AA 精确一致，tool_flow ±1 容差改精确断言）；change_text_style 非法回滚、hue_response 固定序求和、unpremultiply 舍入等低项；tests/ 目录对齐 Swift——clear_pixels 边缘断言部分擦除、pixel-move 三用例移植（cmd-drag 单 undo/duplicate/cmd-arrow 蒙版拒绝）、恒真断言与 let 残骸改真断言；另 5 项核实为误报（colorize 负色相/混色器归一化/Rec.601/blend_gray 舍入均与 C 内核逐字一致）。未修项维持记录（Vision/性能/需镜头档案类）。
- **InlineTextEditor + 检查器跟随**（2026-09-28）：画布位置锚定的多行 Textarea 浮层与 textDraft 双向绑定（Return 换行/Ctrl-Return 提交/Esc 取消/Option 方向键调字距行距）；Type 手势三态（提交旧稿/编辑命中层/拖出点框文本，默认样例文本点击删除）；TransformInspector 六字段随拖拽预览实时同步。6 项 v1 偏差留痕（自绘光标/缩放把手/旋转跟随等）。
**app 接线批次（2026-09-28）**：审计未修清单的 app 侧项全部落地——Blur 蒙版目标采样蒙版覆盖层（Swift BlurTool.swift:8-40 原义，非"蒙版后内容"）、Add Noise 种子改为**面板打开时一次生成**（Swift "fixed while the panel is open"，预览与提交共用，硬编码 7 清除）、Clone Stamp aligned 走 `clone_stroke_offset`、删除活蒙版依赖弹 Swift 三键询问（Bake and Delete / Cancel / Remove Links and Delete，文案逐字，经 Confirm sheet 通道 + `request_delete_layer` 接线）；marquee from_center 经核实为审计误报（Swift 恒传 false，Option-center 是 Shape 专属且已接线）。**快捷键改键即时生效**：keymap 增量重建（快照拆分 compositor:: 前缀/组件基线，clear 后重放——绕开 init 幂等性），Done/Restore/Cancel 三路重绑，过时"需重启"文案清除。**面板形态**：Effects Stroke 改 Swift 的分段双按钮（标题行右侧），SelectionAmount 补 1…max 双向同步滑杆 + 无效提示行（Swift LassoControls.swift:181-249）。新增 16 测试。

- **平台能力缺口（明确结论，不做替代品）**：`select_subject`（Select Subject）与 `subject_removal::run`（Remove Background 底层，现为 stub）都依赖 Vision 前景分割——Swift 无点击点、全画布自动找前景，无合理 1:1 替代源（点击式 Object Selection 是不同功能且已移植）。Filter 菜单的 Remove Background 入口在底层就绪前保持现状；报告记录为平台差异。

---

## 修复进度（2026-09-26 第十九轮：AdjustmentEditing + 面板系统 + 快捷键自定义 + 选区混合修复）

三个子代理并行（AdjustmentEditing+pixel 修复 / Curves+面板 / 快捷键自定义），主线完成接线。测试 595 → 659 全绿（core lib 563 + 集成 28 + app lib 52 + bin 16）。

**§2 AdjustmentEditing 整文件落地**
- session：`begin_adjustment_editing`/`edited_adjustment`/`preview_adjustment_editing`/`finish_adjustment_editing`（OK 一条 undo、Cancel 精确还原、编辑绑定打开时的图层、Invert 无设置）；渲染经 draw_live_composite 直接消费，无渲染端改动。
- app 接线：Layer 菜单 "Edit Adjustment…"——按 kind 打开 Levels/Hue-Saturation/Filter sheet 并从图层设置预填编辑器；滑杆改动写 session 编辑器并经 `preview_adjustment_editing` 实时预览；OK 提交、Cancel 还原（confirm/cancel sheet 分支）。

**§7.1-4 选区 coverage 双缺陷修复（上轮遗留）**
- 方向修正：像素中心经 mapping **正向**映射 + 面积语义采样（IDENTITY 下与 rasterize 逐位一致，50% 边缘伪影消失）；新增 `coverage_path`/`blend_path` 把选区路径经逆变换**重光栅化**（亚像素精确，feather 按逆映射比例重加）；sheets.rs 双重求逆规避移除，既有测试收紧到精确值。

**§9.3 面板/控件批次**
- **Curves 曲线编辑器**（滤镜 sheet Curves 分支，CurvesControls.swift 1:1）：通道下拉、240px 曲线画布（5×5 网格 + 单调 Hermite 采样白线 + 8px 控制点）、点击加点（32 点上限、x∈1…254、距点 ≥1）、近点抓取（<14）、拖动（y 逐字、内部点 x 钳邻点）、窗口级跟踪、Input/Output 读数、Remove/Reset；预览与 OK 走滤镜管线。
- **TypeControls**（Type 工具头）：字体下拉（扫 core 字体目录）、字号（钳 1…2000）、色块（text 取色器）、对齐、Tracking、Leading（Auto 占位）；实时写 `change_text_style`。
- **TransformInspector**（Move 工具头）：X/Y/W/H/Scale/° 字段（draft → 层 → 未链接 mask 的显示值，W/H 锁比、Scale 缩放、旋转 %360）、Auto-select、Show Controls、Sampling 下拉（拐角扭曲禁用）、Flip H/V、Cancel/Apply。

**§9.3 快捷键自定义系统（KeyboardShortcuts.swift:129-319 整系统）**
- 新模块 shortcuts.rs：84 动作目录（稳定 id + Swift 对齐名 + 分组 + 默认键位）、ShortcutRegistry（覆盖/重置/生效表）、冲突与保留键校验（Swift problem(in:) 语义 + Display 文案）、JSON 持久化（平台配置目录、原子写、损坏/冲突全有或全无回退）；`bindings()` 改由注册表生成（菜单提示自动跟随）。19 个测试。
- 顺带修复默认表真冲突：Contract Sheet 与 Canvas Size 同为 ctrl-alt-c（gpui 修饰键序无关）→ Contract 改 `ctrl-alt-shift-c`。

**主线批次**
- §5.3 蒙版绘画打通：蒙版选中时笔画以蒙版网格为基准、蒙版图为源、`mask_paint_white` 定白/黑，提交替换蒙版 asset 保留 placement/link（canPaint 守卫：修复画笔/仿制图章禁于蒙版、调整层禁画、空选区禁画）。
- §5.3 笔刷平滑"绳拽模型"落地（此前滑杆不消费）：半径=平滑值/zoom，绳内抖动不进入笔画、绳绷直时跟随、flush 结束于指针位置。
- 图层面板缩略图 **Cmd 点击载入选区**（Shift 加选/Option 减选）。
- Move 工具 `transformPressLayer` 拾取：auto-select 选中指针下层级更高的图层（含背景上的前景层偏好）、Cmd 直接拾取、空画布拖活动层；Auto-select 勾选框接通。
- §3.3 `mask_clip_image` 的 MaskPlacementCache（8 项/64MP LRU，Arc 身份键）+ interactive limit 改 `.rounded(.up)`。
- §9.3 标尺刻度候选集对齐 CanvasRulers.majorStep（1-2-5 全表、70pt 目标、50k 上限）+ 整数标签函数。

**仍缺（后续批次）**：标尺拖拽建参考线（需 CanvasDrag 扩展）、工具默认值持久化（ToolDefaults）、shortcut 设置 UI 编辑器（注册表 API 已就绪）、TypeControls 字段 ↑↓ 步进与 SF Symbol 图标、TransformInspector 的 Esc/Return 与把手实时跟随、Transform Auto-select 持久化、select_subject 的 Vision 替代源、浮动面板系统、多窗口/标签条、RAW 导入、JPEG DPI 元数据、像素移动（PixelMove）会话入口。

---

## 修复进度（2026-09-26 第十八轮，四个并行方向 + 主线集成）

本轮四个子代理并行（levels_auto / 会话蒙版批 / 选区与新工具 / 测试移植），主线完成集成接线。测试 core 490 → 551（lib）+ 28（新建 `crates/core/tests/` 集成测试）+ app 10 → 16，全局 595 全绿。

**C 域（§7.1-1 LevelsAutomatic 整文件）**
- core 新模块 `adjust/levels_auto.rs`：三模式 Contrast/Color/Neutral、单通道 256 bin、clip 0.001（严格大于）、Neutral gamma=min(9.99, max(0.1, ln(mean)/ln(0.5)))（mean 按 gamma=1 区间对 apply 值加权），浮点运算顺序逐行对齐；12 个测试含 Swift `autoAlgorithmsAndEyedropperCalibration` 数值复刻。
- app `levels_auto` 旧算法（1024 混合 bin 分位）删除，改走 core 三模式；Levels sheet Auto 行三个按钮（Contrast/Color/Neutral）接 `levels_auto_mode`（整体替换设置→同步滑杆→与手动滑杆同路径刷新）。
- §7.1-8 Levels/HueSaturation 提交与预览经选区混合（`pixel::blend` 回混，预览=提交结果）；§7.2 Levels 直方图以选区 coverage 加权。
- §9.3 滤镜滑杆默认值对齐 Filters.swift:39-45（Gaussian 4→1、Motion 20→10、AddNoise 12→10）。

**A 域（§3.1 未修项批量）**
- §3.1-5 undo/redo 恢复语义：`restore(snapshot)`——先 cancel_crop+cancel_gradient；`keep_mask_target = is_mask_selected && 活动层未变`，恢复后按其与蒙版存在性重算 `is_mask_selected`（EditorSession.swift:578-586）。app 在 document 变化时重适配视口。
- §3.1-8 merge 守卫补全：`merge_plan` 逐条（canEditLayers、多选含非组层、组内含像素层、Merge Down 取正下方同父层且组则拒、anchor/parent/name 按 Swift 分支、层级校验失败静默回退）；新增 `can_merge_layers`（§3.1-8/11 相关守卫）。
- §3.1-14 `LayerAdjustment.is_valid` 补 HSV 细分（hue ±360、sat/light ±100、bands handle 有限）。
- §3.1-18 `begin_transform_persistent(bool)`；画布拖拽传 false（EditorCanvas.swift:2129），nudge 传 false。
- 蒙版批：`select_layer_target`、`can_copy_mask`/`copy_mask`（"Copy/Replace Layer Mask"）、`toggle_mask_link`（"Unlink/Link Layer Mask"）、`displayed_mask_placement`（组变换/单层/蒙版草稿/distort 分支）；`set_preview_blend_mode`（LayerAppearance.swift:78，commit_transform 时清除）。

**D/E 域（§8.1-4 + §2 两个整文件）**
- §8.1-4 环→多边形配洞重写为**包含森林**（父级=面积最小且严格包含者，偶深度外环/奇深度洞），三层嵌套不再语义反转；5 个新测试含旧算法必败回归。
- §2 MaskTracing 整文件落地 `tools/mask_tracing.rs`：dark/white/opaque pixels + trace（有向单位边字典、只留拐点、外顺内逆）逐行移植；session 入口 `load_layer_selection`/`load_mask_selection`（Cmd 点击缩略图/Select 菜单，"Load Layer/Mask Selection"）+ 3 个测试。
- §2 ObjectSelection 整文件落地 `tools/object_selection.rs`：Vision 前景蒙版做成参数输入，其后数学全移植——联合双边上采样（空间 σ=5、luma σ=0.15）、腐蚀/膨胀 `min(10,|edgeOffset|)` 次、wand 描迹、RDP 简化容差 1.6、Chaikin 3 轮 0.25/0.75；7 个测试。

**E 域（§5.2-2 + 连续引擎语义对齐）**
- 笔画 provisional tail 改为**解析渲染**：settled 累积 Catmull–Rom 自适应细分段（弦差 ≤0.2px、深度 10，`continuousCurve` 逐行移植），tail=前一采样点→光标直线段，独立于永久 coverage 渲染——下一采样精确替换（drawTail/removeTail 的整网格等价）；tail 不再盖章进 `has_paint` 网格；4 个新测试（湿预览含尾、flush 换尾、样条细分 ≤0.2px、直线塌缩）。

**G+H 域（§9.1/§9.3 菜单与缩略图批次）**
- 菜单动态标题对齐 CompositorApp.swift：Invert/Invert Mask、Transform Selection/Layer（free transform 路由 can_transform_selection）、Duplicate Layer/Layer via Copy（`layer_via_copy` 内部路由）、Create/Release Clipping Mask、Show/Hide Layer、merge_title()、Delete Layer Mask/Layers/Layer（`deleteLayerOrMask` 路由）。
- 菜单新条目：Edit 的 Clear Selection Pixels；Image 的 Curves/Black & White/Color Balance/Exposure/Gradient Map/Grain（走滤镜 sheet）；Filter 的 Remove Background（立即执行一条 undo）；Select 的 Layer's Pixels/Mask's Black Areas；Layer 的 New Adjustment Layer 子菜单（12 种 `add_adjustment`）、Group Selected Layers（⌘G 重绑）、Move Out of Folder、Rename Layer…（接面板重命名）、New Blank Layer。
- §9.3 图层面板缩略图重写（CanvasThumbnail.swift 移植）：**整画布形状**（宽高比适配 30pt 槽、2x backing）+ 深灰棋盘格（12px tile），蒙版选中时显示蒙版缩略图且**边缘灰度延续到画布边**（`mask_edge_tone`）；缓存键含蒙版选中态。
- undo/redo 后 document 变化重适配视口。

**测试移植（§10，新建 crates/core/tests/ 集成测试）**
- DownsampleTests 4/4、InnerGlow+OuterGlow 16/18、Export+JPEGExport 5、TypeTool 纯函数 3 + is_valid 半边，共 28 用例；缺口清单见 tests 内注释（会话流 TypeTool 用例、Metal parity、原子写 app 层）。

**已知遗留（本轮记录，未修）**
- `adjust/pixel.rs` `coverage` 的映射方向实现与命名相反（app 侧用双重求逆规避，探针验证几何正确）+ 采样半像素相位（选区边缘内侧 ~50% 混合）——§7.1-4 的残留，待专项修复。
- AdjustmentEditing 会话（begin/preview/finish/cancel）、select_subject（需 Vision 替代源决策）、滤镜 Gaussian σ 截断（4σ vs CI ≈3σ）、guide 拖拽建线调用点、重命名的 Show-Hide 动态标题 effects 分支——属后续批次。

---

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
- §8.1-2 winding 填充规则落地：`contains` 与 `rasterize` 的 `point_in_path` 改为 nonzero winding——射线穿越按边方向 +1/−1 累加，非零即内（`fillPath(using: .winding)` / `path.contains(using: .winding)`）；同向嵌套环仍为内部、反向环为洞，均有测试。
- §8.1-3 椭圆 4 段 Bézier 落地：`SelectionPath::ellipse` 改为 CG `addEllipse` 的四段 cubic Bézier（kappa=0.5522847498，每段 16 点折线化）；面积与解析椭圆差 <2%（有测试）。
- §8.1-5 expand/contract 描边带补齐：round join 圆弧密度从 π/8 提高到 π/16（最多 64 段）；**极小环的向内 offset 翻转/退化时（|signed_area(inward)| ≥ |signed_area(ring)|）整环按 band 覆盖**——contract 不再漏掉小环。
- §6.1-10 Motion Blur 锥形衰减落地：`motion_blur` 的均匀 box 权重改为 CIMotionBlur 的高斯锥形——每 tap 权重 exp(−t²/2σ²)（σ = radius，归一化）；单点亮斑的拖尾单调衰减且关于源对称（有测试）。
- §6.1-11 halving Lanczos 落地：`halve` 改为分离式 Lanczos3 重采样（`lanczos_resize`/`lanczos3`，clamp 边缘，输出 ceil(w/2)）；平场图像 DC 精确保持（有测试）。
- §6.1-9 补充（grow→blur→trim 完整流程）：GaussianBlur 分支改为 **grow**（四周 pad 透明 sigma×3）→ **blur**（clamp 版高斯，pad 保证 clamp 只触及透明区）→ **trim**（裁回原 extent）——贴边内容也能向外软化扩散；两个新测试。
- §9.2-3 PSD 縦量光栅化升级：`fill_polygons` 改为**解析 signed-area 扫描线积分**，斜边像素精确 128，总覆盖相对解析面积误差 0.039%（仅 u8 量化噪声；有测试）。
- §4.1-3 参考线可视化数据验证：`snapped_move` 落点写入 `snap_guides`（画布中轴线）、commit/cancel 清空；app 画布可直接消费。
- §5.2-7 蒙版分支采样落地：`sample_for_clone` 在层有启用蒙版时将采样内容乘以蒙版覆盖（画布显示的是蒙版后结果）。
- §8.1-6 选区光栅化升级：`rasterize` 从 4×4 均匀超采样升级为**解析面积分**（逐扫描行 winding 边、y-band 切分、闭式梯形积分），边缘像素精确覆盖（0.875 面积 → 223）。
- §8.1-2 winding 填充规则落地：`contains` 与光栅化的 `point_in_path` 改为 nonzero winding——射线穿越按边方向 +1/−1 累加，非零即内；同向嵌套环仍为内部、反向环为洞（有测试）。
- §8.1-3 椭圆 4 段 Bézier 落地：kappa 四段折线化，面积差 <2%（有测试）。
- §8.1-5 描边带补齐：round join 密度 π/16；极小环向内 offset 翻转时整环按 band 覆盖，contract 不再漏掉小环（有测试）。
- §6.1-10 Motion Blur 锥形衰减：高斯权重 σ=radius，归一化；拖尾单调衰减且对称（有测试）。
- §6.1-11 halving Lanczos 落地：分离式 Lanczos3 + ceil(w/2)；平场 DC 精确保持（有测试）。
- §9.2-1 PSD 图层名 MacRoman 解码（完整官方表，非 ASCII 不再变 U+FFFD；注 0x8E='é'/0xE9='È'）。
- §9.2-2 PSD 闭合路径收口 miter/bevel（限长 10:1，不再漏画收口）。
- §7.1-7 几何 warp 越界输出 premultiplied 透明黑（对齐 CIPerspectiveTransform）。
- §7.1-6 GuidedMatte 统一 resample(High) 单路径（删冗余 halve 循环）。
- §5.2-7/8 Blur/Clone 采样对齐：`sample_for_clone` 改为“活动层按 `displayed_transform` 绘制到文档尺寸画布”（`CloneStamp.swift:39-48`），`displayed_transform` 在 session 上新增（变换草稿优先）；Blur 分支采样同步改为画布显示内容（BlurTool.swift:8-40）。
- §5.2-12 形状工具补齐：`commit_transform` 对 shape 层在尺寸变化时**重画 raster**（redrawShape——缩放不再拉伸旧像素）；`toggle_shape_kind` 循环 Rectangle/Ellipse/Rounded 并接入 app 的 Tab 循环；app 形状拖拽补齐 Shift 直线 45° 八分吸附与 Option 中心放大。
- crates/app lib 化：新建 lib target（compositor_app），首批 **6 个单元测试**（坐标换算/往返、zoom clamp、标尺刻度、裁剪框命中、工具标题），crates/app 零测试状态结束。
- §9.2-3 PSD 縦量光栅化升级：`fill_polygons` 从 8 样本/像素提升为 **4×4=16 样本**（各 1/16 权重，winding 前缀扫描），斜边 AA 与解析面积差 <5%（有测试）。
- §3.1-6 canUndo 补齐：挂起渐变计入 can_undo。
- §3.3 add_blank_layer 插入点补齐：选中文件夹时插到其内容顶部。
- §6.1-9 Gaussian Blur 边缘语义：新增 `gaussian_blur_rgba_unclamped`（越界 tap 按透明处理，CI 语义），run_filter 改用。
- §3.1-1 历史字节估算落地：`retained_bytes` 重写——图层位图按 RGBA 4 B/px、**蒙版按 DeviceGray 1 B/px**（bytesPerRow 语义），image 与 thumbnail 分计，标识用 `Arc::as_ptr`（分配地址，替代 data 指针）跨快照去重。
- §3.1-2 快照相等语义落地：`ImportedImage`/`LayerMask`/`LayerText`/`ImageLayer` 手写 `PartialEq`——**图像按 `Arc::ptr_eq` 身份比较**（`===`），其余字段按值；Swift 的"相同字节的新图像仍记历史、克隆快照不记"两条语义均有测试。
- §4.2-3 原子保存强度落地：`write_package` 改为旧包先 rename 到同级 `.backup-*`、暂存包 rename 就位、成功后删 backup；rename 失败自动回滚旧包——任意时刻目标路径都存在完整包（不再有 remove 后的丢失窗口）。
- §5.2-1 heal 修复区域限制落地：`heal_region(painted, w, h)` 纯函数——painted 外扩 `(max(w,h)+32)*3.2` 并裁剪到画布取整；`heal()` 把底图与 coverage 裁到该区域再做膜求解，写回后湿预览熄灭。
- §5.2-4 连续引擎天然文档空间画圆：`render_continuous_tile` 以文档坐标对事件段积分（层像素轴可为任意旋转/缩放映射），`continuous_deposition_is_rotation_invariant` 验证 45° 层映射下沉积行为不变——旋转/非均匀缩放层上笔刷在画布上是圆。
- §5.2-5 Clone Stamp aligned 落地：session `clone_stroke_offset`/`begin_clone_stroke`（aligned 首笔固定 `cloneOffset`、后续复用；unaligned 每笔按笔→源整像素重算），app `begin_stroke` 改走该入口。
- §5.2-6 Gradient 挂起与蒙版渐变落地：session `GradientDraft` + `begin_gradient`（同目标重画重置端点）/`move_gradient`/`refresh_gradient`（端点重渲预览）/`apply_gradient`（undo "Gradient"/"Gradient Mask"，无线点击静默取消）/`cancel_gradient`/`resolve_gradient`；`select_layer` 切层时应用挂起渐变（Swift resolveGradient）；蒙版渐变走灰色调色板且统一 1×1 蒙版从整层白底起步。
- §5.2-13 warp finish 校验落地：`WarpStroke` 携带层快照（asset Arc + transform），`finish_warp` 的身份守卫——笔画期间层被并发修改则丢弃结果；`finish_warp_into_layer` 接收 `selection_clip`，扭曲结果沿选区剪裁。
- §5.2-10/11/14 笔刷三处行为对齐：`.copy` 混合改为 src×(cov×opacity)（目标不留痕迹，影响 finishWarp/Blur 软边）、`fill()` 不再吃笔刷不透明度（Swift 填充不设全局 alpha）、`move_lifted` 整像素移动走最近邻、分数移动走 High 插值。
- §4.2-1/2/5/6 ImageResizer 语义重写（`core/canvas.rs` `resize_image`）：逐层按角点缩放包围盒重光栅化（旋转/非均匀层落到轴对齐包围盒、sampling 选项生效）；统一 1×1 蒙版与已放置蒙版像素原样保留而 placement 随画布缩放（`placing(unitToDocument ∘ scale)` 公式）；同尺寸仅改 resolution 的早退（图层/参考线/选区均不动）；resolution 1…9600 与 100MP 逐层预算校验。
- §4.1-6/7 CanvasResizer 落地：`resize_canvas` 返回 Result——1…30000/anchor 0…8/offset 有限性/逐层 isValid 防御、同尺寸同偏移的接受性 no-op（选区不再被清）、`fill` 有色扩展作为底部 "Canvas Extension" 层（旧画布区域保持透明含空洞，100MP+万层+颜色校验）。
- §4.1-4/5 翻转补齐：`flip_canvas` 镜像选区（仿射 ±1 平移画布宽/高）；`flip_layers` 支持多选（共享包围盒中轴，一条 undo）。
- §8.1-10 选区守卫补齐：`begin_lasso` 需要可编辑文档且无进行中的选区移动、`drag_marquee`/`extend_lasso` 拒绝非有限点、`can_modify_selection` 加 `can_edit_layers` 且 lasso 草稿打开时为假、`nudge_selection` 取整整像素。
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

**第三轮批次（§9.3 快捷键批量补齐 + §6.1-12 采样降档 + §7.1-2 CIColorCube）**

- §9.3 快捷键批量补齐（KeyboardShortcuts.swift:74-124 对齐）：新动作 + handler + 绑定——剪贴板四件套 Cut/⌘X、Copy/⌘C、Copy Merged/⇧⌘C、Paste/⌘V；Invert Pixels/⌘I；Toggle Clipping Mask/⌥⌘G；Move Layer Up/Down（⌘]/⌘[）；Show Transform Controls/⌘H（session `shows_transform_controls` 字段 + 画布 TransformOverlay 门控：`shows || edit.persistent`，LayerRenderer.swift:65 语义）；笔刷硬度 ⇧[/⇧]（core `change_brush_hardness` 25% 四分步进、80%→上 100%/下 75%，EditorSession+Brush.swift:212-218，滑杆同步）；混合模式步进 ⇧−/⇧=；循环形状 ⇧U；Fill 三键（⌥⌫/⌘⌫/⇧⌫，delete+backspace 双写）；Save As/⇧⌘S；Close/⌘W；Snap 总开关/⇧⌘;；Lock Guides/⌥⌘;；Levels 预览/⌥P。**键位对齐**：Export PNG `ctrl-shift-s`→`ctrl-shift-e`（⇧⌘E）、Export JPEG `alt-shift-s`→`ctrl-alt-shift-s`（⌥⇧⌘S）、Show Guides→`ctrl-;`（⌘;）。
- §9.3 菜单补齐：Edit 菜单 Cut/Copy/Copy Merged/Paste/Content-Aware Fill；Image 菜单 Invert；Layer 菜单 Create Clipping Mask/Move Layer Up/Down；View 菜单 Show Transform Controls/Snap/Lock Guides 勾选项（CompositorApp.swift:47-318）。
- §9.3 Lock Guides core 守卫：`hit_guide`/`begin_guide_creation`/`begin_guide_move` 在 `locks_guides` 时拒绝（Guides.swift:74/92），此前字段存在但从不被消费；有测试。
- §9.3 Levels 预览开关落地：sheet 加 Preview 勾选框（Swift `LevelsEdit.preview`/LevelsSheet Toggle 语义——关时预览显示原始像素），⌥P 经 `toggle_levels_preview` 翻转并即时重算预览。
- §6.1-12 预缩小档位修正：新 `downsample_level`（DownsampleCache.level 逐行移植——`factor ≥ 0.5` 不缩、`floor(log2(1/factor))`、maxLevel 6、非有限/非正为 0）；`resample` 的 High 分支从"next_w < w 即 break"改走该档位（恰 0.5 不再多缩一次、深缩小次数与 Swift 一致、超 maxLevel 交给终采）；`draw_affine` 同步改走档位函数（语义等价重写）；**`LayerRenderer.interpolation` 语义落地**——缩小终采一律双线性（Low），放大且 High 走 Lanczos 终采（`resample` 返回 `lanczos_resize`、`draw_affine` 用新 `lanczos_sample` 连续点采样）；4 个新测试（档位表、恰半边界、放大锐利终采、仿射路径）。
- §7.1-2 HueSaturation 渲染路径：`hue_saturation_apply` 改走 **33³ CIColorCube**（`ColorCube::build` 逐行移植 HueSaturationFilter.cube——蓝外/绿中/红内循环、alpha=1、per-hue hue_response 预计算、格点用原 f64 `adjust_hsl` 填充）+ **f32 三线性插值**采样（color×32 格坐标、8 角混合）+ 非预乘→查找→预乘（HueSaturation.swift:238-239 alpha 语义）；`previewLimit=8000` 属对话框预览机制（core 无对应实体），留待 §9.3 面板批次；7 个新测试（格点恒等、三线性混合、与精确逐像素 f64 数学 ≤3/255 一致、色域调整、colorize、恒等短路、路由验证）。
- app 测试扩容（canvas 坐标/状态机）：形状拖拽矩形计算抽为纯函数 `shape_drag_rect`（Shift 45° 八分吸附/Option 中心放大/线段忽略修饰键）并补 3 个测试；`handle_at` 八手柄顺时针全遍历；坐标往返极值（MIN_ZOOM…MAX_ZOOM）1 项。

**测试**：核心 452 → 465（+13：硬度步进 1、采样档位/放大终采/仿射锐化 4、锁定参考线 1、CIColorCube 7）；app 6 → 10（+4：形状拖拽 2、手柄遍历 1、极值往返 1）。全局 475 全绿，工作区零错误。

**第四轮批次（§9.3 面板系统首批 + canvas 键处理层）**

- §9.3 EffectsSheet 落地（子代理）：core 新模块 `effects_session.rs`（`impl EditorSession`：`can_edit_effects`/`active_effects`/`editing_effects`/`selected_effect`/`add_effect`/`select_effect(kind,on,editing)`/`finish_effects_editing(commit)`/`set_effects(effects,on,name)`/`change_effects`/`can_copy_effect`/`copy_effect`，LayerEffects.swift:221-348 逐行——undo 名 "Add|Edit|Cancel|Copy <Kind>" 与默认 "Layer Effects"、新增 stroke/色叠印取背景色其余取默认、`isValid` 守卫、空效果归一为 None、`select_effect` 强制单选并退出蒙版、Cancel 只还原所编辑种类（新增效果 Cancel 即移除）、编辑绑定 `effects_editing.layer_id` 图层不受选区切换影响）；app 侧 SheetKind::Effects sheet（六种效果各自的滑杆组：Stroke 位置/色/尺寸 0-20px/不透明度、投影与内阴影 不透明度/角度/距离/模糊、色叠印不透明度、外发光与内发光 尺寸/不透明度）+ 图层面板脚部 "Fx" 菜单入口（`can_edit_effects` 门控）；OK/Cancel 接 `finish_effects_editing`，滑杆改动经 `change_effects` 实时上画（Swift 同语义每步一条 undo）。7 个测试。
- §9.3 ColorPickerSheet 首批落地（子代理 core + 主线 UI）：core 新模块 `color_picker.rs`——`can_edit_palette`/`set_palette_color`（蒙版选中驱动 `mask_paint_white`、前景色跟随打开的文本草稿）/`open_color_picker(background)`/`open_text_color_picker`/`open_effect_color_picker`/`close_color_picker(commit)`（palette/text/effect 三分支；`.gradientMap` 因 core 无 filter 编辑会话留 no-op）/`preview_effect_color`（效果颜色实时预览到图层，私有 `change_effect_color_for_preview` 镜像 changeEffects 语义）/`type_color`；session 预置 `ColorPickerTarget`（Palette/Effect/GradientMap/Text）与 `ColorPickerState`（hsb 为真值，色相穿灰黑不丢）字段。app 侧新模块 `color_picker_ui.rs`：HSB 滑杆 + 色样预览 + Hex 输入（OK 时读取）+ OK/Cancel 的取色器面板；**色板点击从"交换"改为打开取色器**（前/背景各自命中区；X 键仍交换）；Esc 取消关闭取色器。16 个测试。
- canvas 键处理层（主线）：core `type_opacity_digit(digit, uptime)` + `pending_opacity_digit`——Photoshop 式不透明度数字键（1=10%…0=100%，0.6 秒内两键成精确值如 4→5=45%、0→5=5%，最小 1%；按工具路由：笔刷族→`brush_settings.opacity`、渐变→`gradient_settings.opacity`、Move→新 `set_selected_layers_opacity` 多选一条 "Layer Opacity" undo，LayerAppearance.swift:108-119）；core `set_selected_layers_opacity`（多选含文件夹）。app 侧 `install_key_events`（gpui `window.on_key_event` 挂 Bubble 相，与 install_drag_events 同帧注册）：Space 按住临时切 Hand、keyUp 恢复（`space_saved_tool`）；数字键不透明度（并同步对应面板滑杆——笔刷/渐变/图层不透明度）；⌥方向字距/行距（⇧=×10，行距从 `line_height()` 起算、下限 1，InlineTextEditor.swift:12-22）；⌘⏎ 完成 TextDraft（Windows 映射为 Ctrl）。Escape 取消链修复：`cancel_active` 的 sheet 分支改走 `cancel_sheet`（Effects sheet 的还原语义不再被绕过），并新增取色器分支。2 个 core 测试。
- 集成注记：Effects sheet 的 OK/Cancel 由子代理接入 `cancel_sheet`/`confirm_sheet`，主线随后把 `cancel_active`（Esc 动作路径）接到同一入口，两条取消路径语义一致。

**测试**：核心 465 → 490（+25：不透明度数字键 2、Effects 编辑会话 7、取色器会话 16）；app 10 不变。全局 500 全绿，工作区零错误。

**测试**：核心 321 → 452（+131，另有 app 包 6 个单元测试，全局 458）（+131：历史灰度字节估算与恒等/克隆条目 3 项、图像身份相等 1 项、蒙版/文本/图层身份相等 3 项、原子保存无残留（经既有 store 测试）、参考线交互 3 项与可视化数据 1 项、蒙版采样 1 项、grow→blur→trim 2 项、解析 AA 积分 1 项，其余同前；另加 winding 双环 1 项、椭圆面积 1 项、小环 band 1 项、锥形拖尾 1 项、halve 取整/平场 1 项/参考线交互 3 项/显示变换与形状重画 3 项/PSD 光栅 AA 1 项/app 单元 6 项/解析光栅化与羽化 2 项/MacRoman 与收口角 2 项/透视透明 1 项）。混合公式、heal、渲染语义 6 项、蒙版/删除/选区编辑/调整图层/剪贴板/复制文件夹/浮动变换 4 项/导出 5 项/多选批量与折叠 5 项/重排/眼睛滑动/批量删除/导入 3 项/DistortWarp 6 项/活蒙版剪贴与释放 3 项/烘焙 1 项/Distort 会话 2 项/真字体排版 6 项/文本会话 5 项/连续沉积 3 项/裁剪吸附 4 项/吸管色相与 defringe 4 项/变换吸附 2 项/笔刷 copy·fill·move 3 项/resize 语义与防御 5 项/翻转与选区镜像 2 项/选区守卫 1 项/heal 区域与旋转不变性 2 项/clone aligned 2 项/渐变挂起与蒙版 3 项）。

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
| 1 | 历史字节估算：Swift 按位图实际 bytesPerRow（灰度蒙版 1 B/px）；Rust 一律 `w*h*4`，蒙版多的文档更早触发 256MB 上限裁剪 | `Document/DocumentHistory.swift:103` | ✅ 已补（蒙版 1 B/px、image+thumbnail 分计、Arc 身份去重） |
| 2 | 快照相等语义：Swift 对图像用 `===` 身份比较；Rust derive 值相等（含像素数据），"内容相同的不同图像对象"场景下"文档未变不记历史"判定相反 | `Document/EditorSession.swift:4-7` | ✅ 已补（ImportedImage/LayerMask/LayerText/ImageLayer 手写 PartialEq：图像 Arc 身份、其余按值） |
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
| 4 | `flip_canvas` 不镜像选区（Swift 用仿射镜像 `selection.path`） | `Document/LayerFlip.swift:69-77` | ✅ 已补（选区随画布仿射镜像） |
| 5 | `flip_layers` 不支持多选/组翻转 | `LayerFlip.swift:24-51` | ✅ 已补（多选关于共享包围盒中轴翻转） |
| 6 | Canvas Size 的 fill（"Canvas Extension" 有色扩展层）+ 像素/图层数预算 + 颜色校验 | `IO/CanvasResizer.swift:8-15,23,36-69` | ✅ 已补（`resize_canvas` 的 fill 扩展层与防御校验；app 侧 fill 传参待 UI） |
| 7 | CanvasResizer 防御校验（1…30000、同尺寸早退、逐层 transform.isValid） | `IO/CanvasResizer.swift:8-23` | ✅ 已补（防御校验 + 同尺寸 no-op 保选区） |
| 8 | 参考线拖拽/标尺交互全套 | `Guides.swift:91-179` | ✅ 已补（`hit_guide`/`begin_guide_creation`/`begin_guide_move`/`move_guide_drag`/`finish_guide_drag`/`cancel_guide_drag`/`snapped_guide_position`，undo 名 New Guide/Move Guide/Delete Guide） |

### 4.2 行为差异

| # | 差异 | Swift 位置 | Rust 位置 |
| --- | --- | --- | --- |
| 1 | ImageResizer 旋转层：Swift 取 4 角投影后缩放的包围盒，输出 transform 的 rotation/flip **归零**（ImageSizeTests.swift:56 断言）；Rust 线性缩放 origin/size 并保留 rotation/flip → 旋转/非均匀缩放层尺寸与位置不一致 | `IO/ImageResizer.swift:36-43` | ✅ 已重写（`resize_image` 逐层角点包围盒重光栅化，见顶部修复进度） |
| 2 | 1×1 均匀 mask：Swift 原样保留（分辨率无关，`ImageResizer.swift:77`）；Rust 无条件重采样放大。mask placement 缩放公式同样不同（Swift 用 `placing(unitToDocument ∘ scale)`，`ImageResizer.swift:97`；Rust 线性缩放，`canvas.rs:198-201`） | 同上 | ✅ 已重写（统一/已放置蒙版像素保留、placement 用 placing∘scale 公式） |
| 3 | 原子保存强度：Swift `NSFileCoordinator(.forReplacing)` + FileWrapper atomic（旧包在交换前完好）；Rust 先 `remove_dir_all` 旧包再 rename 暂存包，两步间崩溃丢包 | `IO/ProjectStore.swift:111-118` | ✅ 已补（旧包先 rename 到 `.backup-*`，暂存就位后删除；rename 失败回滚旧包） |
| 4 | PNG 严格性：Swift 拒多帧 APNG 且逐属性查 depth≤8（`ProjectStore.swift:155-161`）；Rust 只拒 16-bit 色型，多帧按首帧接受 | 同上 | ✅ APNG 已拒（`read_png_asset` 的 `is_apng` 检查；depth≤8 由 16-bit 色型拒绝覆盖） |
| 5 | `resize_image_to` 缺 resolution（1…9600）与 100MP 总像素预算校验；Swift 同尺寸时只改 resolution 早退 | `IO/ImageResizer.swift:15-25` | ✅ 已补（resolution 校验、同尺寸早退、逐层预算） |
| 6 | Image Size 无 sampling 选项（Swift `ImageSizeOptions.sampling`，测试用 `.nearest`）；Rust 硬编码 `LayerSampling::High` | `IO/ImageResizer.swift:4-9` | ✅ 已补（`resize_image_to` 带 sampling 参数） |
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
| 1 | heal 修复区域：Swift 限制在 `painted.insetBy(-reach)`（`reach=(max(w,h)+32)*3.2`）内搜索 patch；Rust 全图层网格搜索 → 大图层上选中的源 patch 可不同 | `Document/BrushStroke.swift:776-784` | ✅ 已补（`heal_region` 裁剪工作区，见顶部修复进度） |
| 2 | 笔画 provisional tail 的 coverage 撤销：Swift `drawTail` 保存 tail 可达 tile 的 coverage 图，`removeTail` 精确恢复；Rust 只恢复间距状态，tail 已画的 dab 保留（软笔刷下每次 pointer 事件留直线 tail 多余覆盖） | `BrushStroke.swift:354-387` | `core/src/tools/brush.rs:284-293` |
| 3 | GPU 连续沉积模型缺失：Swift 默认 `useGPU=true` 走 `appendContinuous`（coverage=1−exp(−∫tipDensity)，0.2px 自适应细分）；Rust 是 CPU dab 路径 | `Rendering/MetalBrushCoverage.swift:70-161`、`BrushStroke.swift:149-150,224,281-309` | `core/src/tools/brush.rs`（无 continuous 实现） |
| 4 | 旋转/非均匀缩放图层上的笔刷形状**语义相反**：Swift CPU dab 在文档空间画圆（保证图层上仍是圆）；Rust 只映射圆心、tip 在图层像素空间盖章 → 旋转图层上画出文档空间椭圆 | `BrushStroke.swift:546-547,559-560`（测试 BrushTests.swift:197） | ✅ 连续沉积引擎按文档空间积分（旋转不变性有测试）；dab 盖章路径仅为 has_paint 内部状态 |
| 5 | Clone Stamp aligned 语义缺失：Swift aligned 时复用首笔 `cloneOffset`（取整）；Rust 每次起笔重算（恒 unaligned），`session.rs:148` `clone_offset` 从未写入，`CloneSettings.aligned` 是死配置 | `Document/CloneStamp.swift:23-27`、`EditorSession+Brush.swift:39` | ✅ 已补（`clone_stroke_offset`/`begin_clone_stroke`，app 起笔接入） |
| 6 | Gradient 预览/提交：Swift `GradientEdit` 挂起可重画、切工具自动提交、支持蒙版渐变；Rust 拖完即提交、`mask:false`（蒙版渐变不可用） | `Document/Gradient.swift:36-108` | ✅ 已补（session `GradientDraft` 挂起/重画/应用 + 蒙版灰渐变；切层 resolve 已接入） |
| 7 | Blur 采样对象：Swift 画的是 displayedTransform 后的画布显示（含蒙版分支）；Rust 直接对 `layer.asset` 原图做高斯，无变换/蒙版 | `Document/BlurTool.swift:8-40` | `app/src/canvas.rs:475-482` |
| 8 | Clone/Blur 采样源：Swift 经 displayedTransform 画到文档尺寸（含 sampleAllLayers 合成）；Rust 单图层时直取原始 asset（变换≠单位阵时错位） | `Document/CloneStamp.swift:39-48` | `app/src/canvas.rs:488-496` |
| 9 | heal 随机种子：Swift 每次 `UInt32.random`；Rust 固定 `0x6d2b79f5` | `BrushStroke.swift:793` | `app/src/canvas.rs:510` |
| 10 | `replaces_with_clone` 的 copy 混合：Swift `.copy` blend 结果 = src×(cov×opacity) 不保留 dst；Rust `out*(1-a)+s*a` 保留 dst（半覆盖像素不同，影响 finishWarp/Blur 软边） | `BrushStroke.swift:464` | `core/src/tools/brush.rs:461-468` |
| 11 | `fill()` 是否吃 opacity：Swift 不吃（直接 fill）；Rust 乘 `settings.opacity`（改过笔刷不透明度后填充变淡） | `BrushStroke.swift:626-631` | `core/src/tools/brush.rs:607-638` |
| 12 | 形状工具：Swift `finishShape` 后 uncommitted 时实时重渲（缩放=重绘）；Rust `add_shape_layer` 立即写 asset → 形状缩放是重采样拉伸非重绘；缺 Option 中心放大、直线 45° 吸附、`toggleShapeKind`/`redrawShape` | `Document/ShapeTool.swift:84-100,161-194` | `core/src/session.rs:517-556`、`app/src/canvas.rs:316-322` |
| 13 | warp finish：Swift 校验图层未被并发修改且经选区剪裁；Rust 无守卫、`selection_clip: None`（结果可越界涂写） | `Document/SmudgeLiquify.swift:192-201` | ✅ 已补（`WarpStroke` 层快照身份守卫 + `selection_clip` 剪裁） |
| 14 | move_lifted 采样：Swift 整像素 `.none`/分数 `.high` 插值；Rust 恒 Nearest，punch-out 用截断 `as u8` 非四舍五入 | `BrushStroke.swift:737` | `core/src/tools/brush.rs:719,724-729` |

### 5.3 缺失（本域内）

TypeTool 排版整体：Swift `textAttributes/textBoxSize/textImage`（字体/字距/行距/换行/对齐/padding/100k UTF-16 等校验，`TypeTool.swift:205-255`）+ 会话 `beginText/editActiveText/applyText`（`:67-196`）；Rust 实际渲染是 `app/src/text_bitmap.rs:1-134` 的 **5×7 点阵字体（76 字形）**，`font_name/alignment/tracking/leading/box_size` 全部被忽略（数据模型 `document.rs:331-427` 已移植但不被渲染消费）。（✅ 已移植：core `tools/text.rs` 真字体排版 + session 全流程；app 点阵字体已删除，见顶部修复进度。遗留：富文本样式面板 TypeControls 的 UI。）笔刷平滑 smoothing（`EditorSession+Brush.swift:77-87`）UI 有滑杆但路径不消费；笔刷参数校验与 100MP 预算（`BrushStroke.swift:173-177`）无；笔刷快捷键（`[`/`]`/Shift-[`/`]`/数字键不透明度，`EditorSession+Brush.swift:191-226`）无；蒙版绘画在 app 不可达（`app/src/canvas.rs:459` 硬编码 `is_mask: false`，core 支持）；`BrushCommit` 后台提交 actor（`BrushStroke.swift:844-902`）无。

### 5.4 简化

256px 瓦片引擎 → 整网缓冲（`brush.rs:5-10` 自述）；tip 渲染近似（CG 解析抗锯齿/25 站径向渐变 → 逐像素解析，非 bit-exact）；~~tip 两级缓存 → 每次重算~~ ✅ 已补（tip 按 (直径,硬度) 线程局部记忆化）；clone 采样 CG `.medium` → 最近邻（`brush.rs:456-460`）。

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
| 10 | Motion Blur profile：Swift `CIMotionBlur` 锥形衰减；Rust 改为高斯锥形权重（σ=radius，归一化；有测试） | `Filters.swift:135-138,167-173` | ✅ 已补（高斯锥形权重 σ=radius，归一化） |
| 11 | halving 算法：Swift vImage **Lanczos** + 8px padding + 尺寸向上取整 `(w+1)/2`；Rust 改为分离式 Lanczos3 + ceil(w/2)（8px padding 属实现细节未复刻） | `Rendering/DownsampleCache.swift:82-111` | ✅ 已补（分离式 Lanczos3、ceil 半尺寸；8px padding 属实现细节未复刻） |
| 12 | 预缩小阈值差一档：Rust `scale ≤ 0.5` 即 halve；Swift `factor < 0.5` 才启用（scale 恰 0.5 时 Rust 多缩一次）；且 Swift 缩小时非 Nearest 一律 Low、Rust 无 Low/Medium 概念 | `DownsampleCache.swift:32-35`、`Rendering/LayerRenderer.swift:42-44` | ✅ 已补（`downsample_level` 移植 `DownsampleCache.level`：`factor ≥ 0.5` 不缩、`floor(log2(1/factor))`、maxLevel 6；`resample`/`draw_affine` 改走该档位函数，恰 0.5 留给终采；`LayerRenderer.interpolation` 语义落地——缩小终采一律双线性（Low），放大且 High 走 Lanczos 终采（`lanczos_resize`/`lanczos_sample`），4 个新测试） |
| 13 | Add Noise seed：Swift 每次编辑随机（`Filters.swift:223`）；Rust app 硬编码 7（`state.rs:579`） | `Filters.swift:223` | `app/src/state.rs:579` |
| 14 | 选区相关渲染差异（与 D 域交叉）：蚂蚁线颜色相反且无动画（Swift 白实线+黑虚线相位动画 0.12s；Rust 黑实线+白虚线静止） | `Rendering/TransformOverlay.swift:196-210`、`EditorCanvas.swift:2001-2015` | `app/src/canvas.rs:914-921,1101-1106` |

### 6.2 缺失/简化

- ~~`run_filter` 在 core 内无调用方，由 app 直接对原尺寸图层像素调用，缺 blur padding 与 trim~~ ✅ 已补（`run_filter` 的 GaussianBlur 分支实现 grow→blur→trim 完整流程，app 滤镜预览/提交统一走 `run_filter`）。
- `render_effects` 无结果缓存（Swift `LayerEffects.swift:375-408` cache + EffectsPreviewCache）；无 100MP/80M 尺寸 guard（`LayerEffects.swift:441`、`MetalLayerEffects.swift:61`）；GPU 路径 inner glow 的 `size>0` gate（`MetalLayerEffects.swift:155`）Rust 无（`effects.rs:713`）。
- `mask_clip_image`（`render.rs:27-57`）已补 `mask_clip_image_limited` 的 limit 截断；placement 缓存仍缺。
- effects coverage 的 Gaussian 4σ 截断 vs CI 约 3σ（`raster.rs:554,596`）。

---

## 7. 调整（C 域）

### 7.1 行为差异/简化

| # | 差异 | Swift 位置 | Rust 位置 |
| --- | --- | --- | --- |
| 1 | **app 层 `levels_auto` 与原版算法不同**：Swift 三模式（Contrast/Color/Neutral）、单通道 256 bin、clip 0.001、Neutral gamma=log(mean)/log(0.5)；Rust 对 1024 个混合 bin 求 0.5%/99.5% 分位、无三模式、无 gamma | `Document/LevelsAutomatic.swift:4-37` | `app/src/sheets.rs:382-422` |
| 2 | HueSaturation 渲染路径：Swift 构建 33³ CIColorCube（三线性 + f32 量化）应用；Rust 逐像素 f64 精确计算 HSL——语义等价但输出非 1:1（`dimension=33`、`previewLimit=8000` 无对应） | `Document/HueSaturation.swift:234-243,383-405` | ✅ 已补（`ColorCube` 33³ 构建 + f32 三线性采样 + 非预乘/预乘包裹，`hue_saturation_apply` 改走 cube；与精确逐像素数学 ≤3/255；`previewLimit=8000` 属对话框预览机制，留待 §9.3 面板批次） |
| 3 | 各 `apply` 缺 `is_valid` 检查：Swift 在 CameraRaw/Curves/Exposure/GradientMap/BlackWhite/ColorBalance/Grain 的 `apply` 均先 `guard isValid else throw`；Rust 全部静默处理 | `Document/CameraRaw.swift:224-225`、`Curves.swift:34`、`ImageAdjustments.swift:65,89,133,166,202` | `core/src/adjust/camera_raw.rs:1584-1692`（`is_valid` 定义于 `:1426` 未被 apply 调用）、`curves.rs:92-103`、`image_adjustments.rs:123-595` |
| 4 | 选区 coverage 生成：Swift 把路径经逆变换后重新光栅化（亚像素精确）；Rust 对预生成 coverage 位图做 floor 最近邻查找（旋转/缩放图层边缘不一致），且 `pixel.rs:25-33` 有一段无效死代码 | `Document/PixelAdjust.swift:23-34` | `core/src/adjust/pixel.rs:10-52` |
| 5 | 选区回混精度：Swift `CIBlendWithMask`（浮点）；Rust 整数 `(adj·a+orig·(255−a)+127)/255`（可差 1） | `PixelAdjust.swift:38-46` | `core/src/adjust/pixel.rs:58-82` |
| 6 | GuidedMatte 重采样：Swift CGContext `.high` 一步缩放；Rust halve+resample 组合，滤波器不同非位等（guided_filter 本体 epsilon=1e-4 一致） | `GuidedMatte.swift:68-117` | `core/src/adjust/pixel.rs:257-281` |
| 7 | 几何 warp：Swift 用 `CIPerspectiveTransform`（越界透明）；Rust 手写单应 + bilinear（越界输出透明（对齐 CI） | `CameraRawGeometryCalibration.swift:77-83` | ✅ 已补（越界输出 premultiplied 透明黑） |
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
| 2 | contains/rasterize 用偶奇规则；Swift 用 winding（自交套索结果不同） | `Selection.swift:22,259` | ✅ 已补（`winding_contains`：有向穿越计数非零即内） |
| 3 | 椭圆：Swift `addEllipse` 4 段 Bézier；Rust 改为 kappa 四段 Bézier 折线化（面积差 <2%） | `Selection.swift:218` | ✅ 已补（kappa 四段 Bézier 折线化，面积差 <2%） |
| 4 | 环→多边形配洞按外接矩形取第一个命中：三层嵌套（岛中湖）配错父级、语义反转；CGPath 原生支持任意嵌套 | （CGPath 原生） | `core/src/selection.rs:320-353`（`:339-346` 的 break） |
| 5 | expand/contract 描边带：Swift 整条路径一次 `strokingWithWidth`；Rust 逐环 offset+作差再并，圆弧 π/16 密度、小环退化时整环按 band 覆盖 | `Selection.swift:336` | ✅ 已补（圆弧 π/16 密度；极小环翻转检测整环覆盖，contract 收缩） |
| 6 | coverage 光栅化：Swift CG 解析抗锯齿；Rust 4×4 超采样（16 级灰度）——README "scanline coverage" 名不符实 | `Selection.swift:19-22` | ✅ 已补（解析面积分光栅化，边缘像素精确覆盖，见顶部修复进度） |
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
| 4 | 导出无尺寸 guard、无 DPI 元数据（pHYs）、非原子写 | `ImageExporter.swift:22-23,80-82,111-138` | ✅ 尺寸 guard 与 DPI 元数据已补（`export.rs` 的 `check_export_size` + PNG pHYs 注入/JPEG JFIF 密度修补；app 侧接线待导出路径调用） |
| 5 | 拖放导入（ImageFileDrop）、"Open With"/Dock 打开、RAW 导入 | `IO/ImageFileDrop.swift:7-33`、`IO/CompositorApplicationDelegate.swift:14-22`、`IO/RawImporter.swift` | 无 |

### 9.2 PSD 细节差异

| # | 差异 | Swift 位置 | Rust 位置 |
| --- | --- | --- | --- |
| 1 | 图层名 Pascal 串解码：Swift macOSRoman（Latin-1 兜底）；Rust `from_utf8_lossy`（非 ASCII 单字节名变 U+FFFD） | `IO/PSD/PSDReader.swift:154` | ✅ 已补（完整 MacRoman 表，见顶部修复进度） |
| 2 | 闭合路径收口角：CG 对 closeSubpath 计算收口 miter join；Rust 收口角走 miter/bevel（限长 10:1） | `IO/PSD/PSDVector.swift:62-79` | ✅ 已补（closed 标志传入描边，收口走 miter/bevel） |
| 3 | 矢量光栅化：CG 解析抗锯齿 winding → Rust 自写 8 样本 scanline（已声明替换） | 同上 | `core/src/psd/vector.rs:726-826` |
| 4 | fixture 全局 composite：Swift 经 context 重绘；Rust 尺寸不符时 resample 近似 | `CompositorTests/PSDFixture.swift:230-245` | `core/src/psd/fixture.rs:333-340` |
| 5 | live shape 模型：Swift `LayerShape = style + CGImage`（像素身份）；Rust 纯几何模型 + 合成默认 `line_width:4.0, filled:true` | `Document/ShapeTool.swift:37-53` | `core/src/psd/types.rs:246-258` |
| 6 | 蒙版转换失败报告消息不可达（Rust `layer_mask_asset` 无失败路径，可接受） | `IO/PSD/PSDDocumentBuilder.swift:95-99` | `core/src/psd/document_builder.rs:190-199` |

### 9.3 UI / 应用壳

**快捷键缺失**（`UI/KeyboardShortcuts.swift:74-124` vs `app/src/keys.rs`）：**（已修一批，见下）**原缺：Save As(⇧⌘S)、Export JPEG(⌥⇧⌘S)、Close(⌘W)、Show Transform Controls(⌘H)、剪贴板四件套(⌘X/⌘C/⇧⌘C/⌘V)、Fill(⌥⌫/⌘⌫/⇧⌫)、Curves(⌘M)、Invert(⌘I)、Clipping Mask(⌥⌘G)、Move Layer Up/Down(⌘]/⌘[)、Snap 总开关(⇧⌘;)、Lock Guides(⌥⌘;)、笔刷硬度(⇧[/⇧])、混合模式步进(⇧−/⇧=)、循环形状(⇧U)、Levels 预览(⌥P)。**✅ 以上均已绑定并接 handler**（⌘M 的 Curves sheet UI 仍属 §9.3 面板批次；⇧⌫ Content-Aware Fill 走 core `content_fill` 直接提交，无 sheet）；Lock Guides 同时补上 core 守卫——`hit_guide`/`begin_guide_creation`/`begin_guide_move` 在 `locks_guides` 时拒绝（Guides.swift:74/92，有测试）；Levels 预览开关落地为 sheet 的 Preview 勾选框（`edit.preview` 语义，⌥P 翻转）。**仍缺**：Select Subject(⌥⌘A，core 无 select_subject)、Select 工具(A)、移动选中像素(⌘方向 8 条，core 无 nudge_pixels)。~~不透明度数字键、完成文本(⌘⏎)、Tracking/Leading(⌥方向 8 条)、临时手型(Space)~~ **✅ 已补**：不透明度数字键走 `type_opacity_digit`（0.6 秒两键窗口、按工具路由 brush/gradient/多选透明度并同步滑杆，EditorSession+Brush.swift:191-208）、⌘⏎ 完成 TextDraft、⌥方向字距/行距（⇧=×10，InlineTextEditor.swift:12-22）、Space 按住临时手型（keyUp 恢复工具，gpui `on_key_event` 双阶段分发按 Bubble 过滤）。**键位不同 ✅ 已对齐**：Export PNG 改 `ctrl-shift-e`（Swift ⇧⌘E）、Export JPEG 改 `ctrl-alt-shift-s`（⌥⇧⌘S）、Show Guides 改 `ctrl-;`（⌘;）、Show Grid `ctrl-'`（⌘'）、Snap 总开关 `ctrl-shift-;`、Lock Guides `alt-ctrl-;`。**整个快捷键自定义系统缺失**（`KeyboardShortcuts.swift:129-319`）。

**菜单缺失**（`CompositorApp.swift:47-318` vs `app/src/menu.rs`）：**（已修一批）**Import Images、Save As、Export JPEG、Transform Controls 开关、总 Snap 开关、Lock Guides、Cut/Copy/Copy Merged/Paste、Content-Aware Fill、Invert、Create/Release Clipping Mask、Move Layer Up/Down **✅ 均已补**。仍缺：Fit/Actual Pixels/Zoom 组；Clear Selection Pixels；Select 菜单的 Layer's Pixels/Subject/Mask's Black Areas；Image 菜单的 Curves/Levels/Hue-Saturation（**sheet 存在但菜单无入口**）及 Black & White/Color Balance/Exposure/Gradient Map/Grain；Filter 菜单缺 Remove Background（core `filters.rs:26` 有 case）；Layer 菜单的 New Adjustment Layer 子菜单/Edit Adjustment/Transform Layer/Selection/Layer via Copy/Move Out of Folder/Rename/Show-Hide/动态 Merge 与 Delete 标题。菜单项禁用逻辑基本未用（`ui.rs:85-88` 机制存在）。

**面板/控件**：~~完全缺失——ColorPickerSheet、EffectsSheet~~ **✅ 首批已补**：EffectsSheet（core `effects_session.rs` 全套编辑会话——add/select/finish/set/change/copy effect、undo 名 "Add|Edit|Cancel|Copy <Kind>"、新增 stroke/overlay 取背景色、Cancel 只还原所编辑种类、编辑绑定打开面板时的图层；app 侧 Effects sheet（SheetKind::Effects）+ 图层面板脚部 ✨"Fx" 菜单入口，六种效果各自的滑杆组对齐 EffectsSheet.swift）；ColorPickerSheet（core `color_picker.rs`——can_edit_palette/set_palette_color/open(close)_color_picker/open_text|effect_color_picker/preview_effect_color/type_color，四 target 中 GradientMap 因 core 无 filter 编辑会话暂留 no-op；app 侧 `color_picker_ui.rs` HSB 滑杆+色样+Hex+OK/Cancel 面板，**色板点击由"交换"改为打开取色器**（X 键仍交换），效果取色实时预览到图层）。仍缺：CurvesControls、PSDConversionSheet、RawDevelopSheet、CameraRaw 的 Curve/Mixer/Grading/Detail/Optics/Geometry/Calibration 分区 + 直方图/示波器/剪裁指示/白平衡吸管（app 仅 14 滑块，`app/src/sheets.rs:189-213`）、浮动面板系统（全部为模态居中 overlay，`app/src/sheets_ui.rs:123-145`）、TransformInspector（Rust Move 头只有一个**空 handler** 的 Auto-select 勾选框，`app/src/tools.rs:500-507`）、TypeControls（Rust 只有一行提示文字，`tools.rs:512-514`）。**行为差异**：图层面板缩略图（Swift 整画布形状+棋盘格+蒙版边缘灰度，`UI/CanvasThumbnail.swift:10-62`；Rust 内容裁剪式无棋盘格，`core/src/render.rs:459-472`）；面板宽度固定 248（Swift 202–352 可拖记忆）；混合模式无悬停预览；标尺刻度候选集/目标间距不同且无 label（`UI/CanvasRulers.swift:174-184` vs `app/src/canvas.rs:1205-1214`）；标尺拖拽建参考线无调用；Camera Raw 默认值差异（Gaussian Blur 默认 4 vs Swift 1、Motion Blur 20 vs 10、Add Noise 12 vs 10，`app/src/sheets.rs:170-182` vs `Filters.swift:39-45`）；取色器为滑杆组而非 Swift 的 SV 场+色相条自绘控件。（已修：app 建组原为 "Group N"+把活动层移进组+挂根，现改为调 core `add_group` 的 "Folder N" 语义，见 §3.1-13。）**多窗口**：单窗口固定 1560×920（`app/src/main.rs:30-49`），无标签条/关闭确认/未保存标记 UI（core 有 `is_modified`/`mark_saved` 未接 UI），CloseDocument 直接清空（`workspace.rs:236-244`）。**持久化**：无任何 UI 状态持久化。

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
