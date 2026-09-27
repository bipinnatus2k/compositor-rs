# 工具与选区子系统实现审计（2026-09-28）

审计范围：`crates/core/src/tools/`（brush / heal / distort / object_selection / warp / text / wand / content_fill / mask_tracing / clone_stamp / subject_removal / mod）与 `crates/core/src/selection.rs`。所有结论来自逐行阅读当前 `dev-ui-fix` 分支代码，行号均已核实；旧报告 `docs/agent-reports/D-selection.md`、`E-tools.md`（2026-09-24）仅作交叉核对。app 侧 `canvas.rs`/`state.rs`/`session.rs` 的接线代码为验证调用点而抽查，非全文审计。

## 一、总览表

| 功能 | 实际状态 |
|---|---|
| 画笔/橡皮/平滑（brush.rs 主体） | **基本完整但有缺陷**：Catmull-Rom 细分、连续沉积引擎（8 点 Gauss-Legendre 积分、密度叠加 `1-exp(-∫density)`）、spacing/不透明度封顶、羽化 tip 均为真实移植；缺陷：选区剪裁在网格坐标采样（缩放图层错位）、无参数/像素预算、预览每帧 O(像素×段数) |
| Spot Healing（heal.rs + brush.heal） | **完整实现**：膜求解（粗化+SOR ω=1.8）、5 因子×24 角度 patch 搜索+±3 精调、Box-Muller 颗粒，与 HealPixels.c 逐常数一致；但 heal 失败路径会把预览暗色 wash 永久提交 |
| Smudge/Liquify（warp.rs） | **完整实现**：拾色拖影/前向推动 + finish_warp 回写，与 Swift 逐常数一致 |
| Clone Stamp | **部分**：核心拷贝/替换语义在 brush.rs 完整；aligned 偏移机制在 session.rs:2867-2885 已实现但 **canvas.rs 未调用**（恒为 unaligned）；`tools/clone_stamp.rs` 整文件为死占位 |
| Blur 工具 | **基本完整但有缺陷**：经 displayed transform + 蒙版采样后模糊回写；mask 为目标时错误走 clone 颜色分支 |
| 渐变（fill_gradient） | **core 完整**（支持 alpha 插值、径向/线性）；app 侧蒙版渐变硬编码 `false`、无挂起重画 |
| 填充/清除/像素移动（fill/clear_pixels/lift/move_lifted） | **基本完整但有缺陷**：fill 不吃笔刷不透明度（正确）；clear_pixels 为二值清空丢失软边；选区剪裁同画笔的网格坐标问题 |
| Distort（distort.rs） | **完整实现**：单应/两三角仿射折叠、warp_trimmed/warp_mask/carried 均真实并有测试；个别退化路径静默回退 |
| 文字工具（text.rs） | **完整实现**：fontdue 真字体 + 平台字体查找/替身 + 逐词换行/对齐/tracking；会话 begin/apply_text 已接线；无行内编辑器 UI（点击铺默认文本） |
| Magic Wand（wand.rs） | **完整实现**：种子半径均值参考 + 扫描线洪填 + 沿像素边轮廓追踪，含 800 万边上限 |
| Object Selection（object_selection.rs） | **朴素近似（相对 Swift 的 Vision）但诚实**：joint bilateral 上采样（σ5/0.15）+ 3×3 腐蚀膨胀 + RDP 1.6 + Chaikin 3 轮，文档明说 Vision 是外部输入；**全工程无生产调用方**——Object 模式是无效开关 |
| Content-Aware Fill（content_fill.rs） | **朴素近似但真实**：由外向内 BFS + 4 邻域偏移传播 + 24 随机候选 + r=64 折半随机精化（与 ContentFill.c 逐常数一致）；**filter 管线接的是 no-op `run()`，真正的 `run_fill` 无生产调用** |
| Subject Removal（subject_removal.rs） | **stub**：`image.clone()` + TODO，且已接入 Filter 菜单 → "Remove Background…" 静默无效 |
| Mask Tracing（mask_tracing.rs） | **完整实现且已接线**：有向边缝合 + 直线折叠，session.rs:1415/1449 生产调用 |
| selection.rs（数据结构/布尔/光栅化/羽化） | **完整实现**：解析面积覆盖率（非采样）+ nonzero winding + geo 稳健布尔 + σ=feather/2 边缘钳位高斯；一处注释漂移（"Even-odd"） |

---

## 二、逐条发现

### 高严重度

- **[高] Subject Removal 是 stub 且已接入菜单** — `crates/core/src/tools/subject_removal.rs:7-10`（`run` 返回 `image.clone()`，`// TODO(tools-pass): subject mask + guided matte refine + mask-as-layer-mask`）被 `crates/core/src/filters.rs:369` 直接分发，app 侧 `crates/app/src/workspace.rs:916` → `FilterKind::RemoveBackground`（menu 入口 `crates/app/src/menu.rs:182` "Remove Background…"）。用户点击后经 `filters.rs:404-405` 的 selection blend 得到与原图逐位相同的结果——功能在产品里静默无效。
- **[高] Content-Aware Fill 菜单命令是 no-op，真实算法未接线** — `crates/core/src/tools/content_fill.rs:250-253` 的 `run()` 注释自称 "Standalone entry (no selection)"，无条件 `image.clone()`；`filters.rs:370` 的 `FilterKind::ContentAwareFill => content_fill::run(image)` 完全忽略 `job.selection`（`filters.rs:218` 字段存在）。真实算法入口 `content_fill.rs:236-247 run_fill(image, selection, mapping)` 只有测试调用（`content_fill.rs:282/297/312`）。app 侧 `crates/app/src/state.rs:779-821 run_content_aware_fill`（Shift-Delete，`crates/app/src/shortcuts.rs:110`）传入 selection 后依然拿到原图，并且**无条件开一笔 "Content-Aware Fill" 撤销历史并回写相同像素**（state.rs:805-818）——什么都不做还制造一步 undo。
- **[高] 选区剪裁按笔刷网格坐标采样，缩放图层上剪裁位置错乱** — `crates/core/src/tools/brush.rs:722-741`（result 尾段）、`841-849`（fill）、`878-880`（clear_pixels）、`800-805`（fill_gradient）、`919`（lift_selection）、`941-954`（move_lifted punch-out）都把网格像素下标 `x,y` 直接喂给 `SelectionClip::alpha_at`（`selection.rs:554-567`，其语义是"document pixel"，rect 为文档坐标）。网格分辨率 = 图层像素空间（`brush.rs:150-159`），仅当图层变换为 1:1 时与文档像素重合；图层被 Move 工具缩放后，所有画笔/橡皮/填充/渐变/像素移动的选区边界按缩放因子错位。该偏差被集成测试注释**自认**：`crates/core/tests/selection_edit_tests.rs:200-210`（"on a scaled layer the clear wipes the whole grid; the case runs over a 1:1 full-canvas image instead of Swift's 2×-stretched one"）。
- **[高] 笔刷预览每帧全画布 O(像素 × 段数) 重渲，极端性能陷阱** — `crates/core/src/tools/brush.rs:624-658 effective_coverage` 在 `result()`（:672）中对**整张画布网格**逐像素调用 `render_continuous_tile`（:1179-1226），后者每个像素遍历全部 settled 段（硬笔 `:1198-1204`，软笔每段再做 8 点 Gauss 积分 `:1212-1215`），且没有按段的包围盒剔除（`:1188-1194` 只剔除画布外像素）。app 拖拽预览路径 `crates/app/src/state.rs:1313-1318`（composite）→ `crates/app/src/canvas.rs:52` 每帧调用。长笔画在 2K/4K 画布上是每帧数十亿次运算；段数随笔画长度无界增长（`samples`/`segments` 无上限，`brush.rs:303-336`）。另外 `append`→`curve`→`walk_to(stamp=true)`→`dab`（`brush.rs:473,497,514-564`）仍同步盖章整套 dab 网格，而该网格在引擎路径下只被 `has_paint()`（:567-569）消费——每事件双倍浪费。
- **[高] Wand 的 Object 模式是无效开关：core 算法完整但零调用方** — `crates/core/src/tools/object_selection.rs:66-87 select`、`:92-106 instance_index` 在整个 workspace 内无任何生产调用（grep 验证：仅 mod.rs 声明 + 本文件测试；模块首行 `:17` 标了 `#![allow(dead_code)]`）。UI 却完整暴露该模式：`crates/app/src/tools.rs:401-422` 选项栏可勾选 "Object"，`crates/app/src/canvas.rs:1053-1059` Tab 可切换 `WandMode::Object`，但会话入口 `magic_wand`（`crates/core/src/session.rs:1606-1673`）**从不读取 `self.wand_mode`**，无条件走 `wand_mask`。用户切到 Object 后行为与 Wand 完全相同且无提示。
- **[高] Spot Healing 失败时把预览暗色 wash 永久写入图层** — `crates/core/src/tools/brush.rs:575-614 heal()` 在 `spot_heal` 返回 false 时（如覆盖层触到图层四边、work box 内无 RING 像素，`heal.rs:271-274`）不清 coverage、返回 false；app 侧 `crates/app/src/canvas.rs:729` **忽略返回值**继续 `paint_snapshot()` 提交——此时 `result()` 走 healing 分支（`brush.rs:704-708`，rgb(31) @ 45%），暗色 wash 以真实像素落盘。

### 中严重度

- **[中] RasterEdit 无参数校验/像素预算，超大直径可触发巨量分配** — `crates/core/src/tools/brush.rs:128-210 new()` 不校验 diameter/hardness/opacity（Swift `BrushStroke.swift:173-177` 有 1…2000 等钳制）；`render_tip`（:236-260）按 `diameter.ceil()` 分配方阵灰度图，`tip()` 缓存（:216-233）上限 32 条目但不清单条尺寸——异常大的直径直接分配 size² 字节。30k/1 亿像素级预算检查在 text.rs:223-231 与 distort.rs:198 存在，brush.rs 没有。
- **[中] Blur 工具 + 蒙版目标走错渲染分支** — `crates/app/src/canvas.rs:606-613` 只对 SpotHealing/CloneStamp 拒绝蒙版目标，Blur 放行；随后 `:664-670` 设置 `edit.clone = (blurred, 0)`，而 `brush.rs:681-699` 的 clone 分支优先于 `:709-716` 的 mask 分支执行——对蒙版的"模糊"会把图层模糊样的 RGBA 写进蒙版网格（蒙版快照只取 R 通道，`brush.rs:764-776`），结果语义错误。
- **[中] Clone Stamp aligned 语义在 app 层缺失（core 已备好未用）** — `crates/core/src/session.rs:2867-2885 clone_stroke_offset/begin_clone_stroke` 实现了整像素取整 + aligned 首笔固定偏移，且 `crates/core/tests/tool_flow_tests.rs:365-401` 有测试；但 `crates/app/src/canvas.rs:653-660` 起笔时自行计算 `offset = clone_source - doc`（不取整、不区分 aligned），从不调用会话接口——`clone_settings.aligned`（session.rs:83）在产品路径上是死配置。
- **[中] clear_pixels 二值清空，丢失抗锯齿/羽化软边** — `crates/core/src/tools/brush.rs:874-893`：`alpha_at > 0.0` 即整像素清零（含 RGB），Swift 是 destinationOut 按覆盖率部分擦除；测试注释自认（`crates/core/tests/selection_edit_tests.rs:207-210`，断言被迫从"软边"改成"锐利"）。
- **[中] 蒙版渐变不可达** — `crates/app/src/canvas.rs:937-962 begin_gradient` 中 `RasterEdit::new(..., false, ...)`（:955）蒙版参数硬编码 false；core `fill_gradient` 本身与选区兼容。同时渐变在鼠标抬起即提交（`canvas.rs:482-487`），无 Swift 的挂起/重画/resloveGradient 流程。
- **[中] Marquee 的 Option 从中心拖拽恒关闭** — `crates/app/src/canvas.rs:422` `drag_marquee(doc, shift, false)`，`from_center` 硬编码；`selection.rs:854-877 drag_box_rect` 的 from_center 分支在 app 内不可达（仅测试调用）。
- **[中] Object Selection 若接线将有严重性能问题：每输出像素 441 次双边核采样** — `crates/core/src/tools/object_selection.rs:124-172 joint_bilateral_upsample`：radius = ⌈2σ⌉ = 10（:128），每像素 21×21=441 邻域、每次两次 `guide_luma` 位图读取，4K 图 ≈ 36 亿次权重计算，单线程无分块。接线前需重构（积分图/降采样/并行）。
- **[中] move_lifted punch-out 通道截断而非四舍五入** — `crates/core/src/tools/brush.rs:949-953` `as u8` 截断（E 报告已指出，至今未改）；同函数的采样策略（整像素 Nearest / 分数 High，:956-968）已修复。
- **[中] Heal 搜索区对长笔画是一次性巨量拷贝** — `crates/core/src/tools/brush.rs:586-598`：`heal_region`（:1018-1024，`(max边+32)*3.2` 外扩）对 1000px 长笔画生成约 7600×6600 的子位图（≈200MB RGBA + 50MB 灰度）整块拷贝，而 `spot_heal` 实际只在 spot+ring 的 work box（`heal.rs:217-224`）内求解、patch 源最多外移 ≈2.8×work box。与 Swift 行为一致但内存峰值高，长笔画有卡顿/内存尖峰风险。
- **[中] 受控输入下的浮点 unwrap panic 点** — `crates/core/src/tools/object_selection.rs:305`（`partial_cmp(...).unwrap()`，ring 含 NaN 即 panic；当前生产不可达）；`crates/core/src/selection.rs:727`、`:803`、`:814`（排序 `partial_cmp().unwrap()`；ring 坐标为 NaN 时恐慌。上游 `drag_marquee/extend_lasso` 已有 isFinite 守卫（session.rs:1238/1259），风险被上游挡住，但 API 层无防护）。

### 低严重度

- **[低] `tools/clone_stamp.rs` 整文件死代码** — `crates/core/src/tools/clone_stamp.rs:1-19`：19 行占位（doc 自称 "Placeholder"），其 `CloneSettings` 与 `session.rs:79-95` 的同名结构重复，全工程无人使用（grep 验证：仅 session.rs 用自己的那份）。
- **[低] `RasterEdit::fill_color` 字段从未被赋值** — `crates/core/src/tools/brush.rs:106`（声明）、`:201`（初始化 None）、`:664-669`（读取）；`unwrap_or` 的 settings-RGB 分支恒定生效，该字段及其分支为死代码。
- **[低] `heal_coverage_bounds` 无调用方** — `crates/core/src/tools/heal.rs:15-24` pub 函数，生产/测试零调用（实际路径走 `raster::coverage_bounds`）；`raster.rs:832` 的 doc 反向引用它。
- **[低] `distort::map_path` 无生产调用方** — `crates/core/src/tools/distort.rs:412-443`，仅本文件测试（:560-577）调用；session.rs 的 distort 流程（:4055-4220）用的是 corners_of/is_usable/carried/warp/warp_mask/warp_trimmed。
- **[低] `render_one` 仅测试使用** — `crates/core/src/tools/brush.rs:1136-1157`（非 pub，非测试构建为 dead code）。
- **[低] selection.rs 注释与实现矛盾（文档漂移）** — `crates/core/src/selection.rs:64` "Outer rings and hole rings. Even-odd fill decides coverage." 与实际 nonzero winding 规则（`:821-827 winding_contains`、`:580-599 rasterize`）矛盾；`:1003-1039` 的测试明确断言 "even-odd would hollow this out" 的相反结果。同文件 `:8-9` 模块 doc（analytic/winding）是对的，仅该行残留旧注释。
- **[低] 字体缓存互斥锁毒化时静默返回 None** — `crates/core/src/tools/text.rs:41-43、:47-48` `lock().ok()?` 把中毒锁当成"没有字体"，text_image 随后报 "no font could be loaded"——错误吞掉真实原因。另 `:127` 用 `catch_unwind` 包住 fontdue 解析（正确的防御）。
- **[低] Homography 退化时静默仿射回退** — `crates/core/src/tools/distort.rs:108-118`：`den.abs() <= 1e-12` 时 g/h 保持 0，按仿射解释四角，无任何告警；`apply_inverse`（:141-154）退化时返回 NaN 点，`warp` 的透视分支依赖 `sample_into` 的越界检查（:283-291，NaN 比较恒 false，落到 clamp 后的饱和转换）——不 panic 但可能产出可疑像素，触发条件极端。
- **[低] 蚂蚁线无相位动画、颜色与 Swift 相反** — `crates/app/src/canvas.rs:1326`（"dark under-stroke + white dashes"）、`:1512-1516 paint_polygon`（固定 `[4,4]` dash，无 phase 参数/定时器）。D 报告当时的描述至今仍成立。
- **[低] Clone 采样为最近邻 floor** — `crates/core/src/tools/brush.rs:686-688`（Swift 经 `.medium` 插值）；E 报告简化#4 仍成立。
- **[低] 渐变端色插值截断** — `crates/core/src/tools/brush.rs:829` `as u8` 截断（最多差 1 级）。
- **[低] 源码格式瑕疵（无害）** — `crates/core/src/tools/brush.rs:293`（`pub fn append(&mut self, point: CGPoint) {        if !point.x.is_finite()` 挤在同一行）、`crates/core/src/selection.rs:303`（同类）。
- **[低] wand_trace 的 OOM 错误类别被折叠** — `crates/core/src/tools/wand.rs:122` 注释如实声明（"2 = out of memory — collapsed to None/false here"），`TraceError` 只有 `TooManyEdges`（:235-238）；与 C 的双错误类别不完全一致，但注释与代码相符（不算欺骗性声明）。
- **[低] mask 光栅化非抗锯齿路径逐点采样** — `crates/core/src/selection.rs:588-596` 像素中心判定，与 CG 硬边扫描转换在边界像素上有 ±1 像素差（Swift 同为近似，可接受）。

---

## 三、与旧报告 D/E 的矛盾点

D/E 报告整体已**严重过时**：D 报告的"选区域缺失约一半"与 E 报告的"brush 是最大差距来源"在今天均不再成立。逐条核对结果：

### 与 D-selection.md 矛盾（旧报告论断 → 现状）

1. D §二"抗锯齿 scanline coverage 不属实（实为 4×4 超采样，selection.rs:505-529）" → **已过时**。现为解析面积填充（`selection.rs:580-599` + `627-809`），有 `rasterize_exact_fractional_edge_coverage`（:971-983）、`rasterize_exact_diagonal_coverage`（:986-1000）逐值测试；README 该声称如今成立。
2. D 行为差异#2"contains/rasterize 用偶奇规则" → **已过时**。现为 nonzero winding（`selection.rs:821-850`，测试 `:1003-1039`、`:1140-1170`）；唯一残留是 `:64` 的旧注释（见低严重度条目）。
3. D 行为差异#1"coverage_bounds ceil 次序（feather.ceil()*2）" → **已修复**：`selection.rs:494` 现为 `(self.feather * 2.0).ceil()`。
4. D 行为差异#4"三层嵌套配洞按第一个外接矩形、岛中湖反转" → **已修复**：`selection.rs:318-378` 最小面积父环规则 + 四个专项测试（`:1187-1271` 三层/兄弟洞/四层/布尔后再分组）。
5. D 缺失#4"MaskTracing 整体缺失" → **已过时**：`mask_tracing.rs` 全量实现，session 生产调用在 `session.rs:1415`（dark_pixels）、`:1449`（opaque_pixels），测试 `session.rs:6133-6179`。
6. D 缺失#1/#3/#6"SelectionEdits/FloatingSelection/轮廓移动/像素移动缺失、selection_move_origin 死字段" → **已过时**：`session.rs:2001 fill_selection`、`:2029 clear_selected_pixels`、`:2269 invert_pixels`、`:2103 move_selection_outline`、`:2127 begin_pixel_move`（`:2236-2240` 方向键微调）；`selection_move_origin` 已被消费（`:1213、:2092-2096`）；floating 流程存在于 `:4590-4705`。
7. D 缺失#5"ObjectSelection 整体缺失，Object 模式是无效开关" → **半过时半成立**：core 已完整实现（object_selection.rs），但"Object 模式无效开关"的结论**至今仍成立**且更讽刺——UI 与 core 都有了，中间没接线（见高严重度条目）。
8. D 行为差异#9"wand 超详细轮廓静默失败" → **已修复**：`session.rs:1654-1659` 把 `Err` 写入 `brush_error` 并提示用户。
9. D 行为差异#10"守卫削弱（无 canEditSelection/草稿期可 Expand/NaN 可入草稿）" → **基本已修复**：`session.rs:1515-1519 can_modify_selection` 含 `can_edit_layers` 与 `lasso_draft.is_none()`；`drag_marquee/extend_lasso` 有 isFinite（`:1238、:1259`）。
10. D 行为差异#11"蚂蚁线颜色相反且无动画" → **仍成立**（canvas.rs:1326/1512-1516），唯一未被修复的 D 行为差异。
11. D 行为差异#5"expand/contract 描边带差异（圆弧上限 32、小环不产生带）" → **部分修复**：圆弧上限现为 64、π/16 步长（`selection.rs:281`），小环退化情形有专门处理与测试（`:172-185`、测试 `:1314-1325`）；仍是逐环构造而非整条路径一次描边，几何细节与 CG 仍有差。

### 与 E-tools.md 矛盾（旧报告论断 → 现状）

1. E 行为差异#1"heal.rs:249 ring 膨胀行扫描 clamp 用错维度（.min(wh)）" → **已修复**：`heal.rs:249` 现为 `.min(ww)`，并有针对性测试 `ring_reaches_both_sides_of_tall_boxes`（`heal.rs:512-543`）。
2. E #2"单行/单列 spot 无法修复" → **已修复**：`raster.rs:833-861 coverage_bounds` 只在完全为空时返回 None（注释明说 1px 是合法退化矩形）；`heal.rs:209-216` 不再拒绝；测试 `heal.rs:546-580`。
3. E #3"heal 在整个图层网格上搜索、与 Swift 区域不同" → **已修复**：`brush.rs:586` 用 `heal_region`（`:1018-1024`，`(max+32)*3.2`，与 Swift BrushStroke.swift:776-784 一致）截取子区域。
4. E #4"provisional tail 的 coverage 不撤销" → **已按更优方案解决**：tail 永不进入永久层——`brush.rs:110`（tail_segment 仅预览）、`:1179-1226`（引擎永久/尾段分离）、测试 `tail_preview_never_enters_permanent_paint`（`:1806-1828`）与 `flush_replaces_the_tail_with_the_final_piece`（`:1356-1373`）。
5. E #5"GPU 连续沉积路径（MetalBrushCoverage）完全缺失" → **已过时**：`brush.rs:1026-1226` 完整移植（8 点 Gauss-Legendre `:1032-1033`、密度上限 20 `:1220/:1222`、AA 宽度 `:645-650`、间距不变性/旋转不变性测试 `:1689-1777`）。
6. E #6"旋转图层上笔刷画椭圆（语义相反）" → **已过时**：引擎在文档空间沉积（`brush.rs:634-653` 以 axis_x/axis_y 采样），旋转不变性有测试（`:1734-1777`）；旧 dab 网格路径仅在无事件段时兜底。
7. E #7"Clone Stamp aligned 语义缺失" → **半过时**：core 语义已实现（session.rs:2867-2885 + 测试），但 app 未调用（canvas.rs:653-660）——用户可见行为仍如 E 所述（见中严重度条目）。
8. E #8"渐变前景→透明退化为 fg→fg 纯色" → **已修复**：`crates/app/src/state.rs:1361-1369` 正确构造 `[fg α=1, fg α=0]`，并支持 reversed。
9. E #10/#12"Blur/Clone 采样直接取原始 asset、忽略变换与蒙版" → **已修复**：`canvas.rs:677-710 sample_for_clone` 走 displayed_transform + 蒙版覆盖率相乘；Blur 用同一采样（`:664-670`）。
10. E #11"heal 固定种子 0x6d2b79f5" → **已修复**：`canvas.rs:724-728` 每笔按时间重播种。
11. E #13"replaces_with_clone 保留 dst 的线性插值不等价" → **已修复**：`brush.rs:689-695` 改为 `src × (cov×opacity)` 不保留 dst，测试 `:1831-1857`。
12. E #14"fill() 吃 brush opacity" → **已修复**：`brush.rs:851-853` 注释+代码不吃 opacity（cov 恒 255、alpha 1.0），测试 `:1860-1874`。
13. E #15"撤销名只有三种" → **已修复**：`canvas.rs:737-748` 按工具/目标命名（Paint Mask/Erase/Blur/Clone Stamp/Brush Stroke），蒙版 healing 提交 "Spot Healing"（:732）。
14. E #16"笔画可见性守卫缺失" → **已修复**：`canvas.rs:570-624`（group、蒙版启用、调整层、空选区、SpotHealing/CloneStamp 不上蒙版）。
15. E 缺失#2"TypeTool 5×7 点阵占位" → **已过时**：`text.rs` 以 fontdue 真字体实现排版/换行/对齐/tracking，会话 `session.rs:4251 begin_text`/`:4355 apply_text`/`:4381` 调用 text_image。UI 仍无行内编辑器（`canvas.rs:1022-1039` 点击铺默认文本）——E 的会话流程缺失结论部分过时。
16. E 简化#3"tip 无缓存" → **已修复**：`brush.rs:216-233` 线程局部 (diameter,hardness) 缓存。
17. E 简化#8"finishWarp 无并发守卫、无选区剪裁" → **已修复**：`canvas.rs:798-804` 身份守卫、`:806-821` 传入 selection_clip。
18. E 缺失#4"笔刷平滑缺失" → **已修复**：`brush.rs:262-289 append_smoothed` 绳长模型 + `canvas.rs:588-595`（zoom 折算、仅 Brush 工具）+ `:405/:672` 接线，测试 `:1487-1512`。
19. E 缺失#7"蒙版绘画 is_mask 硬编码 false" → **已修复**：`canvas.rs:596-643` 完整蒙版目标路径（白/黑 by mask_paint_white）。
20. E 缺失#6"笔刷快捷键全部缺失" → **部分过时**：Shift-[/] 硬度（`state.rs:763-775`）、数字键不透明度（`session.rs:1172 type_opacity_digit`）已实现；`[`/`]` 尺寸步进与每工具笔尖记忆（E 缺失#5）仍无命中。

### E/D 中经核实**仍然成立**的论断

- SubjectRemoval 未移植（E 缺失#1）——且比 E 报告时更严重：现已接入 Filter 菜单成为静默 no-op（本报告高严重度第 1 条）。
- 笔刷参数校验与像素预算缺失（E 缺失#3）。
- 每工具笔尖记忆（parkedBrushTips）缺失（E 缺失#5）。
- 渐变蒙版不可用与无挂起重画（E 行为差异#9 的两个子项）。
- Clone aligned 的 app 层行为（E 行为差异#7 的用户可见部分）。
- 整网缓冲替代分块瓦片（E 简化#1，模块注释 `brush.rs:5-10` 自认）。
- 蚂蚁线无动画（D 行为差异#11）。

**矛盾点合计：D 报告 11 项关键论断中 9 项已过时（其中 8 项对应代码已修复/实现），1 项半成立（ObjectSelection），1 项仍成立（蚂蚁线）；E 报告 22 项核对点中 15 项已过时（对应代码已修复/实现），6 项全部或部分仍成立，1 项部分过时。**

---

## 四、总结论

1. **算法层质量高**：12 个文件中 10 个是完整、诚实的实现。三个 C 内核（heal/content_fill/wand）与 Swift 逐常数一致且边界 case 有测试兜底；selection.rs 的解析面积光栅化 + winding + 稳健布尔 + 羽化在同类实现里属于高完成度；brush.rs 的连续沉积引擎是对 Metal 内核的严肃 CPU 移植，非朴素近似。
2. **"智能"功能三缺二**：对象选择、内容识别填充、主体移除这三个听名字像 AI 的功能——object_selection 是真实算法但零接线；content_fill 是真实算法但 filter 管线接错入口；subject_removal 是纯 stub。用户在产品里能点到的三个入口（Object 模式、Shift-Delete、Remove Background…）**全部静默无效**，后两者还会制造空撤销步。
3. **一个系统性几何 bug**：选区剪裁在笔刷网格坐标而非文档坐标采样（brush.rs 六处 + 测试自认），缩放图层上所有选区类编辑错位——这是当前工具域最值得优先修的正确性问题。
4. **性能两处陷阱**：笔刷预览 O(画布×段数) 每帧重渲（大画布长笔画不可用级），以及 object_selection 一旦接线的 O(像素×441) 上采样。
5. **文档/注释卫生**：`selection.rs:64` 的 "Even-odd" 旧注释与 winding 实现矛盾、`content_fill::run` 的 "Standalone entry" 注释掩盖了接线错误、clone_stamp.rs 整文件与 session.rs 重复——三处都是"注释说 A、代码是 B"的实例；其余抽查的 doc comment（heal 的 mode 语义、wand 的错误折叠、brush 的 tail 语义）与代码相符。
