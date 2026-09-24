> 子代理原始报告（域 B：图层变换 / 几何 / 画布尺寸 / 项目文件格式），逐字保留，2026-09-24。

# 图层变换 / 几何 / 画布尺寸 / 项目文件格式 深度对比报告

## 总体结论

1. **transform.rs / geometry.rs 是高质量的 1:1 移植**：`point/contains/placing/following/rounded`、`TransformDrag.updated` 的镜像翻转与比例锁定投影、`TransformSnap.shift` 的“最小移动”判定、`CGAffineTransform.concat` 的 CoreGraphics 语义（`p' = M_self·M_other·p`，fix3.py 修过的公式现已正确）全部逐式核对一致。
2. **canvas.rs 两极分化**：`CanvasSizeDraft/Options` 单位换算、anchor floor 公式、`CropGeometry/CropDrag`、`ImageTrim` 扫描算法、`LayerFlip.mirrored` 数学全部一致；但 **`CanvasResizer` 的 fill（"Canvas Extension" 图层）整套缺失**，**`ImageResizer` 是明显简化版**（旋转层结果不同、1×1 均匀 mask 被错误放大、全部预算/校验缺失）。
3. **三大块整体缺失**：`Distort.swift`（DistortWarp/homography/warp 及 session distort 流程）完全没有移植（只有 `TransformDrag::corners_at` 的拖角数学）；`CropSnap` 与 EditorSession 的对齐吸附（`alignmentSnapTargets/snappedMove`）缺失（`TransformSnap` 已移植但无人调用）；参考线的拖拽/标尺交互缺失（仅数据模型已移植）。`flip_canvas` 未镜像选区，`flip_layers` 不支持多选/组翻转。
4. **project/ 移植质量很高**：manifest v1–9 全部版本规则、校验上限、层级/live-mask 图校验、原子保存、越界文件检查逐一对应（README 写 "v1–8" 是过时描述，实际两边都是 v1–9）。原子保存比 Swift 弱一档（remove+rename 有丢失窗口）；加载时缩略图、多帧 PNG 拒收等有细微差异；无崩溃恢复（Swift 也没有），staging 目录崩溃后不清理。

---

## 一、【缺失】

1. **DistortWarp 全套** — Swift `D:\Project\rust\compositor-rs\Compositor\Compositor\Document\Distort.swift:8-238`（`corners/isUsable/isConvex/affine/homography/imageCorners/warp/warpFolded/warpTrimmed/carried/warpMask/mapPath`）及 session 侧 `beginDistort/previewCorners/distortTarget/distortedEffects/distortPreview/commitDistort/distort(at:)`（Distort.swift:260-392）在 Rust 无任何对应（全仓 grep `DistortWarp|homography|is_convex` 无命中；`crates\core\src\tools\warp.rs` 是 SmudgeLiquify 的移植，非 Distort）。`crates\core\src\transform.rs:231` 的 `TransformEdit.corners` 字段只在 `crates\core\src\session.rs:1181` 被置为 `None`，从未接线。
2. **CropSnap（裁剪边吸附）** — Swift `Compositor\Compositor\Document\Crop.swift:49-112`（`CropSnap.apply`：move 吸附双边、resize 只吸被拖边、ratio 时只吸 move、symmetric 镜像半宽）Rust 无对应；`crates\core\src\session.rs:1457-1463` 的 `drag_crop` 不做任何吸附。
3. **变换/裁剪的对齐吸附目标** — Swift `Crop.swift:116-142`（`transformSnapTargets/snappedMove/cropSnapTargets`）与 `Guides.swift:182-220`（`alignmentSnapTargets`）无 Rust 对应。`crates\core\src\transform.rs:402-435` 的 `TransformSnap::offset/shift` 已移植但全仓无调用点。
4. **flip_canvas 不镜像选区** — Swift `Compositor\Compositor\Document\LayerFlip.swift:69-77` 用仿射镜像 `selection.path`；Rust `crates\core\src\session.rs:1266-1281` 只翻转图层、mask placement、guides，`doc.selection` 原样不动。
5. **flip_layers 不支持多选/组翻转** — Swift `LayerFlip.swift:24-51` 支持 `transformsAsGroup` 时对组内全部成员绕包围盒中心翻转；Rust `crates\core\src\session.rs:1219-1246` 仅翻转单个 active layer（也无 `TransformGroup` 的使用方）。
6. **Canvas Size 的 fill（"Canvas Extension"）** — Swift `Compositor\Compositor\IO\CanvasResizer.swift:36-69`：扩展时在底部插入有色图层，含像素预算（`100_000_000 - used`，38 行）、图层数预算（39 行）、颜色分量校验（40-42 行）；Rust `crates\core\src\canvas.rs:142-158` 完全忽略 `options.fill`（`CanvasExtensionColor` 定义于 canvas.rs:96-101 但无使用者），app 侧 `crates\app\src\sheets_ui.rs:695` 也不传 fill。
7. **CanvasResizer 的全部防御校验与早退** — Swift `CanvasResizer.swift:8-15`：宽高 1...30_000、anchor 0...8、offset 有限且 ≤1e6、同尺寸且零 offset 时原样返回、逐层 `transform.isValid`（23 行）；Rust `canvas.rs:142-158` 一律重建并无任何校验（同尺寸时还会清空 selection，行为不同）。
8. **参考线交互** — Swift `Guides.swift:91-179`（`hitGuide/beginGuideCreation/beginGuideMove/moveGuideDrag/finishGuideDrag/cancelGuideDrag`）与 `snappedGuidePosition`（222-249）、`guideHitDistance`（69）无 Rust 对应；Rust `session.rs:1409-1431` 只有 `clear_guides/add_guide`。
9. **崩溃恢复**：两边都没有（Swift 全仓 grep `autosave|recovery|crash` 无命中），不算 Rust 缺口，但确认“未移植=原版也没有”。

## 二、【行为差异】

1. **ImageResizer 旋转层结果不同** — Swift `Compositor\Compositor\IO\ImageResizer.swift:36-43`：对每层取 4 角经 transform 投影再乘 (sx,sy) 后的包围盒（floor/ceil），**输出 transform 的 rotation/flip 归零**（像素已烘焙；`ImageSizeTests.swift:56` 断言 `rotation == 0`）；Rust `crates\core\src\canvas.rs:179-182` 对 `origin/size` 做线性缩放并保留 rotation/flip——旋转层（如 90°、非均匀缩放）的尺寸、原点与 Swift 不一致。
2. **ImageResizer 的 1×1 均匀 mask 被放大** — Swift `ImageResizer.swift:77`：`width==1 && height==1` 或有 `maskPlacement` 的 mask 原样保留（分辨率无关）；Rust `canvas.rs:191-197` 无条件 `resample` 到 `round(w*sx).max(1)`，1×1 mask 在 2× 缩放后变 2×2。有 placement 的 mask 的 placement 缩放：Swift `ImageResizer.swift:97` 用 `placing(unitToDocument ∘ scale)`，Rust `canvas.rs:198-201` 线性缩放 origin/size，旋转 placement 结果不同。
3. **原子保存强度不同** — Swift `Compositor\Compositor\IO\ProjectStore.swift:111-118`：`NSFileCoordinator(.forReplacing)` + `FileWrapper.write(.atomic)`，Foundation 先写兄弟暂存包再原子替换，旧包在交换前始终完好；Rust `crates\core\src\project\store.rs:512-528`（`write_package`）是 **先 `remove_dir_all` 旧包再 `rename` 暂存包**，两步之间崩溃会丢包（注释自认 "best-effort"）。失败路径的清理（522-527 行删 staging）有对应。
4. **加载资产时缩略图策略** — Swift `ProjectStore.swift:166-170` 用 `CGImageSourceCreateThumbnailAtIndex(maxPixelSize:96)`；Rust `store.rs:297-332` 的 `read_png_asset` 不产缩略图，缩略图延迟到 `convert.rs:78/88` 经 `ImportedImage::new` → `raster.rs:215-222 thumbnail_of`（同为 96px 上限，实现路径不同，结果等价）。
5. **PNG 严格性** — Swift `ProjectStore.swift:155-161` 要求 `CGImageSourceGetCount == 1`（拒多帧 APNG）且逐属性查 depth≤8；Rust `store.rs:299-307` 用 `PngDecoder` 只拒 16-bit 色型，多帧 PNG 会按首帧接受。
6. **`resize_image_to` 缺校验** — Swift `ImageResizer.swift:15-16,25`：新尺寸 1...30_000、resolution 有限且 1...9600、`width*height ≤ 100_000_000` 总预算；Rust `session.rs:1514-1523` 只查尺寸 1..=30_000，resolution 与总像素预算不校验；且 Swift `ImageResizer.swift:21-24` 同尺寸时只改 resolution 提前返回，Rust 无此捷径（结果等价但 `options.sampling` 见下条）。
7. **Image Size 无 sampling 选项** — Swift `ImageResizer.swift:4-9` `ImageSizeOptions.sampling`（测试用 `.nearest`）；Rust `canvas.rs:187,195` 硬编码 `LayerSampling::High`，API 层面无此参数。
8. **LayerSampling 的 serde 默认值不一致（内部隐患）** — Swift `LayerTransform.swift:24` 字段默认 `.high`；`crates\core\src\manifest.rs:206-208` 正确用 `default_sampling()=High`，但 `crates\core\src\transform.rs:10-11` 枚举级 `#[default] Smooth`，导致 `LayerTransform` 自身的 `#[serde(default)]`（transform.rs:40-41）在缺 key 时解码为 Smooth——若 CanvasDocument 的 serde 路径被用于持久化会与 Swift 语义分叉。（复核注：manifest 路径实为 `crates/core/src/project/manifest.rs:202-207`。）
9. **错误类型映射** — Swift `ProjectStore.swift:249-255` `checkFile` 一律抛 `.tooLarge`；Rust `store.rs:268-284` 区分 `Invalid/TooLarge/MissingImage`（manifest 缺失 → Invalid，资产缺失 → MissingImage），语义更细但与 Swift 的 NSCocoaError 泛错不同。

## 三、【简化】

1. **TransformGroup.originals 用 `Vec<(Uuid, LayerTransform)>`**（`crates\core\src\transform.rs:212`）代替 Swift 字典（LayerTransform.swift:105），查找 O(n) 但保序——语义等价的简化。
2. **trim 的渲染入口**：Swift `ImageTrim.swift:179` 经 `ImageExporter.render(snapshot)`（含 effects 的成品合成）；Rust `canvas.rs:438` 用 `draw_live_composite(document)`，效果是否含 effects 取决于 render.rs 实现（本域内数学一致）。
3. **save 的 PNG 编码**：Swift `ProjectStore.swift:89-98` 用 ImageIO/CGImageDestination；Rust `store.rs:357-380` 用 image crate + 手写 premultiply/unpremultiply（336-355 行），舍入方式自洽但非逐位等价。
4. **Swift `ProjectStore.save` 的 `LayerMask.isValid` 检查**（ProjectStore.swift:86）在 Rust `store.rs:464-466` 以“GrayMask 按构造必为 8-bit 灰度”为由省略——合理等效。

## 四、【多出】

1. `crates\core\src\transform.rs:197-204` `LayerTransform::corners()`（即 `DistortWarp.corners(of:)` 的公式，Swift 里它属 Distort.swift:10-12）——公式正确但当前无人调用。
2. `crates\core\src\transform.rs:58-67` 顶层 `HANDLES` 常量与 `DragMode/TransformDrag` 独立于 LayerTransform（Swift 中 handles 是 `LayerTransform.handles` 静态属性，LayerTransform.swift:96-98）——组织形式差异，值完全一致。
3. `crates\core\src\canvas.rs:260-269` 顶层 `standardize()`（对应 `CGRect.standardized`），Rust geometry.rs 未提供通用 standardized。

## 五、manifest 版本兼容逐项核对（v1–v9）

Swift `ProjectStore.swift:180-240` vs Rust `store.rs:38-247`——**全部对应，无遗漏**：

| 规则（Swift 位置） | Rust 位置 | 结论 |
|---|---|---|
| format == "com.compositor.project"（181） | store.rs:39 | ✓ |
| version ∈ 1...9（182；README 写 v1–8 过时） | store.rs:42（MIN/MAX=1/9，manifest.rs:31-32） | ✓ |
| colorSpace=="sRGB"（183） | store.rs:45 | ✓ |
| resolution 有限 ∈1...9600（184-186） | store.rs:48-52 | ✓ |
| 宽高 ∈1...30_000、layers ≤10_000（187-188） | store.rs:53-58 | ✓ |
| text：isValid+有图+非组+无 adjustment（190-192） | store.rs:60-68 | ✓ |
| adjustment ≥v7、非组、无图、isValid；blur/noise ≥v9（193-198） | store.rs:69-86 | ✓ |
| mask ≥v4/组≥v6 且文件名==`<id>.mask.png`；maskEnabled⇒有 maskFile；maskPlacement isValid⇒有 maskFile（199-203） | store.rs:88-101 | ✓ |
| opacity∈0...1；v3 前必须 opacity==1 且 Normal；组 Normal 且（≥v8 或 opacity==1）（204-210） | store.rs:102-117 | ✓ |
| LayerHierarchy.validate（64 层、组无图、父须组、环）（LayerGroups.swift:30-46） | store.rs:160-190 | ✓ |
| LiveMaskGraph.validate（链 <256、源存在非组无 adjustment、无环）（LiveLayerMask.swift:4-19） | store.rs:194-224 | ✓ |
| v5 前 maskSourceID、v1 前 parentID/isGroup 非法（214-215） | store.rs:121-131 | ✓ |
| id 唯一、transform.isValid、name 非空白且 UTF-8 ≤16_384、imageFile==`<id>.png`（216-222） | store.rs:132-148 | ✓ |
| activeLayerID ∈ ids（223） | store.rs:149-153 | ✓ |
| guides：v8 前必须空；≤1000、id 唯一、position 有限且 ≤1e6（227-240） | store.rs:227-247 | ✓ |
| checkSize 1...30_000 每边、100M 累计（图/掩码独立预算）（242-247） | store.rs:251-262 | ✓ |
| manifest ≤4MiB（104/135）；资产 ≤512MiB（153） | store.rs:29-30, 389, 421, 484 | ✓ |

ProjectController.swift（保存/打开 UI 流、`open` 的“确认保存后重载同一文件” 212-215 行、文件拖放队列）属 AppKit 壳层，Rust 侧由 `crates\app` 承担，核心层无需对应。

## 六、测试缺口清单（Swift 用例 → Rust 对应）

**已对应**：ProjectTests 前 6 例（store.rs `project_round_trip_survives_package_move`/`a_dimmed_folder_saves_and_reopens`/`overwrite_replaces_package_and_drops_removed_assets`/`failed_save_preserves_previously_saved_package`/`unsupported_corrupt_and_unsafe_metadata_are_rejected`/`missing_embedded_image_is_rejected`）；`projectRoundTripPreservesAllLayerEffects` → convert.rs `round_trip_preserves_all_layer_styling`；TransformTests `scalePercentSetsBothSidesAboutTheCenter` → transform.rs `scale_keeps_center`；ImageTrimTests `calculateTrimRectTransparentPixels` → canvas.rs `trim_transparent_pixels`；GuideTests `projectRoundTripAndLegacyRejection` → store.rs `guide_checks` + `guides_round_trip`；`layoutGridLinesIncludeMajorsAndSubdivisions` → document.rs `layout_grid_lines`。

**未移植（无任何 Rust #[test] 对应）**：
- TransformTests（13 例中缺 9）：`duplicateTransformPreservesOriginalAndSupportsUndoAndCancel`、`autoSelectCanBeDisabledAndCommandClickOverridesIt`、`autoSelectPicksForegroundLayerStackedOnSelectedBackground`、`hoverRegionsMatchRotatedEdgesCornersAndRotationHandle`、`blankLayersHaveNoTransformHandlesOrEditing`、`rotatedResizeKeepsOppositeAnchorAtEveryHandle`、`documentMappingAndRotatedHitTesting`、`noOpInvalidValuesAndSwitchingTools`、`renderingRotatesScalesAndFlipsWithoutReplacingPixels`（`moveRotateAndShiftConstraints`/`previewCommitCancelAndUndoPreserveSources`/`layerListToolKeysAndTransformNudge` 仅被 `resize_drag_ratio_lock`/`rotate_snap_to_15_with_shift`/`transform_moves_layer_as_one_edit`/`nudge_layer_commits_immediately` 部分覆盖）。
- TransformPressTests：`draggingOutsideTheLayerMovesIt`、`optionDraggingOutsideTheLayerDuplicatesIt`（AppKit 命中测试，引擎层也无等价用例）。
- MaskTransformTests（5 例全部无端到端对应）：`aLinkedMaskMovesWithItsLayerWhicheverThumbnailIsSelected`、`unlinkedTheLayerMovesAloneAndItsMaskStaysOnTheCanvas`、`unlinkedTheMaskMovesAloneAndRelinkedTheyMoveTogether`、`aMovedHideAllMaskKeepsHidingPastItsPixels`、`placementAndLinkAreSavedAndPaintingFollowsThePlacement`（仅 document.rs `layer_mask_placement_logic`/`uniform_mask_has_no_placement` 覆盖 `placement_moving_layer` 公式，painting-follows-placement 与保存后的绘制行为无用例）。
- CropTests：`sameSizeOffsetCropAndExpansionUseExactBounds`、`cropEdgesSnapToNearbyEdges`（CropSnap 缺失）、`snapTargetsAreTheCanvasAndLayerBounds`、`cropDraggingRedrawsOnlyTheOverlay`（`dragGeometrySupportsReverseRatioMoveAndEveryHandle` 仅被 `crop_geometry_create_and_snap`/`crop_shifts_content` 部分覆盖，无逐 handle 遍历、无 undo 断言）。
- CanvasSizeTests：`coloredExtensionPreservesOldTransparencyAndRoundTripsWithUndo`（fill 缺失）、`transparentResizeIsAllocationFreeAndShrinkDoesNotAddFill`；`everyAnchorPreservesSourceAndTransformForExpansionAndShrink` 仅被 `anchor_offsets`（3 个采样点）部分覆盖，无 9 anchors × expand/shrink 的端到端；`relativeRatioAndUnitsUseFinalDimensions` 被 `draft_units_roundtrip`/`locked_axis_scales_other_side` 部分覆盖（缺 centimeters 换算与 invalid(0) 断言）。
- ImageSizeTests：`rotatedHiddenLayerScalesInDocumentAxesAndInvalidSizeIsRejected` 完全未移植（且 Rust 当前行为本就不同，见差异 1/6）；`resizePreservesLayerIdentityAndUndoRestoresSource`、`resolutionOnlyRetainsPixelsAndSurvivesSaveAndExport` 仅被 `resize_image_resamples_layers`/`canvas_and_image_size` 部分覆盖（无身份/undo/同尺寸保留像素断言）。
- ImageTrimTests：`calculateTrimRectBottomRightPixelColor`（Rust 只测了 TopLeft）、`uniformImageReturnsNil`（basedOn 颜色的纯色图 nil 路径无直接测试）、`trimImageDirectly`、`sessionTrimIntegrationAndUndo`（`trim_reports_nothing_to_trim` 无 undo 断言）。
- GuideTests：`lockPreventsCreatingAndMoving`、`snapTargetsFollowViewMenu`、`moveToolDragsAGuideAndRulerDropDeletesIt`、`rulerStepUsesNicePixelIntervals`（交互/标尺整体缺失）；`newGuidesUndoAndClear`（`guides_add_clear` 无 undo 断言）、`canvasAndImageSizeMoveGuides`、`flipCanvasMirrorsGuides` 仅公式级覆盖。
- DistortTests（3 例全部）：`perspectiveMappingHitsTheCornersAndDegenerateShapesAreRefused`、`distortingWarpsTheLayerIntoTheShapeAsOneUndoStep`、`distortedLayerIsTrimmedToItsVisiblePixels` —— DistortWarp 缺失的直接后果。
- ProjectTests：`projectOperationsBlockEditsAndQueueImageImports`、`layerEffectsAreRenderedInExport`（操作互斥/导出渲染域）。

**README 更正建议**：`D:\Project\rust\compositor-rs\README.md:40` 声称 "manifest JSON v1–8 semantics" 与 canvas.rs “复刻 CanvasResizer/ImageResizer/Crop/ImageTrim/LayerFlip”——前者实际为 v1–9（两边一致，README 过时）；后者对 ImageResizer 的“复刻”言过其实（见行为差异 1、2、6、7），且未提及 fill 缺失。
