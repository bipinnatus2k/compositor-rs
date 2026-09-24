> 子代理原始报告（域 C：图像调整），逐字保留，2026-09-24。

# 图像调整功能域：Rust 版 vs Swift 版深度对比报告

## 一、总体结论

**像素内核层面**：AdjustPixels.c 的全部 12 个内核（gradient_map、grain、black_white、color_balance、camera_raw、clip_overlay、camera_raw_effects、camera_raw_curve_color、sharpen_mask_overlay、camera_raw_detail、camera_raw_optics、camera_raw_calibration）与 LevelsPixels.c 的 2 个内核（levels_apply、levels_histogram）已逐公式移植到 `crates/core/src/adjust/`，**所有数值常数（sRGB↔线性常数、gamma、clip 阈值、模糊半径、噪声 hash/种子、曲线采样点数、色温/tint 增益、vignette 椭圆参数等）逐项核对全部一致，未发现任何数值差异**。README 关于“kernel-exact”的声称在内核层面成立。

**功能完整性层面**存在真实差距，集中在三处：
1. **LevelsAutomatic.swift（自动色阶 + 吸管校准）整个文件没有移植到 core**；app 层 `levels_auto` 是一个与原版**算法不同、数值不同**的简化重写（clip 0.5% vs 0.1%，且把 1024 个混合 bin 当作单一通道直方图）。
2. **CameraRaw 的会话级交互**（白平衡吸管流程、defringe 吸管、RGB 读数、定向拖拽、点颜色采样、几何参考线绘制、`CameraRawScope.preview`）在 Rust 中无对应物；其底层数学（neutralize/autoBalance）已移植，defringe 的 290°/90°/span25 逻辑则完全缺失。
3. **HueSaturation**：Swift 渲染走 33³ CIColorCube（三线性插值 + f32 量化），Rust 逐像素精确计算 HSL——语义等价但与 Swift 输出并非 1:1（Rust 反而更精确）。

---

## 二、逐公式常数核对结果（全部一致，列出关键锚点）

以下常数两边完全相同（左边 Swift/C，右边 Rust）：

| 项目 | Swift/C 位置 | Rust 位置 | 值 |
|---|---|---|---|
| sRGB 解码阈值/斜率/指数 | AdjustPixels.c:204-207 (`srgb_to_linear`) | camera_raw.rs:1887-1893 | 0.04045 / 12.92 / ((x+0.055)/1.055)^2.4 |
| sRGB 编码 | AdjustPixels.c:209-214 | camera_raw.rs:1896-1907 | 0.0031308 / 12.92 / 1.055·x^(1/2.4)−0.055 |
| Rec709 亮度 | AdjustPixels.c:216-218 | camera_raw.rs:1910-1912 | 0.2126/0.7152/0.0722 |
| exposure = 2^stops、contrast pivot | AdjustPixels.c:303-325 | camera_raw.rs:2052-2071 | exp2、0.5+(x−0.5)·scale |
| whites/blacks 区间 | AdjustPixels.c:251-260 | camera_raw.rs:1955-1968 | 0.75 / 0.25 |
| vibrance 肤色保护 | AdjustPixels.c:276-281 | camera_raw.rs:1991-1998 | hue 10–50、(sat−0.15)/0.35、0.7 |
| clipping 阈值 | AdjustPixels.c:332,337 | camera_raw.rs:2083-2092 | 254.5/255 与 0.5/255 |
| effects 模糊半径 | AdjustPixels.c:388-393,501,505 | camera_raw.rs:2149-2158,2308,2313 | texture=1、clarity=4，clamp 1..64 |
| dehaze | AdjustPixels.c:395-409 | camera_raw.rs:2160-2177 | 0.8、pivot 0.45−0.1d、0.45、0.4、sat 0.7 |
| vignette 椭圆 | AdjustPixels.c:411-448 | camera_raw.rs:2179-2232 | /√2、shape=(1−round/100)·0.5、start=mid/100·0.85、soft≥0.05、(y−0.45)/0.55、0.75 |
| glow | AdjustPixels.c:507-548 | camera_raw.rs:2316-2356 | base 2(Bloom)/5、threshold 0.55+0.4·range、denom≥0.05、halation(1, 0.35−0.3w, 0.2−0.2w)、其他(0.75+0.25w, 0.6+0.2w, 0.75−0.6w)、gain 1.4 |
| curve/mixer/grading 内核 | AdjustPixels.c:641-718 | camera_raw.rs:2505-2634 | hue ±30/360、lum 0.25、split=0.5−balance·0.2、reach=0.12+blending·0.38、0.85、visualize 0.05 |
| 混色器中心/权重 | AdjustPixels.c:628,669 | camera_raw.rs:2481-2490,2548-2549 | {0,30,60,120,180,240,270,300}/360、40/360 |
| detail 半径 | AdjustPixels.c:720-726,818,843 | camera_raw.rs:2636-2646,2788,2816 | 0.5+s/100·2.5、1+nl/50、1+ncs/40 |
| 锐化 mask | AdjustPixels.c:780,789,900,909 | camera_raw.rs:2730-2741,2868-2880 | 0.35·masking/100、max(0.04, 0.35−t·0.5)、edge·6 / edge·4、contrast·0.25 |
| 色差/镜头暗角 | AdjustPixels.c:956,975-976,996,1005 | camera_raw.rs:2947,2975-2976,3016,3021 | shift=strength·radial²·2.5、/0.35、profileVignette·35.0、0.45 |
| 校准版本系数 | AdjustPixels.c:1026-1034,1044-1045 | camera_raw.rs:3065-3093 | 0.55/0.65/0.75/0.85/0.92/1.0、15/360、0.45、l<0.35、tint·0.06 |
| 温/tint 增益 | CameraRaw.swift:52-56,211-217 | camera_raw.rs:1363-1367,1571-1579 | 0.35 / 0.15 / 0.30 |
| grain 大小映射 | CameraRaw.swift:208 | camera_raw.rs:1566-1568 | 0.5+(size/100)·19.5 |
| grain hash | AdjustPixels.c:31-60,68-73 | image_adjustments.rs:610-707 | mix32(0x7feb352d/0x846ca68b)、lattice(0x9E3779B1/0x85EBCA77)、·1.6、0.35·255、detail=max(0.5,·0.35)、fineSeed=seed^0xA511E9B3、0.4+2.4·l(1−l) |
| 色平衡色调权重 | AdjustPixels.c:156-167,179 | image_adjustments.rs:484-495,508 | a=0.25、b=0.333、scale=0.7、0.299/0.587/0.114 |
| 渐变映射级别 | AdjustPixels.c:22-26 | image_adjustments.rs:228-232 | (2126r+7152g+722b+5000)/10000、(c·a+127)/255 |
| levels 表查找/直方图 | LevelsPixels.c:3-28 | levels.rs:125-158 | unpremultiply、lo/hi 线性插值、weight=α/255、RGB bin=weight/3 |
| 显示缩放 | Levels.swift:47-56 | levels.rs:162-184 | 内部 bin 95 分位、cap=typical·4 |
| 直方图 clip（自动色阶） | LevelsAutomatic.swift:12,14 | **缺失**（见下） | 0.001 |
| 参数曲线 | CameraRawColor.swift:49-59,102-104 | camera_raw.rs:349-365,448-450 | 0.22、span≥0.02、5..90/+2..95/+2..98 |
| 几何校正 | CameraRawGeometryCalibration.swift:135-172,117-128 | camera_raw.rs:1060-1155 | 0.18·strength(1/0.55)、aspect/200、0.15、guided ±25/±45 |
| HSL 互转 | AdjustPixels.c:592-621；HueSaturation.swift:329-361 | camera_raw.rs:2416-2470；hue_saturation.rs:466-511 | 同一公式 |

---

## 三、缺失（Swift 有、Rust 无）

1. **LevelsAutomatic.swift 整文件未移植到 core**
   - `LevelsAuto`（contrast/color/neutral，端点 clip 0.1%）：Compositor/Compositor/Document/LevelsAutomatic.swift:4-37。Rust 全仓库 grep `LevelsAuto|auto_levels` 无结果。
   - `LevelsSettings.sampling(_:mode:)`（黑/灰/白吸管，gamma=log(fraction)/log(0.5)）：LevelsAutomatic.swift:39-60。无移植。
   - `EditorSession.autoLevels` / `sampleLevels`：LevelsAutomatic.swift:62-84。无移植（Rust session.rs:177-180 只有 `levels: Option<LevelsSettings>` 字段，无编辑会话流程）。

2. **CameraRaw 会话级交互**（CameraRaw.swift）
   - `sampleCameraRawWhiteBalance`（CameraRaw.swift:420-441）——数学已移植（camera_raw.rs:1808-1859 `neutralize_linear/neutralize_straight/auto_balance`），但会话流程（点击原图像素→更新滑块→`whiteBalance = .custom`）无。
   - `applyCameraRawAutoWhiteBalance`（CameraRaw.swift:445-459）——同上，仅数学在（camera_raw.rs:1837）。
   - `sampleCameraRawDefringe` + `hueDegrees`（CameraRaw.swift:462-507）——**purpleCenter=290.0、greenCenter=90.0、span=25.0（:480-481）在 Rust 任何位置都不存在**。
   - `updateCameraRawReadout`（CameraRaw.swift:510-527，straight RGB = (c·255 + a/2)/a）——无。
   - `beginCameraRawDrag`/`dragCameraRaw`（定向调整，delta·0.35、mixer 权重分配，CameraRaw.swift:529-560）——无。
   - `sampleCameraRawPointColor`（CameraRaw.swift:562-576，tone=0.2126r+0.7152g+0.0722b、lum=(max+min)/2，:609-637）——无。
   - 几何参考线绘制/提交（CameraRaw.swift:578-607）——无。
   - `CameraRawScope.preview(_:)`（CameraRaw.swift:400-415）——无；Rust 只有 `CameraRawScope::make/overlay`（camera_raw.rs:3234,3291）。

3. **Levels 的直方图选区加权与预览任务**：`LevelsFilter.histogram` 的 selection coverage（Levels.swift:86-101）在 core 有参数位（levels.rs:145 `coverage: Option<&GrayMask>`）但 **app 从不传**（crates/app/src/sheets.rs:146-151 传 `None`）；`LevelsFilter.run` 的选区回混（Levels.swift:80-82）在 Levels 路径无调用者。

---

## 四、行为差异

1. **有效性校验缺失（错误处理语义不同）**
   - `CameraRawSettings.apply`：Swift 在 CameraRaw.swift:224-225 `guard settings.isValid else { throw ProjectError.invalid }`；Rust camera_raw.rs:1584-1692 全程不调用 `is_valid()`（:1426 定义但 apply 未用），非法设置会静默处理。
   - `CurvesSettings.apply`：Swift Curves.swift:34 throw；Rust curves.rs:92-103 无检查。
   - `ExposureSettings/GradientMapSettings/BlackWhiteSettings/ColorBalanceSettings/GrainSettings.apply`：Swift ImageAdjustments.swift:65,89,133,166,202 均 throw；Rust image_adjustments.rs:123-133,177-181,310-328,458-462,587-595 无 is_valid 检查（grain 非法时静默返回原图）。

2. **HueSaturation 渲染路径不同**
   - Swift：`HueSaturationFilter.run` 构建 33³ 颜色立方体经 `CIColorCube`（GPU、三线性插值、f32 存储）应用，`dimension = 33`（HueSaturation.swift:234,240-243）。
   - Rust：`hue_saturation_apply`（mod.rs:27-47）直接逐像素 f64 调 `adjust_hsl`（hue_saturation.rs:409-448）。HSL 公式本身精确一致，但 Swift 输出经过立方体插值与 f32 量化，两者在中间色上有可测差异（Rust 无 33 这个常数，README 的“精确”指 HSL 函数本身而非渲染管线 1:1）。

3. **选区 coverage 生成方式不同**
   - Swift `PixelAdjust.coverage`（PixelAdjust.swift:23-34）：把选区路径经 `pixelToDocument` 逆变换后**重新填充光栅化**（亚像素精确、抗锯齿软边）。
   - Rust `pixel::coverage`（pixel.rs:10-52）：对预生成的 `selection.coverage` 位图做 floor 取整的**最近邻查找**，旋转/缩放图层上边缘覆盖与 Swift 不一致，且 :25-33 有一段无效的死代码（corners 循环 `let _ = (i, corner)`）。

4. **选区回混精度**
   - Swift `PixelAdjust.blend` 用 `CIBlendWithMask`（浮点，PixelAdjust.swift:38-46）；Rust pixel.rs:58-82 用整数 `(adj·a + orig·(255−a) + 127)/255`，最后一舍入与 CI 浮点混合可差 1。

5. **GuidedMatte 重采样**
   - Swift `GuidedMatte.refine` 用 CGContext `.high` 插值一步缩放（GuidedMatte.swift:68-81,100-117）；Rust pixel.rs:257-281 `resample_bitmap_gray` 用 halve+`resample(High)` 组合，滤波器不同，非位等（guided_filter 本体 epsilon=1e-4、半径折算一致）。

6. **几何 warp 实现不同**
   - Swift 用 Core Image `CIPerspectiveTransform`（CameraRawGeometryCalibration.swift:77-83）；Rust 手写单应矩阵 + `bilinear_sample`（camera_raw.rs:3130-3182），后者对越界坐标**clamp 到边缘**（raster.rs:305-334），CI 采样外部为透明——边缘像素外推行为不同。constrain-crop 的 alpha bounds + 居中回填公式一致（camera_raw.rs:1016-1043 vs CameraRawGeometryCalibration.swift:88-99）。

7. **App 层 Levels/HueSaturation 提交绕过选区**：sheets.rs:262-300 直接对整层应用 `levels_apply_settings`/`hue_saturation_apply` 并经 `commit_preview`（state.rs:537-561）整层替换；Swift 两条滤镜都经选区混合（Levels.swift:78-83、HueSaturation.swift:244-250）。FilterJob 路径有选区（state.rs:563-591），但 Levels/HueSaturation sheet 不走它。

---

## 五、简化

1. **`levels_auto`（app 层）与原版算法不同**：crates/app/src/sheets.rs:382-422。
   - 对**整个 1024 bin**（0-255 是 RGB 均值 bin、256-511 红、512-767 绿、768-1023 蓝，混在一起）累计求 0.5%/99.5% 分位（:397,405），再按 `index·255/1023`（:410）映射回滑块。
   - Swift 版：仅用三个**单通道** 256 bin 直方图，clip 0.001（LevelsAutomatic.swift:12,14），Contrast 模式取三通道 low 最小/high 最大共享区间写入 RGB（:17-22），Color 逐通道（:24-33），Neutral 另算 gamma=log(mean)/log(0.5)（:27-31）。Rust 版无 Contrast/Color/Neutral 之分、无 gamma、clip 阈值不同（0.005 vs 0.001）、bin 空间不同——注释自己声称"0.5% and 99.5%"也与原版 0.1% 不符。
2. **Camera Raw UI 只暴露 14 个滑块**（crates/app/src/sheets.rs:190-205：温/tint/曝光/对比/高光/阴影/白/黑/自然饱和/饱和/纹理/清晰/去雾/暗角），curve/mixer/grading/detail/optics/geometry/calibration/glow/grain 面板 core 全支持但 UI 无入口（README 已自述"Camera Raw with 14 sliders"）。
3. **Hue/Saturation sheet 只有 Master 三个滑块**（sheets.rs:154-163,274-300），7 色域波段/吸管/定向调整/colorize 之外的 invert_range 无 UI（core 完整支持）。
4. **预览管线**：Swift 有 8000px 上限降采样 + 异步预览任务（Levels.swift:127-139、HueSaturation.swift:383-405）；Rust app 同步全尺寸预览（sheets.rs:239-246）。
5. 小 bug：sheets.rs:290 colorize 时 `saturation: saturation.max(1.0)`（滑块原始值 −100..100 取 max(1.0)），Swift 是 settings.saturation/100 后 clamp 0...1（HueSaturation.swift:301）；使 colorize 饱和度下限被抬到 1%（原版可为 0）。

---

## 六、多出（Rust 有、Swift 对应文件无）

1. `adjustment.rs` 的 `LayerAdjustment` 把 GaussianBlur/MotionBlur/AddNoise 的参数（半径默认 10、`sampling_margin` 等，adjustment.rs:226-278）集中进来并带 `ApplyNoise` 位置种子——Swift 这些在 Filters.swift/NoisePixels.c 域（不属于本域文件，仅指出跨域合并）。
2. `mod.rs:50-62 levels_apply_settings` / `mod.rs:27-47 hue_saturation_apply` 为调整图层提供的便捷封装——Swift 中对应逻辑在 LayerAdjustment.swift（等价，非差异）。
3. app 层 `FilterJob.visualize_point_color` 默认 0（state.rs:586）导致每次滤镜预览都进入 `paint_color` 分支（camera_raw.rs:1604-1609 `visualize_point_color >= 0` 为真）——因点数为 0 无视觉影响，但与 Swift 默认 −1（CameraRaw.swift:221）不同，属于无谓开销。

---

## 七、README 声称逐条核验

| README 声称 | 结论 |
|---|---|
| levels 表格/levels_apply/histogram/显示缩放精确 | **成立**（levels.rs:125-184 与 LevelsPixels.c 逐式一致）；但直方图的选区加权在 app 未接线，自动色阶整体缺失/被改写 |
| curves 保形 Hermite 样条精确 | **成立**（curves.rs:61-88 与 Curves.swift:18-32 逐式一致；apply 表构造 curves.rs:92-103 = Curves.swift:37）|
| hue_saturation 波段/colorize/HSL 调整精确 | **部分成立**：HSL 函数、波段、colorize、invert_range 数学精确；但渲染不走 Swift 的 33³ CIColorCube（差异 4.2），且 Swift 的 `dimension=33`、`previewLimit=8000` 无对应 |
| image_adjustments 曝光/渐变映射/黑白/色彩平衡/颗粒 kernel-exact | **成立**（image_adjustments.rs 各内核与 AdjustPixels.c 逐常数一致）|
| camera_raw 全管线（几何 warp、校准、光/色、参数曲线+点曲线、混色器、分级、效果、细节、光学、scope）完整移植 | **内核层面成立**（camera_raw.rs 2037-3124 + 3210-3301 覆盖全部 12 个内核与 scope）；**会话/交互层面不完整**（吸管、读数、定向拖拽、点颜色、参考线、Scope.preview 缺失，见第三节 2）|

---

## 八、测试缺口清单（Swift 用例 → Rust 对应）

### LevelsTests.swift（10 用例）
| Swift 用例 | Rust 对应 | 状态 |
|---|---|---|
| identityAndChannelSelectionAreExactNoOps (:30) | levels.rs:191 `level_range_identity` | 仅数学；无“恒等=无操作+无撤销”会话测试 |
| inputClippingGammaOutputInversionAndAlpha (:42) | levels.rs:209/270 | 部分覆盖（gamma、alpha 反预乘）；反转输出黑白的字节级断言无 |
| channelsCoexistAndUseDocumentedOrder (:60) | levels.rs:241,225 | 已覆盖 |
| histogramExcludesTransparencyAndWeightsSelection (:71) | levels.rs:286 | 无 coverage 加权直方图测试 |
| selectionPreviewCancelCommitUndoAndPersistence (:84) | 无 | **未移植**（会话级） |
| stalePreviewCannotReturnAfterOffOrReopen (:116) | 无 | **未移植** |
| histogramDisplayKeepsDistribution... (:132) | levels.rs:298 | 部分（仅 cap=4 分支；等值/稀疏/端点分支无） |
| autoAlgorithmsAndEyedropperCalibration (:152) | 无 | **未移植**（功能本身缺失） |
| eyedropperSamplesOriginalAndRejectsTransparentPixels (:172) | 无 | **未移植** |
| panelPreview (:186) | 无 | 环境门控 UI 截图，N/A |

### HueSaturationTests.swift（12 用例）
| Swift 用例 | Rust 对应 | 状态 |
|---|---|---|
| defaultsAreAnExactNoOp (:46) | hue_saturation.rs:573 `identity_is_noop` | 仅数学 |
| hueRotatesSaturationAndLightness... (:58) | hue_saturation.rs:581,598,607 | 已覆盖（数学） |
| colorizeGivesEverythingOneHueAndKeepsAlpha (:78) | hue_saturation.rs:590 | 部分（无 alpha 断言） |
| adjustmentStaysInsideTheSelectionAndIsOneUndoStep (:89) | pixel.rs:305 `blend_through_selection` | 仅通用混合；无 Hue/Sat 完整流程 |
| previewIsLiveDoesNotTouchTheDocument... (:104) | 无 | **未移植** |
| bandWeightsRampThroughFalloffAndWrapAround (:158) | hue_saturation.rs:518,553 | 部分（330°/30° 肩部=0.5 数值未断言） |
| colorRangesAdjustIndependently (:173) | 无直接对应 | **未移植** |
| rangesLeaveOtherHuesAloneAndInvertFlipsTheBand (:186) | hue_saturation.rs:615 | 已覆盖（invert） |
| slidersEditTheSelectedRangeAndTheAfterBar... (:201) | hue_saturation.rs:563,629 | 已覆盖 |
| eyedroppersRecenterWidenAndNarrowTheBand (:217) | hue_saturation.rs:543,535 | 仅单元级（无合成图采样） |
| targetedAdjustmentPicksTheRangeUnderTheCursor (:239) | 无 | **未移植** |
| samplingNeedsAColorRangeAndAColorfulPixel (:257) | 无 | **未移植** |

### ImageAdjustmentTests.swift（8 用例）
| Swift 用例 | Rust 对应 | 状态 |
|---|---|---|
| exposureWorksInLinearLightWithOffsetAndGamma (:36) | image_adjustments.rs:715-745 | **offset 情形缺失**（Swift :44-46 offset=0.1→89） |
| gradientMapColorsByBrightnessAndReverses (:51) | image_adjustments.rs:748-796 | 已覆盖 |
| grainIsFixedInDocumentSpace... (:70) | image_adjustments.rs:877-913 | 部分（**子区域 origin(10,10) 与整图裁剪逐像素相等**的关键断言无；Rust 只测 shifted≠origin） |
| grainSizeControlsParticleScaleEvenWithRoughness (:89) | 无 | **未移植** |
| settingsSaveAndOlderAdjustmentsStillOpen (:104) | adjustment.rs:395,407 | 部分 |
| newAdjustmentLayersStartFromThePalette... (:122) | 无 | **未移植**（会话/调色板） |
| imageMenuExposureChangesTheLayerInOneStep (:156) | 无 | **未移植** |
| gradientMapColorsUseTheAppColorPicker (:175) | 无 | UI，N/A |

### CameraRawTests.swift（20 用例）
| Swift 用例 | Rust 对应 | 状态 |
|---|---|---|
| defaultsLeavePixelsAndAlphaAlone (:42) | camera_raw.rs:3369 | 已覆盖（`!FilterKind.cameraRaw.isImageAdjustment` 断言无对应测试） |
| exposureAddsOneStopAndContrastPivots (:53) | camera_raw.rs:3406 | 已覆盖 |
| tonalSlidersMoveTheEndTheyName (:78) | camera_raw.rs:3431 | 已覆盖 |
| temperatureWarmsAndTint... (:120) | camera_raw.rs:3470 | 已覆盖 |
| vibranceFavorsDullColors... (:132) | camera_raw.rs:3483 | 已覆盖 |
| eyedropperAndAutoNeutralizeAWarmPixel (:161) | camera_raw.rs:3517 | 数学已覆盖；会话白平衡吸管部分无 |
| autoWhiteBalanceNeutralizesAfterTheScan (:205) | camera_raw.rs:3537-3543 | 部分覆盖（auto 数学+空层）；会话流程无 |
| hiddenGroupIsLeftOutAndOkIsOneUndoOrNone (:233) | camera_raw.rs:3547 | 数学部分；会话撤销部分无 |
| hiddenGroupStaysOutOfTheNextOpen (:287) | 无 | **未移植** |
| textureAndClaritySharpenAnEdge... (:340) | camera_raw.rs:3736 | 已覆盖 |
| dehazeDeepensOrLiftsAndKeepsAlpha (:376) | camera_raw.rs:3766 | 已覆盖 |
| glowIsIdleAtZeroAndHalation... (:398) | camera_raw.rs:3788 | 已覆盖 |
| vignetteDarkensCorners... (:427) | camera_raw.rs:3821 | 已覆盖 |
| grainIsStableAndTheEffectsEye... (:458) | camera_raw.rs:3853 | 已覆盖 |
| histogramFollowsTheGradeAndClippingPaint... (:489) | camera_raw.rs:3895,4018 | 已覆盖 |
| curveMixerAndGradingChangeOnlyTheirOwnTones (:515) | camera_raw.rs:3695 | 已覆盖 |
| detailSharpeningNoiseAndMaskingPreview (:550) | camera_raw.rs:3921,3907 | 已覆盖 |
| opticsDistortionDefringeAndDetailEye (:574) | camera_raw.rs:3959,3942 | 大部分（removeChromaticAberration 的 adjusts 断言、showsDetail 隐藏组无） |
| geometryWarpAndCalibrationPrimaries (:605) | camera_raw.rs:3985,3972 | 已覆盖 |
| guidedUprightFollowsADrawnLine... (:629) | camera_raw.rs:3985 | 部分（“两幅图不得共享同一结果”断言无） |

### CameraRawSliderTests.swift（3 用例：:7,:24,:52）
全部为 UI 控件（轨道渐变色、旋钮命中、点击取值），**无 Rust 对应**（N/A，core 无这些概念）。

### AdjustmentLayerTests.swift / AdjustmentEditorTests（9 用例）
| Swift 用例 | Rust 对应 | 状态 |
|---|---|---|
| globalAdjustmentAffectsBelowButNotAbove (:25) | 无（render/session 域） | **未移植** |
| clippedCurveChangesOnlyItsBase... (:41) | 无 | **未移植** |
| hueOpacityAndMaskPreserveOriginalPixels (:57) | 无 | **未移植** |
| adjustmentPersistsDuplicatesAndUndo... (:73) | adjustment.rs:395 | 仅 serde 往返；撤销/复制/存储无 |
| curvesIdentityAndImageCommandPreserveAlpha (:90) | curves.rs:111,157 | 部分（alpha 平坦化字节序列无） |
| adjustmentBlendAndSoftMaskPreserveCoverage (:101) | 无 | **未移植** |
| invertAppliesWithoutAnEditor (:122) | pixel.rs:321,334 | 数学覆盖；会话“无编辑器”无 |
| sharedEditorsKeepPixelsDynamicAndSupportCancel (:157) | 无 | **未移植**（12 种参数化用例） |
| legacyHSVStillDecodesAndRenders (:262) | adjustment.rs:371,407 | 已覆盖 |

---

**优先修复建议**（按影响排序）：① 将 LevelsAuto/LevelsSample 按原版移植进 core 并重写 app 的 `levels_auto`（当前算法与数值都不对）；② 补 defringe 吸管（290/90/25）与其余 Camera Raw 会话函数；③ 为 `CameraRawSettings::apply`/各 `apply` 补 `is_valid` 检查（或明确决定静默语义）；④ 补 Hue/Sat 立方体等价性说明或实现 33³ 立方体以对齐 Swift 输出。
