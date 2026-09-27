# 调整子系统实现审查（adjust / adjustment 分发）

- 日期：2026-09-28
- 分支：dev-ui-fix
- 审查对象（全部完整读完，非抽样）：
  - `crates/core/src/adjust/camera_raw.rs`（4229 行）
  - `crates/core/src/adjust/hue_saturation.rs`（981 行）
  - `crates/core/src/adjust/image_adjustments.rs`（934 行）
  - `crates/core/src/adjust/pixel.rs`（604 行）
  - `crates/core/src/adjust/levels.rs`（308 行）
  - `crates/core/src/adjust/levels_auto.rs`（308 行）
  - `crates/core/src/adjust/curves.rs`（169 行）
  - `crates/core/src/adjust/mod.rs`（110 行）
  - `crates/core/src/adjustment.rs`（490 行，分发部分）
- 方法：逐行读代码，所有结论以亲自读到的代码 + 精确行号为准；死代码结论先 grep 全 `crates/` 再下断言；`docs/agent-reports/C-adjustments.md`（2026-09-24）仅作交叉核对。

---

## 一、总览表（以实际代码中的枚举/结构为准）

| 功能（代码实体） | 实际状态 | 说明 |
|---|---|---|
| Levels 数学（`LevelRange`/`LevelsSettings`，levels.rs:36-121） | 完整实现 | 归一化钳制（:67-82）、gamma、先通道后 RGB 复合（:118-121）均正确；black<white 由 normalized 保证，无除零 |
| `levels_apply`（levels.rs:125-140） | 完整实现 | unpremultiply→表插值（lo/hi 边界 ：134 有 `lo < 255` 守卫）→premultiply，正确 |
| `levels_histogram`（levels.rs:145-158） | 完整实现 | RGB bin = 三通道均值（非亮度）——注释 ：142-144 与代码一致，自洽 |
| `LevelsHistogramDisplay`（levels.rs:162-184） | 实现完整但生产无调用者 | 唯一非测试调用者是死代码 `CameraRawScope::peak`（见低-4） |
| Levels 自动（`LevelsAuto` Contrast/Color/Neutral，levels_auto.rs） | 完整实现 | 0.1% clip（:93）、四组 256 bin（:80-84）、Neutral gamma=ln(mean)/ln(0.5)（:70）、端点塌缩返回 None（:112）；app 已接线（app/sheets.rs:553-563） |
| Curves（`CurvesSettings`，curves.rs） | 基本完整但有缺陷 | Hermite 保形样条正确；`apply` 有 `is_valid` 守卫（:95-97）；`value()` 单独暴露、对非法数据（重复 x）会除零并静默吞 NaN（见低-5） |
| Hue/Saturation HSL 数学（`adjust_hsl`，hue_saturation.rs:412-451） | 基本完整但有缺陷 | 非 colorize 路径有负色相回绕（:436-439）；**colorize 路径无回绕（:422），负色相输出错误颜色（高-1）** |
| Hue/Saturation 波段（`HueBand`，:78-248） | 完整实现（数学），UI 未接线 | wrap/weight/include/exclude/set_handle 均正确处理 0/360 回绕；但 centered_on/include/exclude/set_handle 全工程仅测试调用 |
| Hue/Saturation 33³ 立方体（`ColorCube` + `apply_cube_rgba8`，:478-598） | 完整实现 | 33³（:471）、f32 三线性（:534-569）、un/premultiply（:584-598），与 mod.rs:35-41 入口一致 |
| `hue_response`（:390-409） | 完整实现 | 361 项表，索引越界有钳制（adjust_hsl:434）；HashMap 迭代序求和（低-9） |
| Exposure（`ExposureSettings`，image_adjustments.rs:54-137） | 完整实现 | sRGB→线性→×2^stops+offset→gamma→编码（:102-121），`apply` 有 is_valid 守卫（:125-127） |
| Gradient Map（:141-241） | 完整实现 | Rec.709 整数亮度（:235）、un/premultiply、表钳制 `level.min(255)` 正确 |
| Black & White（:245-411） | 完整实现 | 六权重主/次色分解（:351-366）+ HSL tint（:369-406），权重越界钳制 |
| Color Balance（:415-540） | 完整实现（近似算法） | tonal_weights（:497-508）为近似色调权重；preserve_luminosity 用 Rec.601 亮度（:521,528），与其他内核的 Rec.709 不一致（低-8） |
| Grain（:544-720） | 完整实现 | 种子化 value-noise（mix32/lattice :623-659）、文档空间坐标入参；但"document space"声明与调整层路径不符（中-6） |
| 选区覆盖/回混（pixel.rs `coverage`/`coverage_path`/`blend`/`blend_path`） | 完整实现且已接线 | coverage_path 经逆变换重光栅化+羽化（:45-79）；blend_with_mask 为 f32 浮点（:94-99）；app sheets.rs:741、filters.rs:405 均调用 |
| `blend_gray`/`invert_mask`（pixel.rs:134-155,184-197） | 死代码 | blend_gray 唯一调用者是 invert_mask，invert_mask 仅测试调用（低-3） |
| Guided Matte（pixel.rs:201-308） | 实现完整但全工程无生产调用者 | guided_box 滑动窗正确（核对过边界递推）；guided_refine 仅测试调用（低-3） |
| Pixel Invert（pixel.rs:165-181） | 完整实现 | `alpha - color`（wrapping，:173-175），与注释一致 |
| Camera Raw 光（曝光/对比/高光/阴影/白/黑，`adjust_camera_raw`，camera_raw.rs:2183-2249） | 真实像素算法（部分粗糙近似） | 曝光为线性光 exp2（真实）；对比度为 sRGB 域 0.5 pivot 线性缩放（:2215-2217，粗糙近似）；高光/阴影为亮度缩放+二次权重（:2080-2098，真实但简化）；白/黑仅作用 >0.75 / <0.25 区间线性段（:2101-2114，粗糙近似）；±100 端极端（阴影 +100 把纯黑提到 0.5 灰）为移植公式行为 |
| Camera Raw 白平衡（`gains` :1577-1585） | 真实实现（线性通道增益近似） | 温度/tint → RGB 增益线性模型，非矩阵 WB；Auto（`auto_balance` :1847-1869 灰世界求解）已被 app 调用（app/state.rs:1621） |
| Camera Raw 纹理/清晰度/Glow（`adjust_camera_raw_effects` :2406-2552） | 真实像素算法 | box blur（非高斯）单应差；Glow 阈值+模糊+着色（:2462-2502），三样式（Diffusion/Bloom/Halation）有真实区别 |
| Camera Raw 去雾（`effects_dehaze` :2306-2323） | 粗糙近似 | 亮度对比 + 饱和度缩放，无大气散射模型/透射率估计——参数生效，但效果与"去雾"名义有差距 |
| Camera Raw 暗角（`effects_vignette` :2325-2378） | 基本完整但有缺陷 | 椭圆形状/羽化真实；**三个 Style 只有两种行为**（中-4） |
| Camera Raw 颗粒（:1299-1305,1681-1692） | 完整实现 | 复用 `adjust_grain`，`units_per_pixel = 1/scale` 保证文档空间锚定（此路径声明成立） |
| Camera Raw 曲线（`CameraRawCurveSettings` + `adjust_camera_raw_curve_color` :2651-2780） | 真实像素算法 | 参数化为区间三角权重×0.22（:349-365，近似）；点曲线经 Hermite 表（:381-407）；refine_saturation（:2675-2681）；热循环性能问题（中-5） |
| Camera Raw 颜色混合器 8 族（:529-622 + 内核 ：2693-2717） | 真实像素算法 | 色相距离权重、±30°/饱和乘性/亮度 0.25；权重归一化仅在 sum>1（低-6） |
| Camera Raw 点颜色（`CameraRawPointColor` ≤8 个，:486-525 + 内核 ：2718-2734） | 真实像素算法 | 三维（h/s/l）权重、visualize 变暗（:2769-2777）；`visualize: bool` 字段 write-only（低-14） |
| Camera Raw 颜色分级 4 轮（`CameraRawGradingSettings` + 内核 ：2739-2768） | 真实像素算法 | 阴影/中间调/高光/全局权重归一（:2742-2750）、balance 移动交叉点（:2739） |
| Camera Raw 锐化/降噪（`adjust_camera_raw_detail` :2896-3037） | 真实像素算法（粗糙） | 亮度 NR=box blur+边感知混合；颜色 NR 只模糊 HSL 饱和度平面（:2961-2996，不动色相噪声）；锐化=unsharp+8 采样点边缘度量（`sharpen_edge_at` :2823-2847）；蒙版预览（:2849-2894）真实 |
| Camera Raw 光学（`adjust_camera_raw_optics` :3137-3193） | 部分近似 | 手动畸变真实（`lens_distort`）；**去色差仅水平 R/B 位移**（中-2）；**"镜头配置文件"无任何档案数据，是固定近似**（中-3）；defringe（紫/绿色相范围去饱和 ：3039-3071）真实且吸管已接线（app/state.rs:1572-1577） |
| Camera Raw 几何（`CameraRawGeometrySettings::apply` :1006-1050 + `perspective_warp` :3278-3336） | 真实实现 | 单应矩阵求逆+双线性，越界透明（:3325-3330，有回归测试 :4176-4199）；constrain crop 裁剪回填（:1022-1049） |
| Camera Raw Upright Guided（:1066-1097） | 粗糙近似/半 stub | 仅第一根线参与旋转；第二根线只按角度符号给**固定 ±25**（:1092-1093）；第三根及以后忽略；无消隐点求解（中-1）。当前无 UI 入口 |
| Camera Raw 校准（`adjust_camera_raw_calibration` :3195-3270） | 粗糙近似 | 主导通道 hue/sat 微移 + 阴影 tint，非 3×3 原色矩阵；process 版本 1-6 仅缩放强度（:3211-3223，低-11） |
| Camera Raw 裁剪警告/锐化蒙版预览（`clip_overlay` :3451-3455、`CameraRawClipping` :68-74） | 完整实现且已接线 | filters.rs:224/359-367 |
| Camera Raw 示波器（`CameraRawScope` :3364-3448、`CameraRawScopeMode` :132-139） | 死代码 | make/peak/overlay 全工程（含 app）无调用者，仅本模块测试用（低-1） |
| Camera Raw 面板组开关（`CameraRawSettings::applying` 及各子设置 `applying`） | 死代码 | 10 参版本 ：1511-1569 与子设置 ：709/:793/:895/:1000/:1226 全部仅测试调用（低-1） |
| 调整层分发（adjustment.rs `LayerAdjustment::apply` :323-371） | 完整实现 | 12 种 AdjustmentKind 全部有真实实现路径；**Grain 的 region 恒为 None**（render.rs:552,875 只传 None），见中-6 |

UI 暴露面（现状，非缺陷）：Camera Raw 滤镜 UI 仅 14 个滑块（app/sheets.rs:292-310，映射 sheets.rs:530-547）；Hue/Saturation sheet 仅 Master 三滑块 + colorize 复选框（app/sheets.rs:404-431、sheets_ui.rs:528-543）。curve/mixer/grading/detail/optics/geometry/calibration/glow/grain 在 core 完整但无 UI 入口——因此中-1（Guided）等 core 缺陷当前对终端用户不可达，但对库使用者与未来 UI 是现实风险。

---

## 二、发现清单

### 高

- **[高] Colorize 模式负色相输出错误颜色** — `crates/core/src/adjust/hue_saturation.rs:422` + `crates/core/src/adjust/hue_saturation.rs:624-645`
  `adjust_hsl` 的 colorize 分支 `hue = settings.hue() % 360.0`（:422）没有负数回绕（对比非 colorize 分支 ：436-439 有 `if hue < 0.0 { hue += 360.0 }`）。Rust 的 `%` 保留符号，hue=-30 时 `to_rgb`（:624-645）里 `sector = -0.5`，`sector as i32` 落到 0 臂且 `second` 为负（:630），例如 hue=-30°（应为 330° 品红粉）输出约为 `(l+c/2, l-c, l-c/2)` 而正确是 `(l+c/2, l-c/2, l)`——绿/蓝通道完全错乱；hue=-180° 时三个通道全错。该路径经 33³ 立方体（`ColorCube::build` → `adjust_hsl` :499-505）放大到全部 35937 个格点。UI 可达：app 色相滑块范围 −180…180（app/sheets.rs:405）+ colorize 复选框（app/sheets_ui.rs:528-543）；`LayerAdjustment::is_valid`（adjustment.rs:284）允许 |hue|≤360，旧文档中的负 hue 也可触发。测试盲区：`colorize_fills_with_hue`（:724-729）只用 colorize_start()=hue 0。

### 中

- **[中] Upright Guided 是固定值近似，不是引导线求解** — `crates/core/src/adjust/camera_raw.rs:1066-1097`
  `guided_corrections` 只读第一根线算旋转（:1067-1076）；有第二根线时 `vertical`/`horizontal` 只按其角度的**符号**给硬编码 `±25.0`（:1092-1093），与线的实际倾角无关；第三根及以后的线完全忽略（:1085 只取 `guides[1]`）。注释 ：141 声称 "guided reads drawn lines"，实际只有第一根线的角度被真实使用。无 UI 入口（见总览表末注），但对库语义是误导。
- **[中] "Remove Chromatic Aberration" 只做水平方向校正** — `crates/core/src/adjust/camera_raw.rs:3073-3104`
  `optics_chromatic` 只沿 x 轴对 R/B 采样位移（:3094-3097 的 `rx`/`bx`，y 不变）且强度固定 0.45（:3167）。真实横向色差是径向的（含 y 分量），纵向边缘的色边完全不会被去除。开关名暗示的能力与实现差距大。
- **[中] "Enable Lens Profile" 没有任何镜头配置文件数据，是固定近似** — `crates/core/src/adjust/camera_raw.rs:899-909, 3161-3162`
  `distortion_k`（:901-909）把 `profile_distortion/100 × 0.35` 直接加进手动畸变；`profile_vignetting` 只以固定 `×35.0` 进入暗角校正（:3162）。开启开关（默认 profile_vignetting=100，:838-839）即无条件施加同一通用校正——没有镜头型号、没有档案插值。字段名/开关名声称的能力不存在。
- **[中] 暗角三个 Style 只有两种实际行为，Paint Overlay 与名称不符** — `crates/core/src/adjust/camera_raw.rs:2356-2377`
  `effects_vignette` 中 style==0（Highlight Priority）有高光保护（:2357-2360），style==1（Color Priority）是"去饱和"近似（:2371-2377），style==2（Paint Overlay）没有任何独立行为（既无高光保护也无去饱和，等于"无选项"分支）。枚举 ：46-55 给用户的暗示是三种不同的上色方式。
- **[中] 曲线/混色器/分级内核每像素重复计算常量** — `crates/core/src/adjust/camera_raw.rs:2752-2767`
  `adjust_camera_raw_curve_color` 的像素循环里，`hsl_to_rgb(wh, 1.0, 0.5)`（:2760）只依赖 `grade` 常量，却对每个像素 × 4 个轮盘重算（每次 3 个 `hue_to_rgb` 含除法）。2400 万像素图约 1 亿次纯冗余转换；提升到循环外是一行缓存。同函数内混色器/点颜色循环本身是必要的。
- **[中] Grain 的"文档空间锚定"声明在调整层路径不成立** — `crates/core/src/adjust/image_adjustments.rs:542-543`（注释 "fixed in document space by seed"）vs `crates/core/src/adjustment.rs:341-349` + `crates/core/src/render.rs:552, 875`
  `LayerAdjustment::apply` 在 region 为 None 时以 (0,0) 与 1.0 单位/像素调用 `GrainSettings::apply`（adjustment.rs:342-348），而 render.rs 两处调用均传 `None`——颗粒相位锚定在**图层像素网格**而非文档：图层移动/两图层同参数时颗粒随之平移/重合。Camera Raw 滤镜路径通过 `1.0/scale`（camera_raw.rs:1681-1692、filters.rs:371）正确实现文档空间锚定，两条路径行为不一致，注释声明只对后者成立。

### 低

- **[低] camera_raw.rs 存在大片死代码，且被模块级 `#![allow(dead_code)]`（:4）掩盖** — `crates/core/src/adjust/camera_raw.rs:309-326, 369-379, 410-439, 570-580, 709-712, 793-795, 895-897, 1000-1002, 1226-1228, 132-139, 1511-1569, 1818-1843, 1931-1941, 3364-3448`
  grep 全 `crates/` 验证：`medium_contrast`（:309，零调用含测试）、`region_index`（:369，仅测试）、`nudged`（:410，仅测试）、`weights_for_hue`（:570，仅测试；内核在 ：2693-2695 重算同式）、5 个子设置 `applying`（:709/:793/:895/:1000/:1226，无调用者）、`CameraRawSettings::applying` 10 参版本（:1511，仅测试）、`neutralize_straight`/`neutralize_linear` 对外唯一入口 `apply_auto_white_balance`（:1931，仅测试；app 在 state.rs:1621 直接调 `auto_balance`）、`CameraRawScope`+`CameraRawScopeMode`（:3364-3448/:132-139，含 vectorscope/直方图整套，无任何 UI 调用）。
- **[低] `CameraRawPointColor.visualize` 字段 write-only** — `crates/core/src/adjust/camera_raw.rs:508` — 仅被 serde 往返写入（测试 ：3545），内核的 visualize 走独立 i32 参数（:1596,:2769）。
- **[低] hue_saturation 死代码** — `crates/core/src/adjust/hue_saturation.rs:251-257, 320-322, 454-467, 152-171, 174-190, 193-209, 231-247` — `HueSampleMode`、`colorize_start`、`shifted_hue`、`HueBand::centered_on/include/exclude/set_handle` 全部仅本模块测试调用；波段编辑/吸管取样 UI 未接线。`mod.rs:14-17` 还把 `HueSampleMode` 导出为公共 API。
- **[低] pixel.rs 死代码** — `crates/core/src/adjust/pixel.rs:134-155, 184-197, 201-308` — `blend_gray` 唯一调用者是 `invert_mask`，后者仅测试调用；`guided_box/guided_filter/guided_refine`（GuidedMatte 移植，实现本身正确）无任何生产调用者。
- **[低] `LevelsHistogramDisplay` 无生产调用者** — `crates/core/src/adjust/levels.rs:162-184` — 唯一非测试调用者是死代码 `CameraRawScope::peak`（camera_raw.rs:3378-3385）；Levels 面板的显示缩放不经过它。
- **[低] `CurvesSettings::value` 对未验证数据除零并把 NaN 静默钳成边界值** — `crates/core/src/adjust/curves.rs:63-88` — `value()` 是 pub 且不校验：重复 x 时 `d`/`h` 为 0/NaN，`((x-p[i].x)/h).min(1.0).max(0.0)`（:82）按 Rust `f64::min/max` 语义把 NaN 吞成 1.0，最终 `y.min(255.0).max(0.0)`（:87）把 NaN 吞成 255。`apply` 有 `is_valid` 守卫（:95）兜底，直接调用 `value` 的 UI 路径（app/sheets_ui.rs:1982）依赖数据合法性。同函数每次调用分配 slope Vec（:68）。
- **[低] 混色器权重归一化仅在 sum>1 时执行** — `crates/core/src/adjust/camera_raw.rs:2704-2708` — 两族权重和恰为 1.0 附近有不连续（0.999 不除、1.001 除），边界色相上偏移量有轻微跳变。
- **[低] `blend_gray` 仍是整数舍入，与 RGBA 路径的 f32 混合不一致** — `crates/core/src/adjust/pixel.rs:151` vs `:94-99`。
- **[低] preserve_luminosity 用 Rec.601 亮度，其余内核用 Rec.709** — `crates/core/src/adjust/image_adjustments.rs:521, 528` vs `:235`（渐变映射）、`:708`（颗粒）、camera_raw.rs:2056-2058。
- **[低] `hue_response` 按 HashMap 迭代序做浮点求和** — `crates/core/src/adjust/hue_saturation.rs:394` — 求和顺序跨运行不定（≤7 项，实际影响在舍入位以下，仅可复现性问题）。
- **[低] `CameraRawSettings::is_valid` 失败时静默返回原图** — `crates/core/src/adjust/camera_raw.rs:1600-1602`（另 ：1604 的 identity 早退）— 与 Swift 抛错（注释自引 CameraRaw.swift:224）语义不同：用户得不到"设置非法"反馈。同样模式见 image_adjustments.rs:125/:182/:318/:469/:602 与 curves.rs:95。属已声明的静默设计，列为语义差异。
- **[低] 校准 process 版本 1-6 仅缩放强度** — `crates/core/src/adjust/camera_raw.rs:3211-3223` — 枚举 `CameraRawProcessVersion`（:162-177）暗示六代处理差异，实际只是一个 0.55…1.0 的乘数。
- **[低] 预览半径下限 1 使低 scale 预览与成品不一致** — `crates/core/src/adjust/camera_raw.rs:2295-2304`（`effects_radius` 下限 1）、`:2782-2792`（`detail_radius` 下限 0.5 再进 `effects_radius` 取整 ≥1）— 强降采样预览时纹理/清晰度/锐化的核宽相对偏大。
- **[低] `mixer.points` 截断不一致** — `crates/core/src/adjust/camera_raw.rs:619`（`normalized()` take(8)）vs `:564-566`（`adjusts()` 检查全部点）— 第 9 个及以后的点带位移时 `adjusts()=true` 但内核忽略之；当前无点颜色 UI，理论性。

---

## 三、与旧报告 C 的矛盾点（旧报告 ：220 行版本，共 13 条实质性矛盾）

旧报告的关键论断大多已因代码演进失效；以下按旧报告位置列出，右侧为当前代码证据：

1. **旧 §三.1/§五.1「LevelsAutomatic 整文件未移植；app 层 `levels_auto` 是算法不同的简化重写（clip 0.5%、1024 bin 混算、无三种模式、无 gamma）」** — 矛盾。现在 `crates/core/src/adjust/levels_auto.rs`（整文件 1-308）实现了 Contrast/Color/Neutral 三模式、0.1% clip（:93）、正确的四组 256 bin 读取（:80-84）、Neutral gamma=ln(mean)/ln(0.5)（:70）；app 已接线（app/sheets.rs:553-563、sheets_ui.rs:489-517）。旧报告引用的 sheets.rs:382-422 旧实现已不存在。
2. **旧 §三.2「defringe 的 290°/90°/span25 在 Rust 任何位置都不存在」** — 矛盾。`DEFRINGE_PURPLE_CENTER=290.0/DEFRINGE_GREEN_CENTER=90.0/DEFRINGE_SPAN=25.0`（camera_raw.rs:855-857）、`apply_defringe_sample`（:1911-1926），且 app 吸管已接线（app/state.rs:1572-1577）。
3. **旧 §三.2「`applyCameraRawAutoWhiteBalance` 无对应」** — 矛盾（函数层面）。`apply_auto_white_balance` 现存于 camera_raw.rs:1931-1941；但 app 仍绕开它直接调 `auto_balance`（app/state.rs:1621），该函数本身沦为 test-only（本报告低-1）。
4. **旧 §三.3「levels 直方图的 selection coverage 参数 app 从不传」** — 矛盾。app/sheets.rs:174-182 现以 `histogram_coverage`（sheets.rs:747-761，内部走 `pixel::coverage_path`）传入 `levels_histogram`。
5. **旧 §四.1「`CameraRawSettings.apply` 全程不调用 is_valid」** — 矛盾。camera_raw.rs:1600-1602 已守卫。
6. **旧 §四.1「`CurvesSettings.apply` 无检查」** — 矛盾。curves.rs:95-97 已守卫。
7. **旧 §四.1「Exposure/GradientMap/BlackWhite/ColorBalance/Grain 的 apply 均无 is_valid 检查」** — 矛盾。image_adjustments.rs:125-127、:182-184、:318-320、:469-474、:602-607 均已守卫。
8. **旧 §四.2「Rust HueSaturation 逐像素 f64 直算、不走 33³ 立方体、Rust 无 33 这个常数」** — 矛盾。`CUBE_DIMENSION=33`（hue_saturation.rs:471），入口 `hue_saturation_apply` 走 `ColorCube::build` + 三线性 `apply_cube_rgba8`（mod.rs:35-41），并有立方体等价性测试（hue_saturation.rs:897-918）。
9. **旧 §四.3「pixel::coverage :25-33 有一段无效死代码（corners 循环 `let _ = (i, corner)`）」** — 矛盾。现 pixel.rs:16-38 无此代码。
10. **旧 §四.4「Rust blend 用整数 `(adj·a+orig·(255−a)+127)/255`」** — 矛盾（RGBA 主路径）。`blend_with_mask` 现为 f32 浮点混合（pixel.rs:94-99）；`blend_gray` 仍整数（本报告低-8，属灰度辅助路径）。
11. **旧 §四.6「Rust perspective warp 对越界坐标 clamp 到边缘、边缘外推与 CI 不同」** — 矛盾。`perspective_warp` 现显式把越界样本留为透明（camera_raw.rs:3325-3330），并有回归测试 `perspective_warp_leaves_off_image_samples_transparent`（:4176-4199）。
12. **旧 §四.7「App 层 Levels/HueSaturation 提交绕过选区」** — 矛盾。两个 sheet 的预览/提交现在都经 `self.blend_adjusted(...)`（app/sheets.rs:396、:433），其内部走 `pixel::blend_path`（app/sheets.rs:728-745）；滤镜路径也有选区混合（filters.rs:404-406）。
13. **旧 §五.5「sheets.rs:290 colorize 时 `saturation: saturation.max(1.0)` 把 colorize 饱和度下限抬到 1%」** — 矛盾。现为 `(saturation / 100.0).clamp(0.0, 1.0)`（app/sheets.rs:423），零保持为零。

旧报告中仍然成立的论断（核对无误，非矛盾）：Camera Raw UI 仅 14 个滑块（app/sheets.rs:292-310）；Hue/Saturation sheet 仅 Master 三滑块（app/sheets.rs:404-415）；`CameraRawScope.preview` 无对应——且如今 `CameraRawScope` 整体已成死代码（比旧报告说的更退步）；像素内核常数层面与项目自带 C 内核一致这一点与本报告读到的代码不冲突（本报告未逐常数对 C 源复核，仅确认注释与代码自洽）。

---

## 四、总结论

1. **adjust 子系统是全 workspace 里实现完成度最高的部分之一**：levels/levels_auto/curves/exposure/gradient map/black & white/color balance/grain/hue-saturation（含 33³ 立方体）均为真实像素算法且有守卫与测试；Camera Raw 的 12 个内核全部存在且参数真实生效——没有发现"只存参数不生效"的滑块（唯一例外是子滑块对主滑块的依附关系，如 `noise_luminance_contrast` 仅在 `noise_luminance>0` 时参与，这与 Camera Raw 本身语义一致）。
2. **唯一的高严重度缺陷是 Hue/Saturation colorize + 负色相**（hue_saturation.rs:422 缺负数回绕），UI 可达、静默产出错误颜色，建议立即修（一行 `hue = hue.rem_euclid(360.0)`）。
3. **中严重度集中在"名实不符"**：Guided Upright 的固定 ±25、无档案数据的 Lens Profile、仅水平位移的去色差、三种暗角样式只有两种行为、以及 Grain 调整层路径与"文档空间锚定"声明的矛盾。这些多数当前无 UI 入口或影响有限，但都会在 UI 补齐时变成用户可见的正确性问题。
4. **死代码量显著**（camera_raw 的 scope/applying/nudged/weights_for_hue/medium_contrast、pixel.rs 的 GuidedMatte 全套、hue_saturation 的波段编辑方法），且被 `#![allow(dead_code)]`（camera_raw.rs:4）与 `adjustment.rs:3` 的同类属性掩盖；其中 GuidedMatte 与 CameraRawScope 是完整实现却未接线的功能，应决定"接线或删除"。
5. **与旧报告 C 共 13 条实质性矛盾**，绝大多数是旧报告所述缺口已被修复（levels_auto 移植、defringe 吸管、is_valid 守卫、33³ 立方体、越界透明、选区接线等）；使用旧报告时必须以本报告行号为准。
