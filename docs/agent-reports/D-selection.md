> 子代理原始报告（域 D：选区系统），逐字保留，2026-09-24。

# 选区系统深度对比报告（Swift Compositor vs Rust compositor-rs）

## 一、总体结论

**底层几何/像素内核移植质量高，会话层功能缺口大。**

- **逐算法等价（可放心的部分）**：`wand_mask` / `wand_trace`（flood fill + 轮廓追踪）与 `WandPixels.c` 逐行等价；羽化 σ=feather/2 属实；expand/contract 的 union/subtract 描边带语义属实；applySelection/selectAll/deselect/invert/feather 的组合与撤销语义一致。
- **README 有两条不准确**：① "antialiased scanline coverage" 实际是 4×4 超采样逐点判定，不是 scanline 解析覆盖；② "same hash/flood-fill/outline trace" 中的 "hash" 两边代码都不存在（引用色匹配是逐通道 ±tolerance 比较，无哈希）。
- **整个功能域缺失约一半**：SelectionEdits（填充/清除/反相/像素移动）、SelectionClipboard（复制/剪切/粘贴/Layer via Copy/Copy Merged）、FloatingSelection（Cmd-T 浮动变换）、MaskTracing（Cmd 点击载入蒙版/图层选区）、ObjectSelection（Vision 物体选择）、选区轮廓拖动/方向键微调的完整语义，Rust 侧全部没有实现。
- GuidedMatte 数学派生已移植（`adjust/pixel.rs`），但其唯一调用方 SubjectRemoval 在 Rust 是 TODO。

---

## 二、README 声称逐条验证

| README 声称 | 验证结果 |
|---|---|
| `selection.rs` 复刻 polygon boolean（geo 库） | **属实**。`crates/core/src/selection.rs:12` 引入 `geo::BooleanOps`；union/intersection/subtract 见 selection.rs:157-167，对应 `Compositor/Compositor/Document/Selection.swift:236-244` 的 `CGPath.intersection/union/subtracting(using: .winding)`。但引擎不同（geo/robust polygon vs CoreGraphics），输出顶点不保证逐位相同；且 Rust 自身的环→多边形重建有嵌套配洞缺陷（见行为差异 #4） |
| 抗锯齿 scanline coverage | **不属实（名不符实）**。`selection.rs:505-529 rasterize` 是每像素 4×4 规则网格超采样 + 偶奇射线判定（`const SS: usize = 4`，selection.rs:511），并非 scanline 解析覆盖率；Swift 侧是 `Selection.swift:19-22` 的 `CGContext.setShouldAntialias + fillPath(using: .winding)`（CoreGraphics 解析覆盖） |
| 羽化 = σ/2 高斯 | **属实**。Swift：`Selection.swift:27` `applyingGaussianBlur(sigma: feather / 2)`；Rust：`selection.rs:444` `gaussian_blur_gray(&mask, self.feather / 2.0)`，`raster.rs:592-621 gaussian_blur_gray` / `raster.rs:636-649 gaussian_kernel` 中该参数就是高斯 σ（`exp(-d²/2σ²)`，半径 `ceil(4σ)`）。核截断与 CI 工作色彩空间有细微数值差（见行为差异 #7） |
| 扩展/收缩 = round-join 描边带 | **结构属实、构造方式不同**。Swift：`Selection.swift:336 resizeSelection` 用 `path.copy(strokingWithWidth: abs(delta)*2, lineCap: .round, lineJoin: .round, miterLimit: 10)` 一次描边整条路径；Rust：`selection.rs:173-194 stroke_band` 逐环内外各偏移 `width/2` 后作差再并（`offset_ring` selection.rs:233-303，凸角圆弧 join）。语义等价，几何与退化情形有差（见行为差异 #5） |
| wand 复刻相同 hash/flood-fill/轮廓追踪 | **flood-fill 与轮廓追踪属实且逐行等价**；"hash" 不存在——两边都是逐通道 ±tolerance 比较（`WandPixels.c:9-15 wand_matches` vs `tools/wand.rs:13-21 wand_matches`） |

---

## 三、【缺失】（Rust 侧完全没有对应实现）

1. **SelectionEdits.swift 整体**（`Compositor/Compositor/Document/SelectionEdits.swift`）
   - `fillSelection(with:)`（:40）、`clearSelectedPixels()`（:54）、`deleteKeyPressed()`（:63）、`deleteLayerOrMask()`（:71）、`canInvert`（:82）、`invertPixels()`（:90）、`expandedUniformMask`（:129）→ session.rs 中无 `fill_selection` / `clear_selected_pixels` / `invert_pixels`（grep 全 crates 无命中）。Rust 只有 `tools/brush.rs:665-727` 的 `RasterEdit::lift_selection` / `move_lifted`（对应 `BrushStroke.swift:684 liftSelection` / `:703 moveLifted`），是底层半成品。
   - **像素移动（Cmd 拖动）**：`PixelMove` 类（SelectionEdits.swift:14-29）、`beginPixelMove`（:141）、`movePixels`（:155）、`displayedSelection`（:164）、`finishPixelMove`（:176）、`cancelPixelMove`（:187）、`nudgePixels`（:194）→ 全缺。session.rs:163 虽声明了 `selection_move_origin` 字段，但全仓库无任何使用（死字段）。

2. **SelectionClipboard.swift 整体**（`Compositor/Compositor/Document/SelectionClipboard.swift`）
   - `PixelClipboard`（:4）、`selectionCopyRegion()`（:15，含 0.001 容差取整）、`canCopyPixels`（:27）、`renderSelectedPixels`（:34）、`canCopyMerged`（:55）、`renderMergedPixels`/`copyMergedSelection`（:61/:80）、`copySelection`（:90）、`cutSelection`（:109）、`paste`（:123）、`layerViaCopy`（:138）→ crates/core 与 crates/app 均无（grep `clipboard|paste|copy_selection` 无命中）。仅 `duplicateActiveLayer`（:147）有对应 `session.rs:491 duplicate_active_layer`。

3. **FloatingSelection.swift 整体**（`Compositor/Compositor/Document/FloatingSelection.swift`）
   - `canTransformSelection`（:15）、`transformCommand`（:22）、`beginSelectionTransform`（:27）、`floatingSelectionTransform`（:59）、`mergeFloatingTransform`（:68）、`cancelFloatingTransform`（:110）、`FloatingMerge.merge`（:117-158）→ 全缺。Rust 仅存数据壳：`selection.rs:605-612 FloatingTransform` 结构体 + `transform.rs:228 TransformEdit.floating: Option<FloatingTransform>` 字段，且 `session.rs:1180 begin_transform` 永远置 `floating: None`，无任何代码置为 Some。

4. **MaskTracing.swift 整体**（`Compositor/Compositor/Document/MaskTracing.swift`）
   - `darkPixels`/`whitePixels`/`opaquePixels`/`trace`（:4-70）、`loadMaskSelection`（:76）、`loadLayerSelection`（:87）→ 全缺（grep `mask_tracing|dark_pixels|opaque_pixels` 无命中）。

5. **ObjectSelection.swift 整体**（`Compositor/Compositor/Document/ObjectSelection.swift`）
   - `ObjectSelectionSettings`（:5）、`select`/`selectAvailable`（:29/:38）、`instanceIndex`（:55）、`edgePreservedBinaryMask`（:67）、`adjusted/eroded/dilated`（:84-124）、`smoothed/simplifyClosed/simplifyOpen/chaikin`（:128-225）、`selectObject`（:273）→ 全缺。`tools/mod.rs:3` 注释声称移植了 `ObjectSelection.swift`，但只有注释。UI 层 `crates/app/src/canvas.rs:792-804` 与 `crates/app/src/tools.rs:343-361` 可以把 `wand_mode` 切到 `Object`，但点击处理 `crates/app/src/canvas.rs:239` 无条件调用 `magic_wand`——Object 模式是无效开关。

6. **选区轮廓移动**（`Selection.swift:257-290`）
   - `canMoveSelection`（:257）、`beginSelectionMove`（:263）、`moveSelection`（:272）、`endSelectionMove`（:279）→ 缺（见上，`selection_move_origin` 死字段）。

7. **光标/选项栏模式逻辑**（`Selection.swift:129-139`）
   - `lassoCursorMode`（:129）、`displayedSelectionMode`（:134）、`updateHeldSelectionKeys`/`heldSelectionMode`（:136）→ Rust session.rs 无对应（仅有 `selection_mode`，session.rs:662-670，等价于 Selection.swift:123）。

8. **GuidedMatte.refine 的调用链**
   - 算法本体已移植：`adjust/pixel.rs:152 guided_box` / `:187 guided_filter` / `:233 guided_refine`（对应 `GuidedMatte.swift:9 box` / `:45 filter` / `:100 refine`，He/Sun/Tang 公式逐项一致，epsilon 同为 1e-4）。但 Swift 唯一调用方 `SubjectRemoval.swift:51`（`GuidedMatte.refine`）在 Rust 是 `tools/subject_removal.rs:8` 的 `TODO(tools-pass): subject mask + guided matte refine`。即 `guided_refine` 只有测试调用，无生产调用。（`LayerEffects.swift:503-551` 用的是 `levels/image` 两个工具函数，Rust effects.rs 走了自己的 `shape_coverage` 路径，属 effects 域。）

9. **蚂蚁线动画与 displayedSelection 语义**
   - Swift：`Rendering/TransformOverlay.swift:196-210 drawSelection` 画"白色实线 + 黑色 [4,4] 虚线（相位 `antsPhase`）"，且读 `session.displayedSelection`（像素拖动/浮动变换时轮廓跟随）；`Rendering/EditorCanvas.swift:2001-2015 updateAntsTimer` 以 0.12s 步进 `antsPhase (mod 8)`。
   - Rust：`crates/app/src/canvas.rs:914-921` 画"黑色(0x000000aa)实线 + 白色(0xffffffff) [4,4] 虚线"，`canvas.rs:1101-1106 paint_polygon` 的 `dash_array` 无相位参数、无动画定时器；场景构建 `canvas.rs:125-130` 直接读 `document.selection`，无 `displayedSelection` 等价物（因像素拖动/浮动变换本身缺失）。

---

## 四、【行为差异】（两边都有实现，但结果不同）

1. **coverage_bounds 的 ceil 次序**
   - Swift：`Selection.swift:35` `insetBy(dx: -ceil(feather * 2))` → `ceil(feather×2)`。
   - Rust：`selection.rs:432` `let inset = self.feather.ceil() * 2.0;` → `(ceil(feather))×2`。
   - feather 为非整数时（`featherSelection` 的 `√(f²+a²)` 常产生非整数，如 3+3→4.2426）外扩不同：Swift ceil(8.485)=9，Rust 5×2=10。影响 `clip()` 的区域和复制范围。

2. **contains/rasterize 用偶奇规则，Swift 用 winding**
   - Swift：`Selection.swift:259` `path.contains(point, using: .winding)`；coverage 填充 `Selection.swift:22` `.winding`。
   - Rust：`selection.rs:134-155 SelectionPath::contains`、`selection.rs:544-562 point_in_path` 均为 even-odd（注释 selection.rs:502-504 自认"boolean 引擎不会产出重叠环所以等价"）。对自交多边形（自交 freehand 套索：Swift winding 填满全部环面，Rust 偶奇抵消交替瓣）和 8 字轮廓结果不同。

3. **椭圆几何**
   - Swift：`Selection.swift:218 finishLasso` 用 `outline.addEllipse(in:)`（4 段 Bézier）；预览 `TransformOverlay.swift` 同样 `addEllipse`。
   - Rust：`selection.rs:81-93 SelectionPath::ellipse` 用 96 段折线逼近（注释承认"CG 加 Bézier；光栅化在抗锯齿分辨率下等同"）。边界附近像素的覆盖值有细微差。

4. **环→多边形配洞按外接矩形，三层嵌套（岛中湖）配错父级**
   - `selection.rs:320-353 polygons_from_rings`：每个环挂到"第一个外接矩形包含它的已收集多边形"（selection.rs:339-346，`break` 在第一个命中）。对 A(外)⊃B(洞)⊃C(岛) 且输入顺序 A,B,C 时，C 会被挂为 A 的洞而不是 B 的洞，选择语义反转。CGPath 原生支持任意嵌套。该函数在每次对既有选区做 union/subtract/expand/contract（`to_geo`）时触发。

5. **expand/contract 描边带构造**
   - Swift：`Selection.swift:336` 对整条路径一次 `strokingWithWidth`（所有子路径统一描边，CG 语义）。
   - Rust：`selection.rs:173-194 stroke_band` 逐环 `offset_ring` 外偏 d 与内偏 d 作差再并；`offset_ring`（selection.rs:233-303）凸角圆弧步数上限 32（selection.rs:283，约 22.5°/段），环 `<3` 点或面积 `≈0` 直接跳过（selection.rs:177-183/239-241）。极小环上 Swift 会产生整块圆润描边带（contract 后清空），Rust 可能不产生带（contract 不收缩）。大圆角处的离散化也不同。

6. **coverage 光栅化近似**
   - Swift：`Selection.swift:19-22` CG 解析抗锯齿覆盖；关闭抗锯齿时 CG 自身硬边扫描转换。
   - Rust：`selection.rs:510-529` 4×4 超采样（16 级灰度）；`selection.rs:530-540` 非抗锯齿为像素中心点采样（x+0.5）。边缘灰度值与 CG 不逐位一致（选择编辑的边缘混合比例会有 ±1/16 级差）。

7. **羽化模糊实现**
   - Swift：`Selection.swift:27` `CIImage.clampedToExtent().applyingGaussianBlur(sigma:).cropped(to:)` + `PixelAdjust.render(isMask: true)`（PixelAdjust.swift:48-54，L8 输出）——Core Image 内部（工作色彩空间、自己的核截断）。
   - Rust：`selection.rs:444` → `raster.rs:592-621`（半径 `ceil(4σ)` 截断核，边缘 clamp——`raster.rs:623-634 clamp_mirror_index` 实为边缘复制）。
   - 两者 σ=feather/2 语义一致，但核形状/色彩空间差异使边缘灰度不逐位相同；对 `SelectionFeatherTests` 那种"≥4 个 8–247 之间像素"级别的断言等价，精确值不等。

8. **magic_wand 采样源与空白图层行为**
   - Swift：`MagicWand.swift:128-137 selectionSample`：空白图层/文件夹返回**全透明画布图**（wand 会对全画布命中；MagicWandTests.swift:101-103 明确断言 200 像素全选），且活动图层用 `displayedTransform(for:)`（EditorSession.swift:482-487，含变换预览）绘制。
   - Rust：`session.rs:1054-1064 selection_sample`：空白图层（`asset: None`，document.rs:548）返回 `None` → `magic_wand`（session.rs:998-1000）直接返回、**什么都不做**；图层用 `layer.transform`（session.rs:1061）绘制，忽略进行中的变换预览。

9. **magic_wand 守卫与错误上报**
   - Swift：`MagicWand.swift:99` 守卫 `canEditSelection, !isProjectBusy, selectionMoveOrigin == nil`；`MagicWand.swift:63/109` 把 `tooDetailed`（`WandPixels.c:7` 800 万边上限，wand_trace 返回 -2）写入 `brushError` 提示用户。
   - Rust：`session.rs:987-997` 只守卫文档与点范围；`session.rs:1037` `Err(_) => return`——超详细轮廓**静默失败**，用户无感知。`tools/wand.rs:123/235-238 TraceError` 只保留 TooManyEdges，C 的 OOM 错误类别（-1）被折叠。

10. **begin_lasso / can_modify_selection / nudge_selection 守卫削弱**
    - Swift `Selection.swift:143`：`guard tool.isSelectionTool, tool != .wand, canEditSelection, selectionMoveOrigin == nil`；Rust `session.rs:672-675` 只查工具种类，无 `canEditSelection`/`selectionMoveOrigin` 检查。
    - Swift `Selection.swift:293`：`selection?.isEmpty == false && canEditSelection && lassoDraft == nil`；Rust `session.rs:898-900 can_modify_selection` 只查非空——**画套索草稿期间也能 Expand/Contract/Feather**。
    - Swift `Selection.swift:272-277 moveSelection`：偏移取整（`offset.width.rounded()`）且从 `selectionMoveOrigin` 出发；Rust `session.rs:885-896 nudge_selection` 用 `translated(dx, dy)` 不取整、无 `selectionMoveOrigin`/`canEditSelection`/空选区检查（空选区也能"移动"）。
    - Swift `Selection.swift:157/166`：`dragMarquee`/`extendLasso` 有 `point.x.isFinite` 守卫；Rust `session.rs:697-725 drag_marquee`/`extend_lasso` 无 isFinite 检查（NaN 点可进入草稿）。

11. **蚂蚁线颜色相反且无动画**
    - Swift：白实线在下 + 黑虚线（相位动画）在上（TransformOverlay.swift:204-209）。
    - Rust：黑实线在下 + 白虚线在上，无相位（canvas.rs:917-918, 1105）。视觉风格不同且静止。

---

## 五、【简化】（语义近似、刻意降级）

1. **rasterize 超采样代替解析覆盖**：`selection.rs:511`（16 级灰度 vs CG 连续覆盖）——即 README "scanline coverage" 的实情。
2. **offset_ring 的圆角离散化**：`selection.rs:283` 每凸角 `ceil(|Δ|/(π/8))` 步、上限 32 段，近似 CGPath stroking 的圆弧。
3. **guided_refine 重采样链**：Swift `GuidedMatte.swift:69-79/110-115` 用 CGContext/CI 一次 `.high` 插值缩放；Rust `adjust/pixel.rs:257-281 resample_bitmap_gray` 用"逐步 halve + LayerSampling::High"近似（注释自认"as `.high` does"）。
4. **wand OOM 错误类别折叠**：C `WandPixels.c:46/70/117` 的 malloc/realloc 失败链（-1）在 `tools/wand.rs` 中不存在（Vec 直接分配）。
5. **floating 变换只留数据结构**：`selection.rs:605` + `transform.rs:228` 存在但无 begin/merge 流程（也列入缺失 #3）。

## 六、【多出】

Rust 侧没有功能级多出；仅实现层附加：
- `selection.rs:196-214 SelectionPath::translated/transformed`（对应 CGPath `copy(using:)` 的显式方法化，等价）；
- `selection.rs:484-499 SelectionClip::alpha_at`（把 Swift `Selection.swift:58-65 SelectionClip.apply` 的 CGContext mask-clip 改为逐像素最近邻查询，供 `adjust/pixel.rs:10 coverage`/`tools/brush.rs:690` 使用；语义等价）；
- `raster.rs:550-589 gaussian_blur_rgba`（Swift 无对应单点函数，供 effects 用）。

---

## 七、测试缺口清单（Swift 用例 → Rust 对应）

Rust 侧相关内联测试位置：`crates/core/src/selection.rs:614-760`（14 个）、`crates/core/src/tools/wand.rs:240-340`（7 个）、`crates/core/src/session.rs:1592-1960`（选区相关 6 个）、`crates/core/src/adjust/pixel.rs:283-415`（选区/引导相关 8 个）、`crates/core/src/tools/brush.rs:854/946`、`crates/core/src/tools/content_fill.rs:267`。

**SelectionTests.swift（23 用例）**
| Swift 用例 | Rust 对应 |
|---|---|
| replaceAddAndSubtractCombineOutlines (:32) | 部分覆盖：session.rs:1705 `marquee_selects_and_composes` + selection.rs:631/643（纯几何层；无 coverage 级组合断言） |
| modifiersPickModeAndSelectionIsClippedToCanvas (:44) | 大部分缺：`selection_mode` 无测试；画布裁剪仅 selection.rs:699 `clip_region_bounded_by_canvas` |
| emptySelectionIsDistinctFromNoSelection (:55) | 部分：selection.rs:707 `empty_selection_clips_everything`、:643 `subtract_can_leave_hole`；session 级"空选区上 subtract 不动"缺 |
| clickDeselectsAndSelectionStepsUndo (:68) | 部分：session.rs:1727 `lasso_click_enclosing_nothing_deselects`；undo 链断言缺 |
| polygonalCornersCanBeRemovedAndClosed (:83) | **缺**（`remove_last_lasso_point` 有实现无测试） |
| antialiasingControlsEdgeCoverage (:101) | 部分：selection.rs:673 `rasterize_antialiased_edge_partial`；antialiased=false 硬边无测试 |
| selectAllInverseAndToolSwitchCancelsDraft (:114) | 部分：session.rs:1761 `invert_selection`；selectAll/切换工具取消草稿缺 |
| cursorBadgeFollowsModifiersButKeepsAnOutlinesStartingMode (:126) | **缺**（功能本身缺） |
| draggingMovesTheOutlineInWholePixelsAsOneUndo (:147) | **缺**（功能缺） |
| movingOffCanvasAndBackKeepsTheWholeShape (:164) | **缺**（功能缺） |
| arrowNudgesAndMoveIsOnlyForNewModeOnARealSelection (:177) | **缺**（`nudge_selection` 有实现无测试） |
| expandAndContractGrowAndShrinkTheOutline (:194) | 部分：session.rs:1737 `expand_contract_and_feather` + selection.rs:715/724 |
| expandStaysOnCanvasAndContractCanEmptyTheSelection (:211) | 部分：扩到画布裁剪在 session.rs:917-925 有实现；contract 到空选区无测试 |
| cmdClickingAMaskSelectsItsBlackAreas (:240) | **缺**（MaskTracing 缺） |
| maskSelectionFollowsTheLayerTransformAndIgnoresAllWhiteMasks (:257) | **缺** |
| cmdClickingALayerSelectsItsOpaquePixels (:273) | **缺** |
| marqueeDrawsWholePixelRectanglesInAnyDirection (:309) | 部分：session.rs:1705（反向拖动、含 0.5 取整边界未测） |
| marqueeShiftMakesSquaresAndCenteredDragsGrowFromTheAnchor (:324) | 部分：selection.rs:734 `drag_box_square_and_center`（session 级未测） |
| marqueeEllipseSelectsAnOvalInItsBoxAndShiftMakesACircle (:338) | 部分：selection.rs:745 `ellipse_contains_center_not_corners`；session 椭圆流程缺 |
| optionDraggingTheMarqueeSubtractsWithoutDrawingFromTheCenter (:359) | **缺**（CanvasView UI 级） |
| mKeyChoosesTheMarqueeAndKeepsTheShapeLastSet (:390) | **缺**（app 层按键；`toggle_marquee_kind` 无测试） |
| shiftStartsAnAddAndOnlyAFreshShiftSquaresTheMarquee (:410) | **缺**（UI 级） |
| lKeyChoosesTheLassoAndKeepsTheModeLastSet (:451) | **缺** |

**SelectionEditTests.swift（16 用例）**
| Swift 用例 | Rust 对应 |
|---|---|
| brushPaintsOnlyInsideTheSelection (:30) | 部分：机制由 brush.rs `selection_clip`+`fill_respects_selection`(:854) 覆盖，无画笔端到端 |
| emptySelectionEditsNothing (:43) | 部分：pixel.rs:389 `empty_selection_zeroes_coverage`；session 级（fill/clear/gradient 全被挡）缺 |
| gradientStaysInsideTheSelection (:64) | **缺**（clip 调用存在于 app/canvas.rs:456/696，无测试） |
| fillUsesPaletteInsideSelectionOrWholeLayerWithoutOne (:77) | **缺**（`fillSelection` 缺） |
| deleteClearsSelectedPixelsOrDeletesTheLayerWithoutASelection (:94) | **缺** |
| maskFillHidesOnlyTheSelectedArea (:111) | **缺** |
| clipFollowsScaledLayersAndSoftensEdges (:129) | 部分：pixel.rs:295 `coverage_on_image_grid` + :305 `blend_through_selection`；缩放图层+软边端到端缺 |
| maskButtonAddsWhiteMaskOrHidesTheSelection (:157) | **缺**（选区建蒙版缺） |
| layerMenuMasksUseTheSelection (:179) | **缺** |
| maskFromSelectionLinesUpOnScaledLayers (:199) | **缺** |
| deletingWithTheMaskTargetedRemovesOnlyTheMask (:218) | 部分：session.rs:1790 `add_and_delete_mask`（无 Delete 键路由） |
| cmdDragMovesSelectedPixelsAndOutlineAsOneUndo (:249) | **缺**（PixelMove 缺） |
| duplicatePixelDragPreservesSourceAndUndoesTogether (:270) | 部分：brush.rs:946 `lift_and_move_pixels`（RasterEdit 层，无撤销/大纲联动） |
| cmdArrowNudgesPixelsAndMasksRefuse (:289) | **缺** |
| invertKeepsTransparencyStaysInSelectionAndWorksOnMasks (:302) | 部分：pixel.rs:321/334/342（`invert_pixels` 会话入口缺） |
| pixelMoveNeverShowsTheOutlineAtItsOldSpot (:331) | **缺** |
| invertHandlesLargeImagesAndUniformMasksWithASelection (:352) | 部分（1×1 uniform 蒙版扩展 `expandedUniformMask` 缺） |
| quickOperationsNeverDimTheInterface (:372) | **缺**（UI busy 域） |
| invertWorksInEveryTool (:393) | **缺** |

**SelectionFeatherTests.swift（1 用例）**
- featherSoftensTheSelectionAndWhatItClips (:7) → 部分：selection.rs:683 `coverage_feather_softens`；端到端"经羽化 clip 填充出软边"（:32-45）**缺**。

**SelectionClipboardTests.swift（9 用例）— 全部缺**
- copyAndPastePutsPixelsOnANewLayerInPlace (:48)、cutLeavesAHoleAndPasteRestoresThePixels (:68)、layerViaCopyCopiesTheSelectionOrDuplicatesTheLayer (:78)、transformSelectionMovesPixelsAndOutlineAsOneUndo (:92)、escapeRestoresExactlyWithoutAnUndoStep (:117)、applyingAnUnchangedTransformLeavesSoftEdgesUntouched (:131)、scalingAndMovingPastTheLayerEdgeGrowsTheLayer (:143)、lassoShapedSelectionCopiesAndPastes (:162)、copyMergedTakesEveryVisibleLayerNotJustTheActiveOne (:178) → 剪贴板与浮动变换功能均缺，无任何对应 `#[test]`。

**MagicWandTests.swift（6 用例）**
| Swift 用例 | Rust 对应 |
|---|---|
| contiguousStopsAtOtherColorsWhileNonContiguousFindsEveryMatch (:43) | 部分：wand.rs:255/264（轮廓像素还原、行序断言缺） |
| toleranceAppliesToEveryChannelIncludingAlpha (:58) | 部分：wand.rs:264（alpha 通道容差未测） |
| sampleSizeAveragesThePixelsAroundTheClick (:70) | 部分：wand.rs:278 `wand_seed_radius_averages` |
| outlinesReproduceTheirPixelsWithHolesAndCornerTouches (:80) | 部分：wand.rs:305/319（洞 + 对角单点触碰场景未测） |
| theWandReadsTheActiveLayerOrEveryVisibleLayerAndCombinesModes (:92) | 部分：session.rs:1774 `magic_wand_selects_uniform_region`；sample_all_layers/add/subtract 组合缺；且"空白图层全选"在 Rust 行为不同（见行为差异 #8），该断言**不可直接移植** |
| clickingInsideASelectionMakesANewWandSelectionRatherThanDeselecting (:117) | **缺**（UI 级） |

**GroupingSelectionTests.swift（3 用例）— 全部缺**
- singleLayerAndFolderAreWrappedRatherThanCreatingAChildFolder (:6)、multipleSelectionPreservesOrderAndSelectedFolderDescendants (:23)、itemsFromDifferentFoldersUseCommonParentAndEmptySelectionCreatesEmptyGroup (:49) → Rust 无 `groupSelectedLayers`/`addGroup` 会话 API（`is_group` 字段与 folder 渲染存在，crates/core/src/document.rs:509；成组功能未实现）。

---

### 附：核对说明
所有行号均经 `grep -n` 或 Read 逐条核实；关键差异点复核来源：selection.rs:432 vs Selection.swift:35（ceil 次序）、session.rs:898-900 vs Selection.swift:293（守卫）、session.rs:1037 vs MagicWand.swift:63（错误上报）、session.rs:1054-1064 + document.rs:548 vs MagicWand.swift:128-137（空白图层采样）、canvas.rs:914-921/1101-1106 vs TransformOverlay.swift:196-210 + EditorCanvas.swift:2001-2015（蚂蚁线）、transform.rs:228 + session.rs:1180 vs FloatingSelection.swift（浮动变换）、tools/subject_removal.rs:8 vs SubjectRemoval.swift:51（guided refine 调用链）。
