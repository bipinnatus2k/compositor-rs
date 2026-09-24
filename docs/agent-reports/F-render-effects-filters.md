> 子代理原始报告（域 F：渲染合成 / 图层效果 / 滤镜 / 混合模式 / 光栅基础），逐字保留，2026-09-24。

# 渲染合成 / 图层效果 / 滤镜 / 混合模式 / 光栅基础：Swift vs Rust 深度对比报告

## 总体结论

逐条核对后，**README 的声明只有一部分成立**：

- **基本成立**：blend.rs 的 24 种模式枚举、serde 名、可分离/非可分离公式主体与 PDF 一致；effects.rs 的 `extreme()` 滑窗是 Swift 的逐行 1:1 移植；效果展开顺序（shadow → outer glow → 外描边 → 像素 → color overlay → inner glow → inner shadow → 内描边）两边一致；`effect_margin`/`placed_transform` 公式一致；Add Noise 的 hash 常数与单色/彩色/高斯逻辑、Lens Correction 的畸变模型**逐常数一致**；`pixel_to_document`、`falloff` 一致。
- **不成立/有重大出入**：① Rust 的调整图层作用于**图层自身像素**，而 Swift 是作用于**画布上已合成的下方内容**（真正的调整图层）；② `composite_pixel` 在半透明 backdrop 处偏离 PDF 合成公式；③ **HardMix 公式在 cs>0.5 时是错的**；④ 带“蒙版+效果”的图层在 Rust 被双重蒙版且效果越界采样错位；⑤ TiledLayerRenderer / RasterSnapshot / DownsampleCache 缓存 / LiveMaskRenderer（剪贴蒙版栈）完全缺失；⑥ 剪贴蒙版 source 的 coverage 不含源层的效果/蒙版/不透明度；⑦ 文件夹不透明度（effectiveOpacity）在合成中缺失；⑧ 滤镜管线缺少“grow→blur→trim”流程，且 `run_filter` 在 core 内无调用方。

---

## 一、【缺失】（Swift 有，Rust 侧完全不存在）

1. **TiledLayerRenderer 全部机制** — `Compositor/Compositor/Rendering/TiledLayerRenderer.swift:14-419`（`Piece`、`support(level:)` :27、`drawRaster` :54、`drawStroke` :65、`drawMaskStroke` :108、`drawReplacing`/透明度层 :157-196、`piece()` :295、`interiors()` :325、`aligned()` :349、`snapped()` :272）与 `TiledPieceCache` :373-419。Rust 侧仅 `crates/core/src/render.rs:1-4` 的注释声称“复刻其语义”，实际绘制只有 `render.rs:18-21` `draw_layer_pixels`（整图 `draw_affine` 一次仿射重采样）；全仓 grep 无任何 patches/pieces/`RasterSnapshot` 结构。
2. **RasterSnapshot 稀疏位图** — `RasterSnapshot.swift:6-176`（`base`+`patches`、`alignment` :15、`replacing()` 的瓦片分裂 :32-87、惰性物化 `bytes()` :158-166）。Rust 的 `crates/core/src/tools/brush.rs:520` `paint_snapshot` 直接产出整图 `ImportedImage`，无瓦片/共享/对齐概念。
3. **DownsampleCache 的 level 选择与缓存** — `DownsampleCache.swift:13-112`（`level(for:)` :32-35、`maxLevel=6` :18、100M 像素预算 :16、按 `ObjectIdentifier` 的 LRU :44-77）。Rust 只有 `raster.rs:389-411` 的单步 `halve` 与 `draw_affine` 内联预缩小（`raster.rs:502-518`），没有层级选择函数与任何缓存。
4. **LiveMaskRenderer（剪贴蒙版运行时）** — `LiveMaskRenderer.swift:7-146`：`prepareStacks` :63-77（连续剪贴子层成组、共享 base alpha）、`drawComposite` :78-111、`coverage()` :129-145（剪贴源 coverage 缓存、循环防护 256 层）。Rust 完全没有；render.rs 用 `mask_source_id` 直乘 source alpha 替代（见行为差异 6/7）。
5. **AdjustmentSurface** — `AdjustmentSurface.swift:3-20`（给空间性调整挖 halo 离屏）。Rust 无对应；与缺失 4 相关。
6. **LayerEffectsSurface（绘画中效果实时重建）** — `LayerEffectsSurface.swift:7-158`。Rust 无对应（性能向，但属于效果功能域）。
7. **EffectsPreviewCache** — `EffectsPreviewCache.swift:6-141`（sideLimit 预算 :51、:73、seed 机制 :54-58）。Rust 无对应（预览向）。
8. **LayerRenderer.drawBrushPreview** — `LayerRenderer.swift:86-172`（无重叠抗锯齿的分片预览）。Rust 无对应。
9. **滤镜的 session 级流程** — `Filters.swift` 的 `FilterEdit` :200-386、`blurMargin`/`growForBlur`/`grow(to:)` :297-337、`commitFilter` 的 grow→run→`trimmed` :502-517。Rust core 的 `filters.rs:350` `run_filter` **在 core 内无任何调用点**（grep 证实），由 app 层 `crates/app/src/state.rs:564-587` 直接对原尺寸图层像素调用，且无 blur padding、无 trim（`session.rs:1368` 的 `trimmed` 只用于 Merge Down）。seed 也硬编码为 7（state.rs:579），而 Swift 是每次编辑随机（Filters.swift:223）。
10. **MetalLayerEffects（GPU 路径）** — `MetalLayerEffects.swift:58-207`。Rust 只实现了 CPU 回退语义（可接受，无 Metal），但注意 Swift 实际生产路径优先走 GPU，其 shadow 位移是**双线性**（Metal kernel `effects_shift` :272-290），CPU 回退是最近邻；Rust 取了最近邻语义。GPU 的 inner glow 额外 gate `size > 0`（:155），CPU 路径（LayerEffects.swift:485）与 Rust（effects.rs:713）都无此 gate。
11. **文件夹不透明度** — Swift `LayerGroups.swift:63-71` `LayerOpacity.effective`（逐层乘祖先 folder 的 opacity），在 `EditorCanvas.swift:817` 用于每个图层。Rust `render.rs:233` 只用 `layer.opacity.clamp(0,1)`，祖先 folder 的 opacity 不参与合成。
12. SampleRingOverlay.swift:4-29、BrushCursorOverlay.swift:3-95 为纯 AppKit 显示视图，引擎无需对应（非缺口，仅备案）。

## 二、【行为差异】（两边都有实现，但结果不同）

1. **调整图层作用域（最重大）** — Swift 中带 `adjustment` 的层由 `LiveMaskRenderer.drawComposite`（LiveMaskRenderer.swift:84-90）拦截，`adjust()` :21-60 对 `context.makeImage()`（**画布上已绘的所有下方内容**）应用调整后画回；还支持自身蒙版裁剪作用域（`adjustmentClip`，EditorCanvas.swift:954-966）与作用强度=有效不透明度（`adjustmentOpacity` + `CIBlendWithMask`，LiveMaskRenderer.swift:47-55）。Rust `render.rs:129-131` 把 `layer.adjustment` 应用于**该层自己的像素**再上画。语义完全不同（作用对象、蒙版裁剪、不透明度三方面都不同）。`render.rs:600-623` 的测试 `adjustment_layer_alters_pixels` 固化的就是这种“自己改自己”的行为。
2. **`composite_pixel` 半透明 backdrop 偏离 PDF** — Rust `blend.rs:373`：`co = B·αs + Cb·αb·(1−αs)`；PDF/CoreImage（SeparableBlend.swift:22-46 的 CIFilter、以及 CGContext 各 cgMode）为 `co = αs(1−αb)·Cs + αsαb·B(Cb,Cs) + αb(1−αs)·Cb`。αb∈(0,1) 时 Rust 用混合色 B 替代了本应显示的源色 Cs：例：50% 不透明照片上叠 Multiply 白色层，PDF 给出白，Rust 偏灰/黑。canvas 是逐层累积的，底层半透明时必然出现。
3. **HardMix 公式错误** — Rust `blend.rs:224-231` 用 `VividLight(cb,cs) < 0.5 → 0`；PDF/Photoshop 为 `Cb ≤ 1−Cs → 0，否则 1`。cs ≤ 0.5 时二者一致；**cs > 0.5 时阈值错位**：如 (cb=0.3, cs=0.9)：Rust → 0（0.3/0.8=0.375<0.5），Photoshop → 1（0.3+0.9≥1）。且 cb+cs 恰等于 1 时两边结果相反（ColorBurn 恰得 0.5，Rust 判 1，PDF 判 0）。`blend.rs:443-445` 的测试用例 (0.6,0.7) 恰好落在两边都为 1 的区域，未能暴露。
4. **Divide 的零源特例** — Rust `blend.rs:235-245`：`cs≤0 && cb≤0 → 0`；PDF 规定 `Cs = 0 → 1`（黑底叠黑源：Rust 黑，PDF/PS 白）。
5. **“蒙版+效果”双重蒙版 / 效果越界采样错位** — Rust：`render.rs:143-146` 取 own_mask → `render.rs:177-185` 传入 `render_effects` 烘进 shown（正确）→ 但 `render.rs:206-229` 又用 own_mask 对 placed alpha 再乘一次；此时 `doc_to_grid`（`render.rs:206`）映射的是**已 grown 的网格**而 `own_mask` 是原始尺寸：层内半遮罩像素 coverage 被平方；效果区域（原网格之外）坐标传入 `sample_grid_alpha`（`render.rs:254-271`）后，右/下方向被 `sample_bitmap` 钳到 mask 边缘行/列值，左/上方向判 0 —— 即阴影/辉光/描边被错误地按 mask 边缘值调制或直接裁掉。Swift 把 mask 烘进 shown（LayerEffects.swift:445 `masked()`）后以 **`mask: nil`** 绘制效果图（EditorCanvas.swift:878-879、894、900-901），效果不被蒙版二次裁剪。
6. **剪贴蒙版 source 的 coverage 来源** — Swift `LiveMaskRenderer.coverage()`（LiveMaskRenderer.swift:129-145）通过完整 `drawOwn` 渲染源层（含效果、不透明度 `effectiveOpacity`、自身蒙版、递归剪贴）。Rust `render.rs:276-304` `multiply_by_source_alpha` 只取 `source.asset` 原始像素 alpha 与 source transform——**源层的效果/蒙版/不透明度全部不参与**。
7. **剪贴组的混合/成组语义** — Swift：连续剪贴子层在共享 group surface 内先互相合成（子层 backdrop 是其他子层），保留 base 的 alpha（`prepareStacks` LiveMaskRenderer.swift:63-77 + `drawComposite` :84-111），整组以 base 的 blend mode 上画；且剪贴关系基于 renderLayers 顺序的相邻性。Rust：每个被剪贴层各自乘 source alpha 后，直接对**整画布** backdrop 用**自己的** blend mode 合成（`render.rs:234-250`），`mask_source_id` 也不要求相邻/同父。
8. **Gaussian Blur 滤镜的边界** — Swift 刻意不 clamp（Filters.swift:146-148 注释 + :164-166，CI 无 clampedToExtent → 边缘向透明扩散），并靠先 pad（:297-313）后 trim（:115-133）实现“溢出再裁”。Rust `gaussian_blur_rgba`（raster.rs:550-589，`clamp_mirror_index` :623-634）**边缘复制**，且 app 层（state.rs:564-587）无 pad/trim——层边缘处结果方向相反（一个变淡、一个保持能量）。注意 effects 的 coverage blur 两边都 clamp（LayerEffects.swift:567 `clampedToExtent` vs effects.rs:573 `gaussian_blur_gray`），那处是一致的。
9. **Motion Blur kernel 形状** — Swift 用 `CIMotionBlur`（锥形/类高斯 taper，Filters.swift:135-138、:167-173），半径 = distance/√12。Rust `filters.rs:401-441` 用**均匀 box 平均 + `floor` 整像素最近邻采样**（非轴向角度时 snaps 到整数像素）；半径系数一致（`MOTION_RADIUS_PER_PIXEL` :343 = 1/√12），但 profile 与亚像素采样不同，长 streak 下边缘条纹表现不同。
10. **halving 算法不同** — Swift `DownsampleCache.halve`（DownsampleCache.swift:82-111）：vImage **Lanczos** + 四周 8px 透明 padding + `rgba_clamp_premultiplied` 抑制振铃 + 尺寸**向上取整** `(w+1)/2` + 灰度 mask 重复末行/列；Rust `raster.rs:389-411` `halve`：2×2 **box 平均** + 尺寸**向下取整** `w/2` + 奇数边复制最后像素。减半结果与“level k 像素 i 覆盖 2^k 源像素”的对齐语义都不同。
11. **预缩小阈值差一档** — Rust `draw_affine`（raster.rs:504）在 `scale ≤ 0.5` 即 halve；Swift `DownsampleCache.level(for:)`（:32-35）`factor < 0.5` 才启用。scale 恰为 0.5 时 Rust 多缩一次。
12. **插值质量策略** — Swift 缩小时非 Nearest 一律 Low、放大用层自身 quality（LayerRenderer.swift:42-44）；Rust 不区分（`sample_bitmap` raster.rs:293-297 注释自认 Smooth/High 同为双线性），无 Low/Medium 概念。
13. **Add Noise 原点取整** — Swift `Int64(noiseOrigin.x.rounded(.down))`（Filters.swift:181）；Rust `job.noise_origin.x.floor() as i64`（filters.rs:385-386）。对负坐标 `rounded(.down)` 与 floor 等价，当前一致（备案：两边若改用 trunc 会失配）。hash/常数本身逐项一致（NoisePixels.c:5-49 vs filters.rs:230-283）✓。
14. **Lens Correction** — 畸变模型、`k = distortion/100 × 0.35`、双线性权重、`lround` vs `+0.5` 截断逐项一致（LensPixels.c:4-37 vs filters.rs:286-339）✓；Swift 侧仅多 `scale` 相对化注释（Filters.swift:185），Rust 同为相对自身尺寸 ✓。

## 三、【简化】

1. `render_effects`（effects.rs:580-749）无结果缓存：Swift 有 `LayerEffectsRenderer.cache`（LayerEffects.swift:375-408，按 image/mask/effects 身份命中）+ EffectsPreviewCache。
2. 无尺寸上限 guard：Swift `render` 有 `width*height ≤ 100_000_000`（LayerEffects.swift:441）、GPU 路径 80M（MetalLayerEffects.swift:61）；Rust 无。
3. `mask_clip_image`（render.rs:27-57）无 Swift `clipImage` 的 `limit` 交互降采样（LayerMask.swift:110-126）与 MaskPlacementCache（:128 起）。核心重采样逻辑（`placed`/`background_of`，document.rs:294-310 vs LayerMask.swift:66-84）一致 ✓。
4. 高斯 kernel 半径 4σ（raster.rs:554、:596）vs CI 内部约 3σ 截断——大 sigma 时尾部权重更多。
5. `MetalBrushCoverage`（MetalBrushCoverage.swift:88-160）的连续沉积模型（8 点 Gauss–Legendre 积分 tipDensity、`1−exp(−density)`、硬笔尖 `(radius−d)/AA+0.5`）在 Rust `tools/brush.rs:201-227 tip` + :384-400 dab（max/screen 叠加、整 tip blit）被近似为离散 dab 盖章——软笔画的沉积曲线不同（属笔刷域，但 Swift 侧该文件属本功能域清单）。
6. `resample`（raster.rs:339-386）的 High 预缩小循环条件（:360-363）冗长且与 `draw_affine` 的循环（:502-518）逻辑不一致（两处阈值/终止条件写法不同）。

## 四、【多出】（Rust 有，Swift 对应处没有）

1. `render.rs:319-436`：`render_shape_image`/`rounded_rect_path`/`shrink_by`/`paint_distance_to_segment` —— Swift 形状渲染在 ShapeTool/`shapeTransformPreview` 侧，Rust 把它并入渲染器（合理的架构差异，备案）。
2. `render.rs:202-250` 手写的逐像素 mask 乘法与 unpremultiply→`composite_pixel`→premultiply 合成循环（替代 CGContext/CI 的原生合成）——必要，但正是差异 2/5 的载体。
3. `filters.rs:467-483` `crop_bitmap`（Swift 用 `CGImage.cropping`）。
4. `render.rs:439-455` `sample_composite_color` 的 unpremultiply 取色（Swift 在 `sampleCompositeColor`，非本文件）——对齐项。

## 五、测试缺口清单（Swift `Compositor/CompositorTests/` → Rust `#[test]`）

**TiledLayerTests.swift（6 用例）— 全部未移植**（Rust 无 tiling 基础设施）：
- `tiledLayersDrawLikeOneImage`（:68，参数 (0.2,0°)(0.7,0°)(0.3,25°)）
- `maskStrokesDrawLikeOneMask`（:115，同参数）
- `paintingAtTheLayersEdgeDoesNotChangeIt`（:143，参数 10.749/1.0）
- `translucentStrokesDrawLikeOneImage`（:192）
- `paintingAScaledDownLayerDoesNotShiftItsPixels`（:282）
- `paintingAScaledDownLayersMaskDoesNotShiftItsPixels`（:290）

**RasterSnapshotTests.swift（5 用例）— 全部未移植**：`mouseUpAndNextStrokeNeverFlattenTheDocument`(:16)、`snapshotsStayImmutableAndDisplayMatchesExportAcrossSuccessiveStrokes`(:42)、`maskMouseUpIsImmediateAndPreservesTheImage`(:81)、`softwareFallbackKeepsTheSoftStrokeContinuous`(:101)、`eightHundredPixelSoftStrokeHasNoPeriodicRidges`(:120)。

**DownsampleTests.swift（4 用例）— 全部未移植**：`halvingsAreReusedAndOnlyUsedForLargeReductions`(:37)、`aHardEdgeStaysSharpShrunkEightTimes`(:46)、`fineStripesAverageToFlatGrayWithoutShimmer`(:55)、`translucentEdgesStayValidAndMasksStayGray`(:66)。Rust 仅有 `raster.rs:756 halve_averages_quads`、`:767 resample_nearest_picks_centers` 等弱断言单元测试。

**FilterTests.swift（4 用例）— 部分**：
- `gaussianBlurSoftensAHardEdgeAndSpreadsPastTheLayerEdgeAsOneUndoStep`(:7) → session 级（grow/trim/一次 undo）**未移植**；filters.rs:584 `gaussian_blur_spreads_and_keeps_size` 与 :610 `trimmed_crops_to_content` 只覆盖单元片段，且由于缺失一.9，Rust 无法通过该 Swift 断言（blur 不向层外扩散）。
- `motionBlurStreaksAlongItsAngleCounterclockwiseFromHorizontal`(:48) → 部分：filters.rs:597 覆盖水平/垂直，**45° 对角断言未移植**。
- `addNoiseChangesColorButNeverAlphaAndMonochromaticKeepsGrays`(:74) → 部分：filters.rs:515/:544 覆盖确定性/单色/origin，**“alpha 永不变”与“透明区保持全零”断言未移植**。
- `removeDistortionBendsAboutTheCenterAndOnlyPincushionCorrectionOpensTheCorners`(:101) → 部分：filters.rs:557/:569 只测恒等与角落移动，四象限方向性与“只有 pincushion 打开角落”未移植。

**InnerGlowTests.swift（8 用例）**：`innerGlowDefaultsAndValidation`(:13)、`innerGlowCodableRoundTrip`(:34)、`innerGlowBackwardCompatibility`(:53)、`innerGlowRendersInsideSourceWithoutBoundsExpansion`(:103)、`innerGlowRendersAroundTextGlyphs`(:141)、`innerGlowPreservedInExport`(:219) **未移植**（effects.rs 无 inner glow 渲染/serde round-trip 测试；`serde_defaults_mean_visible` :800 只测 stroke 默认值）；`layerEffectsIntegration`(:75) 部分（effects.rs:823 `effects_kinds_and_colors`）；`innerGlowCPUAndMetalParity`(:181) 无对应（无 Metal）。

**OuterGlowTests.swift（10 用例）**：`outerGlowDefaultsAndValidation`(:9)、`outerGlowCodableRoundTrip`(:29)、`layerEffectsCodableBackwardCompatibility`(:67)、`outerGlowRendersOmnidirectionally`(:103)、`outerGlowSizeAndOpacityVariations`(:146)、`outerGlowRendersAroundTextGlyphs`(:186)、`outerGlowCombinedWithStrokeAndDropShadow`(:237)、`outerGlowPreservedInExport`(:275) **未移植**；`layerEffectsIntegration`(:43) 部分（同上）；`outerGlowCPUAndMetalParity`(:301) 无对应。Rust 效果渲染测试仅有 shadow（effects.rs:859）与 stroke（effects.rs:881）。

**BlendShortcutTests.swift（1 用例）**：`shiftPlusAndMinusStepTheActiveLayersBlendModeWhereverFocusIsExceptTextFields`(:7) → 逻辑部分对应 `session.rs:1678 blend_mode_cycles_through_menu_order`（顺序与 wrap 覆盖）；焦点/文本框例外与 undo 断言未移植（UI 域，勉强合理）。

**优先修复建议排序**：二.1（调整图层作用域）≥ 二.5（双重蒙版）> 二.3（HardMix）> 二.2（半透明 backdrop 合成）> 一.4/二.6/二.7（剪贴蒙版语义）> 一.11（folder opacity）> 二.8/一.9（滤镜边界与流程）> 其余。
