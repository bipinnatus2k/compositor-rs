> 子代理原始报告（域 E：绘画/修复/变形工具），逐字保留，2026-09-24。

# 绘画/修复/变形工具功能域 · Rust vs Swift 深度对比报告

## 总体结论

三个 C 内核的移植**接近但非完全 kernel-exact**：`heal.rs` 存在 1 处确认的代码偏差（ring 膨胀行扫描的 clamp 用错维度）+ 1 处边界条件差异（单行/单列 spot 无法修复）；`content_fill.rs` 逐常数一致，可认为 kernel-exact；`warp.rs`（Smudge/Liquify）逐常数一致。**brush.rs 是最大的差距来源**：无 GPU 连续沉积路径（MetalBrushCoverage 的积分模型完全缺失，Rust 复刻的是 Swift 的 CPU 回退路径）、provisional tail 的 coverage 撤销被简化、图层变换下笔刷形状语义相反、参数校验与像素预算缺失。**SubjectRemoval 完全未移植**（stub），**TypeTool 排版完全未实现**（5×7 点阵字体的占位实现），**Clone Stamp 的 aligned 语义、笔刷平滑（smoothing）、每工具笔尖记忆、渐变“前景色→透明”在 Rust 均缺失或行为错误**。README 的声称（brush 全要素复刻、heal/content_fill kernel-exact、warp+finishWarp、形状实时重渲）大部分方向正确，但 "brush 复刻” 与 “heal kernel-exact" 两条不完全成立，详见下文。

---

## 一、逐常数核对（一致项，确认无误）

| 常数 | Swift 位置 | Rust 位置 | 两边数值 |
|---|---|---|---|
| 软笔刷 falloff | `Compositor/Compositor/Document/BrushStroke.swift:76-79` `BrushRaster.falloff` | `crates/core/src/raster.rs:264-269` `falloff` | k=2.5，`(exp(-2.5u²)-e^-2.5)/(1-e^-2.5)` |
| dab 间距系数 | `BrushStroke.swift:409` `spacingFraction` | `raster.rs:272-278` `spacing_fraction`（测试 `raster.rs:703-706`） | hard≥1 → 0.015，否则 0.025 |
| 最小间距 | `BrushStroke.swift:413` | `crates/core/src/tools/brush.rs:325` | `max(0.25, diameter*fraction)` |
| Catmull-Rom 张力 | `BrushStroke.swift:392` `knot = t + max(0.0001, sqrt(dist))`，分片 `ceil(len/2)`（:398） | `brush.rs:298-310` | centripetal 参数化逐式一致 |
| coverage 混合 | `BrushStroke.swift:539,549`（hard=lighten / soft=screen） | `brush.rs:394-398`（max / `255-(255-a)(255-b)/255`） | 语义一致 |
| heal 膜求解迭代 | `Rendering/HealPixels.c:55`（300）、`:56`（粗层条件 w>32&&h>32&&depth<16）、`:86`（粗层后 40）、`:91`（omega=1.8） | `crates/core/src/tools/heal.rs:89,91,147,149` | 全部一致 |
| heal ring 宽度 | `HealPixels.c:122-124`（`size/8` clamp 2…16） | `heal.rs:215-216` | 一致 |
| heal patch 搜索 | `HealPixels.c:163`（factors `{1.05,1.35,1.75,2.25,2.8}`）、`:167`（24 角度， π/12 步）、`:172`（mode2 罚因子 1+0.6f / 其他 1+0.1f）、`:180-185`（±3 精调） | `heal.rs:281,285,293,305-313` | 全部一致 |
| heal 纹理颗粒 | `HealPixels.c:24-30`（hash `0x7feb352d/0x846ca68b`、unit=`>>8`/16777216）、`:223`（detail×0.9）、`:235-237`（Box-Muller `sqrt(-2ln(1-u1))cos(2πu2)`，key^`0x68e31da4`） | `heal.rs:27-39,382,402-405` | 全部一致 |
| content_fill | `Rendering/ContentFill.c:6-9`（LCG `1664525/1013904223`）、`:30`（radius=2 当 w,h≥5）、`:56`（seed `0x6d2b79f5`）、`:63`（28 候选=4 邻居传播+24 随机）、`:74`（随机细化 r=64→1 折半） | `crates/core/src/tools/content_fill.rs:11-18,77,145,162,185` | 全部一致（含 `donors[0]` 兜底、`DBL_MAX` 初值） |
| Smudge/Liquify | `Document/SmudgeLiquify.swift:40-42`（diameter≥2、hardness clamp 0…0.98、strength 0.01…1）、`:52`（radius=ceil(d/2)）、`:54-60`（weight: u≥1→0，u≤h→1，smoothstep `t²(3-2t)`）、`:70`（间距 smudge 0.08 / 其他 0.025，max 1）、`:129`（margin=ceil(|move|)+2）、`:152-164`（双线性采样） | `crates/core/src/tools/warp.rs:36-38,54-56,59-69,81,173,214-229` | 全部一致 |
| finishWarp 回写 | `SmudgeLiquify.swift:196-205`（diameter+4、hardness=1、opacity=1、clone=(result,.zero)、replacesWithClone=true、逐点 append+flush） | `warp.rs:260-277` `finish_warp_into_layer` | 一致 |
| 修复预览暗色 wash | `BrushStroke.swift:478,499`（rgb 0.12 → 31，alpha 0.45） | `brush.rs:480-481`（`[31,31,31]`，0.45） | 一致 |
| NoisePixels hash | `Rendering/NoisePixels.c:5-13,23,32,36-41` | `crates/core/src/filters.rs:249-258,270-276` | 同 hash/同 Box-Muller（属 Filters 域，此处仅确认） |

## 二、【行为差异】（两边都实现但结果不同）

1. **heal ring 膨胀行扫描 clamp 维度错误（真 bug）**
   - C：`Compositor/Compositor/Rendering/HealPixels.c:143` — 行扫描 `hi = x+ring+1 > ww ? ww : x+ring+1`（钳到行宽 `ww`）。
   - Rust：`crates/core/src/tools/heal.rs:249` — 写成了 `.min(wh)`（钳到 `wh`）。当工作盒 `wh > ww` 且 `x+ring+1 > ww` 时，`prefix[hi]` 读到本行从未写入的陈旧单元（恒 0），`near` 判定偏少 → RING 集合比 C 薄，修复结果偏离。列扫描 `heal.rs:264` 用 `.min(wh)` 是对的。**非方盒（ww≠wh）+ ring≥2 时必现**。

2. **heal 无法处理单行/单列 spot**
   - C：`HealPixels.c:8-22` `heal_coverage_bounds` 只在全空时清零，1 像素高的 spot 照常修复。
   - Rust：`crates/core/src/raster.rs:675` `coverage_bounds` 的 `if max_x <= min_x || max_y <= min_y { return None }` 把高度或宽度为 1 的 coverage 直接判空 → `heal.rs:201-204` `spot_heal` 返回 false，什么都不做。

3. **heal 修复区域范围不同**
   - Swift：`BrushStroke.swift:776-784` — 修复限制在 `painted.insetBy(-reach)`，`reach = (max(w,h)+32)*3.2`，patch 搜索只能在该子区域内取源。
   - Rust：`heal.rs:192-224` — 直接在整个图层网格上搜索。大图层上合法 patch 偏移集合不同 → 选中的源 patch 可能不同 → 像素结果不同。

4. **provisional tail 的 coverage 不撤销**
   - Swift：`BrushStroke.swift:354-387` `drawTail` 保存 tail 可达 tile 的 coverage 图（`tailBackup`），`removeTail` 精确恢复后再画曲线段。
   - Rust：`brush.rs:284-293` `restore_tail_state` 只恢复 `(previous, distance_to_next)` 间距状态，tail 已画进 `coverage` 的 dab 保留（注释自认）。软笔刷（screen 累积）下每次 pointer 事件都会留下直线 tail 的多余覆盖 → 与 Swift 结果系统性偏差。Swift 测试 `BrushTests.swift:371 liveStrokeReachesNewestSampleAndTailIsReplacedExactly` 验证的正是这一点。

5. **GPU 连续沉积路径无对应实现（MetalBrushCoverage）**
   - Swift：`Rendering/MetalBrushCoverage.swift:70-161` — 默认路径（`BrushStroke.swift:149-150,224` `useGPU=true` 时走 `appendContinuous`/`continuousCurve`，:281-309 自适应细分 0.2px 容差）；coverage = `1-exp(-∫tipDensity)`，8 点 Gauss-Legendre 积分（:119-131），软笔密度上限 20（:157-158），硬笔 AA 宽 `max(0.001, min(scaleX,scaleY))`（:50,92）。
   - Rust：`crates/core/src/tools/brush.rs` 无任何对应物（`continuous*` 零命中）。Rust 复刻的是 Swift 的 CPU 回退路径（逐 dab 间距盖章），因此 `BrushIntersectionTests.swift:55 accumulationDependsOnDistanceNotEventCount`（GPU 路径性质）在 Rust 上原理性不成立。

6. **旋转/非均匀缩放图层上的笔刷形状语义相反**
   - Swift：`BrushStroke.swift:546-547,559-560` — CPU dab 在**文档空间**画圆（`concatenate(inverse)` 后 fillEllipse/径向渐变），保证旋转/非均匀缩放图层上笔刷在文档内仍为圆（测试 `BrushTests.swift:197 brushStaysCircularOnNonuniformRotatedFlippedLayer`）。
   - Rust：`brush.rs:360-374` — 只把圆心经 `inverse` 映射，tip 恒在**图层像素空间**盖章；旋转图层上 Rust 画的是文档空间的椭圆。

7. **Clone Stamp aligned 语义缺失**
   - Swift：`Document/CloneStamp.swift:23-27` `cloneStrokeOffset`（aligned 时复用首笔记录的 `cloneOffset`，offset `.rounded()`），`EditorSession+Brush.swift:39` 在起笔时写入 `cloneOffset`。
   - Rust：`crates/app/src/canvas.rs:465-472` — 每次起笔都按 `clone_source - doc` 重算（恒为 unaligned 行为），`session.clone_offset`（`session.rs:148`）从未被写入；offset 也没有取整。`CloneSettings.aligned`（`session.rs:88`）是死配置。

8. **渐变“前景色→透明”（默认样式）实现错误**
   - Swift：`Document/Gradient.swift:78-80` `gradientColors` — foregroundToTransparent 为 `[fg α=1, fg α=0]`。
   - Rust：`crates/app/src/state.rs:430-439` — `[fg, fg]` 且 `straight_of`（state.rs:603-610）两端 alpha 均为 255 → 退化为不透明 fg→fg 纯色填充。core 侧 `brush.rs:556-604` `fill_gradient` 本身支持 alpha 插值，是 app 侧传参错误。

9. **Gradient 预览/提交模型差异**：Swift `GradientEdit` 挂起可重画、切工具自动提交（`Gradient.swift:36-108`）；Rust `canvas.rs:682-707,362-367` 拖完鼠标即提交（无挂起/重画/resloveGradient 流程），且 `mask:false`（canvas.rs:699）→ **蒙版渐变不可用**（Swift 支持，`GradientTests.swift:126`）。

10. **Blur 采样对象不同**
    - Swift：`Document/BlurTool.swift:8-40` `blurSample` — 画的是**图层经 displayedTransform 后的画布显示**（含蒙版分支：edge-tone 背景 + coverage），sigma=`min(30,max(1.5,d/10))`（CI 高斯）。
    - Rust：`canvas.rs:475-482` — 直接对 `layer.asset` 原图像做 `gaussian_blur_rgba`，无变换、无蒙版分支（sigma clamp 一致）。

11. **heal 随机种子**：Swift `BrushStroke.swift:793` 每次 `UInt32.random`；Rust `canvas.rs:510` 固定 `0x6d2b79f5`。颗粒纹理每次可复现但与 Swift 逐像素不同（Create Texture 模式）。

12. **Clone/Blur 采样源不同**：Swift `CloneStamp.swift:39-48` `cloneSample` 把图层经 `displayedTransform` 画到文档尺寸画布（含 sampleAllLayers 合成）；Rust `canvas.rs:488-496` `sample_for_clone` 单图层时直接取原始 asset 位图（变换≠单位阵时错位）。

13. **replaces_with_clone 的 copy 混合不等价**：Swift `BrushStroke.swift:464` `.copy` blend + `setAlpha(opacity)`：结果 = src×(cov×opacity)，**不保留 dst**；Rust `brush.rs:461-468`：`out = out*(1-a) + s*a`（线性插值保留 dst）。半覆盖像素上两者不同（影响 finishWarp/ Blur 的软边）。

14. **`fill()` 是否吃 opacity**：Swift `BrushStroke.swift:626-631` `fill(_:)` 直接 `setFillColor + fill`，不吃 `settings.opacity`；Rust `brush.rs:607-638` `fill` 乘了 `settings.opacity`（默认 1 时无差，改过笔刷不透明度后填充变淡）。

15. **撤销名称不一致**：Swift `EditorSession+Brush.swift:141`（"Brush Stroke"/"Clone Stamp"/"Blur"/"Paint Mask"…）；Rust `canvas.rs:512-521` 只产生 "Spot Healing"/"Erase"/"Paint"（clone/blur 笔画也被记为 "Paint"）。

16. **笔画可见性守卫缺失**：Swift `EditorSession+Brush.swift:5-11` `canPaint`（可见性、蒙版启用、调整层禁画等）未在 Rust `canvas.rs:427-439` 对应（只挡 is_group）。

## 三、【缺失】（Swift 有、Rust 无）

1. **SubjectRemoval 整体未移植**：`Document/SubjectRemoval.swift:1-143`（Vision 前景分割 `vision`、`refined` 的 guided-matte 抠边/Shift Edge 阈值移边/matteContrast、`subjectMask` 与既有 mask 相乘、`run` 预览、`selectSubject` 选区化）；Rust `crates/core/src/tools/subject_removal.rs:7-10` 仅 `image.clone()` stub（`// TODO(tools-pass)`）。README 未给该行条目，属纯缺失。
2. **TypeTool 文字排版全部缺失**：Swift `Document/TypeTool.swift:205-255` `textAttributes/textBoxSize/textImage`（NSFont 字体、`.kern` tracking、minimum/maximumLineHeight=leading、byWordWrapping 换行、left/center/right 对齐、padding=12、boxSize 固定段落框、100k UTF-16/2000pt/30k px/100MP 校验）；Rust 实际渲染是 `crates/app/src/text_bitmap.rs:1-134` 的 **5×7 点阵字体**（76 字形），`render_text(content, font_size, color)` 只用字号，`font_name/alignment/tracking/leading/box_size/padding` 全部被忽略（数据模型 `document.rs:331-427` 已移植且有校验，但渲染不消费）。会话级流程 `beginText/editActiveText/applyText/recolorText/beginText(in:)`（TypeTool.swift:67-196）在 Rust 无对应（app 只有 `canvas.rs:748-784` `create_text_layer` 单向建层）；`Rendering/InlineTextEditor.swift`（24KB 行内编辑器）无对应。
3. **笔刷参数校验与像素预算**：Swift `BrushStroke.swift:173-177`（diameter 1…2000、hardness 0…1、opacity 0.01…1、尺寸 1…1e9、源 ≤30k，否则 `ProjectError.tooLarge`）与 `EditorSession+Brush.swift:13-26` `makeRasterEdit` 的 `pixelLimit = 1e8 - 其他图层像素`（含蒙版像素二次扣减）；Rust `RasterEdit::new`（brush.rs:118-197）无任何校验/预算字段。
4. **笔刷平滑（Smoothing）**：Swift `EditorSession+Brush.swift:77-87` `smoothed`（绳长模型，屏幕点数/zoom）；Rust `brush_settings.smoothing` 仅存在于 UI 滑杆（`crates/app/src/state.rs:162,218`），笔画路径（`canvas.rs`）不消费。
5. **每工具笔尖记忆（parkedBrushTips）**：Swift `EditorSession.swift:219-223,332-353`（Clone Stamp 与 Smear 家族各自记忆 tip，初始 `(40,0,1)` 软笔尖）；Rust 无对应（全局一套 `brush_settings`）。
6. **笔刷快捷键**：Swift `EditorSession+Brush.swift:195-226` `typeOpacityDigit`（数字键不透明度、两位数组合）、`changeBrushHardness`（Shift-[/] 25% 步进）、`changeBrushSize`（`[`/`]` 1.2× 步进，至少 1px，上限 2000）；Rust `crates/app/src/keys.rs` 中这些全部无命中。
7. **蒙版绘画整体**：Rust app 的 `begin_stroke` 把 `is_mask` 硬编码为 `false`（`canvas.rs:459`）——Swift 的蒙版笔画/蒙版渐变（`EditorSession+Brush.swift:53`、`GradientTests.swift:126`）在应用层不可达（core 的 `RasterEdit` 支持 `is_mask` 且有测试 brush.rs:984-997）。
8. **ShapeTool 的 redrawShape / shapeTransformPreview / toggleShapeKind**：Swift `ShapeTool.swift:161-194`（缩放时按新尺寸重渲形状、圆角矩形拖拽中实时预览、Shift-U 轮换）；Rust 无对应。且 `session.rs:517-556` `add_shape_layer` 立即写入 asset，`render.rs:117-122` 的“无 asset 即实时重渲”路径在 app 流程中永不触发 → 形状图层缩放是重采样拉伸而非重绘（README 声称的 "re-rendered live" 仅对无 asset 图层成立）。
9. **拖拽修饰键**：Swift `ShapeTool.swift:84-100` `dragShape`（Option 中心放大、直线 45° 吸附）；Rust `canvas.rs:316-322` 只有 Shift 正方（`fromCenter` 恒 false，无角度吸附）。
10. **BrushCommit 后台提交 actor**：`BrushStroke.swift:844-902`（提交时一次 alpha 边界扫描裁剪、蒙版扩展 `expandMask`）无对应；Rust `paint_snapshot`（brush.rs:520-551）在提交线程同步完成。
11. **ToolDefaults**：`Document/ToolDefaults.swift:9-22`（UserDefaults 持久化工具开关）Rust 无对应物（grep 零命中）——多属 UI 开关域，列出备查。
12. **`BrushSettings.healingMode` 字段**：Swift `BrushStroke.swift:24` 内嵌于设置；Rust 改为 session 级 `spot_healing_mode`（`session.rs:46-51,142`）+ heal(mode) 参数——语义等价，仅结构差异（不算缺陷，顺带说明）。

## 四、【简化】（Rust 有对应物但做了取舍）

1. **分块瓦片引擎 → 整网缓冲**：Swift 256px tile（`BrushStroke.swift:89-91,144`，按需分配、tailBackup 按 tile 存图、`brush_alpha_bounds` 只扫已触 tile）；Rust 全网格单缓冲（brush.rs 模块注释 5-10 行自述"identical semantics, simpler bookkeeping"）。语义大体等价，但 `paint_snapshot` 的边界用整图 `alpha_bounds()`（brush.rs:524）vs Swift 逐 tile `brush_alpha_bounds`（C `BrushPixels.c:3-21`，`Rendering/BrushPixels.c`）——结果一致，性能不同。
2. **tip 渲染近似**：Swift 硬笔尖 = CG `fillEllipse` 解析抗锯齿、软笔尖 = 25 站 CG 径向渐变（`BrushStroke.swift:198-215`）；Rust 逐像素解析（`brush.rs:201-225`：硬边 `((1-d)*radius)` 线性 AA，软边直接算 falloff）。同公式不同采样，非 bit-exact。
3. **tip 无缓存**：Swift `stamp`（≤160px 预渲染，`BrushStroke.swift:107,193`）/`gridTip`（≤3000px、方形等比层 1:1 blit，:112-114,186-192）两级缓存；Rust `dab` 每次整张重算 tip（brush.rs:362）。纯性能，无数值差。
4. **clone 最近邻采样**：Swift 经 CG `.medium` 插值绘制 clone 图（`BrushStroke.swift:465`）；Rust floor+nearest（brush.rs:456-460）。
5. **move_lifted 采样与舍入**：Swift 整像素偏移 `.none`、分数偏移 `.high` 插值（`BrushStroke.swift:737`）；Rust 恒 `LayerSampling::Nearest`（brush.rs:724-729）；punch-out 通道用截断 `as u8`（brush.rs:719）而非四舍五入。
6. **liftSelection 取材**：Swift 从原始 `source` 抠出（`BrushStroke.swift:684-699`）；Rust 从合成结果 `result()` 抠出（brush.rs:681）。
7. **形状光栅化技术不同**：Swift CGPath fill/stroke（CG 抗锯齿，`ShapeTool.swift:197-223`）；Rust 选择覆盖 + `shrink_by`/`paint_distance_to_segment` 线性 AA（`render.rs:319-429`）。圆角矩形圆弧只有 4 段折线近似（render.rs:372，`FRAC_PI_2/4`），CG 是真圆弧。
8. **warp finish 的守卫与选区**：Swift `finishWarp` 校验图层未被并发修改、且经 `makeRasterEdit` 带选区剪裁（`SmudgeLiquify.swift:192-201`）；Rust `finish_warp_into_layer`（warp.rs:248-283）无版本守卫、`selection_clip: None` —— 变形结果不落在选区内时 Rust 会越界涂写。

## 五、【多出】（Rust 有、Swift 无）

1. `ShapeKind::RoundedRectangle` 作为独立种类（`document.rs:470-480`，serde 名 "Rounded Rectangle"）；Swift 只有 rectangle/ellipse/line 三种，圆角是 rectangle 的 `cornerRadius` 属性（`ShapeTool.swift:3-15`）。存档格式与 Swift manifest 的 kind 枚举不一致。
2. `LayerShape.filled` 字段 + 描边空心形状渲染（`document.rs:443-444`、`render.rs:346-350` `shrink_by`）；Swift 形状恒填充（仅 line 描边）。
3. `LayerShape.start/end` 为非 Optional 且有默认 `(0,0)→(1,1)`（`document.rs:447-455`）；Swift 是 `CGPoint?`（`ShapeTool.swift:29-30`，nil 表示 legacy 角对角内缩线）。渲染时 Rust 无 legacy 内缩回退（`render.rs:337-344` 恒用 start/end）。

## 六、测试缺口清单（Swift 用例 → Rust 对应）

Rust 侧现有相关内联测试：`brush.rs:784-1054`（tip_hard_and_soft、stroke_paints_foreground_with_opacity_cap、eraser_reduces_alpha、straight_line_between_far_points、fill_respects_selection、gradient_linear_from_black_to_white、gradient_radial_fades_from_center、clone_stamp_copies_from_offset、lift_and_move_pixels、paint_snapshot_crops_to_bounds、mask_stroke_paints_foreground_gray、healing_stroke_replaces_wash_with_real_inpaint、healing_without_paint_is_noop）、`heal.rs:432-509`（4 条）、`content_fill.rs:266-318`（3 条）、`warp.rs:294-388`（5 条）、`raster.rs:693-706`（falloff/spacing）、`render.rs:626-708`（shape 5 条）、`document.rs`（text 数据模型 2 条）。

**BrushTests.swift（19 用例）— 未移植 16**：
- 未移植：`bracketKeysReachTheBrushWhereverFocusIsExceptTextFields`(:38)、`shiftBracketsStepHardnessFromTheCanvas`(:91)、`continuousStrokeCrossesTilesAndCommitsOneUndo`(:119)、`softBrushProducesPartialAlphaAndCancelPreservesDocument`(:140)、`softMaskPaintingPreviewMatchesCommitAndPersists`(:160)、`brushStaysCircularOnNonuniformRotatedFlippedLayer`(:197，且 Rust 现实现会失败)、`maskedImagePaintingUsesCoverageAndOpacityOnlyOnce`(:221)、`importedImageLayerExpandsAcrossCanvasWithoutMovingImageOrMask`(:237)、`softStrokeBuildsCoverageWhileKeepingItsFeatheredRim`(:320)、`spacedDabsLeaveNoVisibleRippleAlongTheStroke`(:336)、`sparseMouseSamplesFollowACurveInsteadOfStraightChords`(:353)、`liveStrokeReachesNewestSampleAndTailIsReplacedExactly`(:371，tail 撤销缺失)、`opacityAppliesToMaskPainting`(:387)、`shiftBracketsStepHardnessByQuarters`(:405)、`numberKeysSetBrushAndGradientOpacity`(:420)、`foldersHiddenLayersAndDisabledMasksRejectPainting`(:448)。
- 部分对应：`paintedBoundsTrimTilePaddingAndKeepSoftEdges`(:275) ≈ `paint_snapshot_crops_to_bounds`；`opacityCapsTheWholeStrokeEvenWhereItOverlapsItself`(:306) ≈ `stroke_paints_foreground_with_opacity_cap`（无来回多遍重叠断言）。`largeBlankCanvasOnlyAllocatesTouchedTilesUntilCommit`(:439) 因无瓦片设计 N/A。

**BrushIntersectionTests.swift（4）— 全部未移植**：`selfCrossingsBlendInsteadOfTakingTheStrongestEdge`(:37)、`accumulationDependsOnDistanceNotEventCount`(:55，GPU 沉积模型缺失)、`softCrossingsRespectStrokeOpacityAndFlushIsIdempotent`(:69，含 flush 幂等 + alpha≤102 上限断言)、`exportCrossingExample`(:83)。

**BrushPerformanceTests.swift（1）— 未移植**：`fourKInteractiveStroke`(:8，依赖 Metal)。

**CloneStampTests.swift（2）— 全部未移植**：`copiesTheSourceUnderTheBrushKeepingAlignmentUntilItIsTurnedOff`(:23，aligned/unaligned 断言 Rust 现实现必失败)、`cloneStampKeepsItsOwnSoftBrushTip`(:63，parked tips 缺失)。Rust `clone_stamp_copies_from_offset` 只覆盖基本拷贝。

**SpotHealingTests.swift（1×3 模式）— 部分移植**：`healsTheBlemishUnderTheBrushAndNothingElse`(:36，参数化 ContentAware/CreateTexture/ProximityMatch) 引擎层有间接覆盖（`heal.rs:433` 基本修复、`heal.rs:487` mode1 颗粒），但**会话级“只动笔刷内像素”断言与 mode2（Proximity Match）无用例**；Rust 固定种子也无法对齐 Swift 随机种子。

**ShapeToolTests.swift（4）— 全部未移植**（session 级）：`rectangleFillsANewLayerWithTheForegroundColorAsOneUndoStep`(:32)、`ellipseLeavesItsCornersClearWithShiftCircleAndOptionFromCenter`(:54)、`aClickEscapeOrToolSwitchMakesNoLayer`(:67)、`roundedRectanglesFollowTheRadiusAndClampToAPill`(:84，Rust 有 clamp 逻辑 render.rs:368 但无 pill 断言测试)。core 的 `render.rs:626-708` 5 条 shape 测试只覆盖渲染函数。

**TypeToolTests.swift（8）— 全部未移植**：`createEditCancelAndUndo`(:14)、`transformsDuplicatesAndClippingKeepTextEditable`(:42)、`saveReopenAndRasterize`(:68)、`rasterHasTransparentBackgroundAndColoredGlyphs`(:92)、`clippingToTextExportsColoredGlyphsOnTransparency`(:106)、`paragraphBoxAndToolSwitchCommitEditableText`(:131)、`emptyNewParagraphIsDiscarded`(:147)、`invalidAndStaleDraftsDoNotChangeDocument`(:156)。Rust 仅有数据模型测试（`document.rs: text_style_defaults_and_validation`、`live_text_requires_matching_image`、`project/convert.rs: rasterized_text_drops_metadata_on_save`）。

**GradientTests.swift（8）— 2 部分 / 6 未移植**：部分对应 `foregroundToBackgroundFillsCanvasAndCommitsOneUndo`(:37)≈`gradient_linear_from_black_to_white`、`radialSpreadsFromStartToRimInEveryDirection`(:55)≈`gradient_radial_fades_from_center`。未移植：`reverseOpacityAndDirectionFollowSettings`(:69)、`foregroundToTransparentPreservesUnderlyingPixelsAndAlpha`(:83，Rust 现实现此功能是坏的)、`cancelUndoAndClicksLeaveDocumentUntouched`(:102)、`redraggingReplacesPendingLineWithoutAccumulating`(:116)、`maskGradientWritesCoverageInsideLayerBounds`(:126)、`paletteChangesUpdatePendingPreview`(:144)。

**SmartEditTests.swift（4）— 引擎部分覆盖**：`fillContinuesRepeatingTexture`(:6) ≈ `content_fill.rs: fills_selection_from_surroundings`+`noise_field_preserves_texture_statistics`（无 ≥114/120 纹理连续性断言）；`fillReconstructsBackgroundInsideSelectionAndUndoes`(:42)、`cancelAndNoSourceLeaveOriginalUntouched`(:60) 引擎层部分对应（`no_known_pixels_returns_none`），undo/cancel 流程无用例；`visionRequestRunsOnAnImage`(:72) N/A。

## 七、README 声称逐条裁定

| README（:33-35,43-44） | 裁定 |
|---|---|
| brush “centripetal Catmull-Rom, dab spacing, coverage caps, clone/erase/heal wash, gradients, fills, pixel moves” | **大体成立**（CPU 回退路径逐式一致），但 clone aligned、GPU 连续沉积、tail 精确撤销、smoothing、参数校验缺失；“gradients” 的前景→透明损坏 |
| heal “membrane solve + patch search, kernel-exact” | **不完全成立**：heal.rs:249 clamp 维度 bug、单行/单列 spot 拒修、修复区域范围不同，三处使结果可偏离 C |
| content_fill “coherent-offset patch synthesis, kernel-exact” | **成立**（逐常数一致） |
| warp “Smudge/Liquify + finishWarp” | **成立**（核心逐常数一致；finishWarp 缺选区剪裁与并发守卫） |
| ShapeTool “Rectangle/Rounded/Ellipse/Line 可编辑 LayerShape 实时重渲” | **部分成立**：渲染函数存在，但 app 创建即落 asset，实时重渲路径不可达；缺 redrawShape/修饰键/直线吸附 |
| TypeTool（README 表中并入 :18 "text style"） | 数据模型成立，**渲染/编辑流程不成立**（点阵字体占位） |
