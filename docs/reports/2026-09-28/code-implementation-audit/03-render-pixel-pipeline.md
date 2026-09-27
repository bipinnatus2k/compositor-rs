# 渲染与像素管线实现审查（2026-09-28）

审查范围：`crates/core/src/{render,blend,filters,effects,effects_session,raster,export,raw}.rs`。
方法：全部 8 个文件逐行通读（非抽样）；关键结论经 grep 全工程验证调用点、`cargo test --no-run` / `--list` 实测；旧报告 `docs/agent-reports/F-render-effects-filters.md`（2026-09-24）仅作交叉核对，不作事实来源。所有行号来自当前 dev-ui-fix 分支实际代码。

## 总览表

| 文件 | 主要功能块 | 实际状态 |
|---|---|---|
| render.rs (1557 行) | 图层合成（变换/蒙版/效果/剪贴栈/调整图层/文件夹/混合） | **基本完整但有缺陷**（1 处可复现 panic 路径；3 个测试因代码结构错误从未运行；整画布临时分配的性能陷阱） |
| blend.rs (548 行) | 24 种混合模式 + PDF 合成 | **完整实现**（公式与 PDF/Photoshop 一致，仅文档数字漂移） |
| filters.rs (759 行) | 高斯/运动模糊、加噪、镜头校正等 13 种滤镜分发 | **基本完整但有缺陷**（真实卷积；运动模糊边界能量丢失 + 最近邻采样；种子硬编码在调用侧） |
| effects.rs (916 行) | 6 种图层效果渲染（阴影/发光/描边/叠加） | **基本完整但有缺陷**（真实算法非占位；蒙版尺寸不变量仅 debug_assert，release 可 panic） |
| effects_session.rs (536 行) | 效果面板会话状态机（添加/取消/复制/undo） | **完整实现**（未发现逻辑问题） |
| raster.rs (1121 行) | Bitmap/GrayMask、仿射绘制、Lanczos/双线性重采样、高斯模糊 | **基本完整但有缺陷**（算法真实；死代码、注释漂移、无条件整图 clone） |
| export.rs (328 行) | PNG/JPEG 导出、matte 压平、DPI 注入 | **完整实现**（错误静默吞掉为 Option::None） |
| raw.rs (162 行) | 相机 RAW 解码（rawler） | **完整实现**（薄封装，已经 session.decode_image 接线；失败静默） |

说明：render.rs:6、filters.rs:5、effects.rs:4、effects_session.rs:7 均有文件级 `#![allow(dead_code)]`，会掩盖死代码告警。

## 逐条发现

### render.rs

- **[高] 1×1 实心蒙版 + 任意启用效果 = 合成时越界 panic** — `crates/core/src/render.rs:695-701`（`gray_of` 对 `placed.pixel(i % width, i / width)` 直接索引）、调用点 `render.rs:442-444`。触发链全部为生产代码：`session.rs:1703-1720` `add_layer_mask` 写入 `LayerMask::solid`（`document.rs:264-277`，1×1 像素、placement=None）→ `render.rs:169-172` `mask_clip_image` 对 placement=None 原样返回 1×1 位图 → 只要该层像素大于 1×1 且带任一启用效果（如加个投影），`gray_of` 在 i=1 时索引 `data[4]`（Vec 长度 4）→ index out of bounds panic。UI 可达：`crates/app/src/layers_panel.rs:150`、`crates/app/src/workspace.rs:528`。`effects.rs:595` 的尺寸不变量只有 `debug_assert_eq!`，release 下无保护。另：mask 资产**大于**层像素时（placement=None 声称 1:1）`gray_of` 不 panic 但只读 mask 左上角区域，效果烘焙用错蒙版数据。
- **[高] 4 个测试嵌套错位，3 个从未被运行** — `crates/core/src/render.rs:1376-1457`。`shape_rectangle_fills_with_color`（1378）的函数体内嵌套声明了另外三个 `#[test]`：`effects_cache_hits_for_identical_input`（1379-1395）、`mask_clip_limit_caps_the_longest_side`（1397-1424）、`effects_guard_degrades_huge_results`（1426-1441），而它自己的测试体被挤到 1443-1456。代码能编译（嵌套 fn 合法），但 `cargo test -p compositor-core --lib -- --list` 实测只有 `shape_rectangle_fills_with_color` 与顶层的 `mask_clip_limit_caps_and_ceils` 被收集——上述 3 个测试**静默不运行**。其中 `effects_guard_degrades_huge_results` 是 effects 100MP 降级 guard（`effects.rs:587-589`）在工程里的唯一测试，等于该 guard 无测试保护。典型合并/编辑事故。
- **[中] `mask_clip_image_limited` 的交互降采样 limit 从未接线** — `crates/core/src/render.rs:157-225`。doc（155-156）声称 limit "caps the longest side while a transform is interactive"，但生产唯一调用 `render.rs:152` 传 `None`；传 `Some` 的只有测试（1108、1421）。Swift `LayerMask.clipImage` 的交互期降采样实际未接入任何变换流程。
- **[中] 每层每帧两份整图拷贝 + 每层一张整画布临时位图** — `crates/core/src/render.rs:427`（`asset.image.as_ref().clone()` 整层像素克隆）叠加 `raster.rs:627`（`draw_affine` 内再 `src.clone()`，即使 Smooth/Nearest 不需要预缩小也克隆）；`render.rs:498` 每层新建目标画布大小的 `placed`；`render.rs:343` 每个剪贴源再渲染一张画布级 coverage 位图。100MP 画布 × N 层时每帧瞬时分配达数百 MB～GB 级。
- **[中] 取一个像素颜色要完整重合成整个文档** — `crates/core/src/render.rs:827-843`，831 行调用 `draw_live_composite`（全图层、全效果重渲染）只为读一个像素；吸管拖动时每帧触发（`crates/app/src/canvas.rs` 调用）。
- **[低] 效果缓存以 `Arc::as_ptr` 作像素标识** — `crates/core/src/render.rs:446-449`、缓存定义 21-22。地址复用（ABA）可致陈旧命中；`composite_onto` 每轮开头清空（255）缓解了单轮内的问题，但缓存满 8 条整体清空（44-50）在 >8 个效果图层时退化为每轮全量重渲染。
- **[低] `sample_grid_alpha` 的 transform 参数被显式丢弃** — `crates/core/src/render.rs:690`（`let _ = transform;`）。另注意它对 mask 网格越界坐标依赖 `bilinear_sample` 的钳位外延（691）而非返回 0——1×1 实心蒙版因此"碰巧"正确，而 mask 小于层网格时边缘值被拖尾。
- **[低] 圆角矩形圆角为 4 段折线近似** — `crates/core/src/render.rs:760-768`（`FRAC_PI_2 / 4.0`，每 45° 一个顶点），大尺寸形状可见多边形感；有内联注释承认，但无文档声明精度。
- **[低] 线条抗锯齿为线性近似** — `crates/core/src/render.rs:812`（`coverage = (radius - d + 0.5).clamp(0,1)`），非面积覆盖率。

### blend.rs

- 24 种模式全部为真实实现：可分离式（`blend.rs:162-247`）、非可分离 Hue/Sat/Color/Luminosity（322-334，含 PDF 的 clip_color/set_lum/set_sat 249-320）。三处旧报告点名的错误均已修复：`composite_pixel` 现为完整 PDF 式 `co = αs(1−αb)·Cs + αsαb·B + αb(1−αs)·Cb`（369-372）；HardMix 为 `Cb+Cs < 1 → 0`（224-231，测试 448-457 专门覆盖边界）；Divide 零源返回白（235-242，测试 468-472）。
- **[低] 文档声称 25 种模式，实际 24** — `crates/core/src/blend.rs:3`（"All 25 modes"）vs `blend.rs:64`（`ALL: [LayerBlendMode; 24]`）及测试 544。
- **[低] `b_separable` 对非可分离模式 `unreachable!`** — `blend.rs:243-245`。当前仅私有内部调用且已被 `blend_color`（337-350）分流，安全，但属潜在 panic 路径。
- **[低·备案] `clip_color` 负分支无除零护栏** — `blend.rs:257-264`（`l_n = l - n` 为 0 需全通道相等且为负，合法 u8 输入不可达；正分支 267-269 有护栏，两处不对称）。

### filters.rs

- 滤镜主体为真实算法：Add Noise 是逐像素位置哈希 + Box–Muller（229-266），Lens Correction 是真实径向畸变双线性逆映射（286-339），Gaussian Blur 有真实的 grow→blur→trim（371-387、512-521），均非占位。
- **[中] motion_blur 边界能量丢失** — `crates/core/src/filters.rs:441-459`。出界 tap 直接 `continue`（449-451）且不按剩余权重重新归一化，累计进 `acc`（初值 0）：靠近边界处像素沿 streak 方向把 alpha/颜色衰减向透明黑；若窗口内所有 tap 出界则输出全 0（461-463 无条件覆写）。与 CI 的边缘延展语义不同，长 streak 时边界出现渐隐条纹。
- **[中] motion_blur 为整像素最近邻采样** — `filters.rs:447-448`（`(x + 0.5 + dx).floor()`），无亚像素插值，斜向角度呈阶梯状（旧报告二.9 的"亚像素采样不同"这部分仍然成立）。
- **[中] 实现与文档不符：声称 "even streak"，实为高斯 taper** — `filters.rs:410-412` doc 写 "an even streak of length d"，426-439 实现按 `exp(−t²/2σ²)`（σ=radius）加权；`filters.rs:439` `let weights = weights;` 为无效自赋值语句。
- **[中] Add Noise 种子硬编码，"随机图案"声明落空** — `filters.rs:220-221` `FilterJob.seed` 文档写 "Add Noise's random pattern"，但生产调用全部硬编码 `seed: 7`（`crates/app/src/state.rs:798`、`state.rs:1565`）；`session.rs:2462` 的 `rand_seed()`（5800）只服务 grain 调整图层路径。结果：任何图层、任何文档的每次 Add Noise 图案完全相同，与 Swift 每次随机（旧报告 F 一.9 引 Filters.swift:223）不符。
- **[低] GaussianBlur 的 pad=3σ 小于模糊核半径 4σ** — `filters.rs:380` vs `raster.rs:682`。但钳位发生在透明 pad 上、等效零填充，数值影响可忽略；372-378 注释"clamp 只落在 pad 里"在字面上不严谨（clamp 落在 pad 的复制值上，仍是透明）。
- **[低] 无效测试 `resample_alias_used`** — `filters.rs:756-758`，仅 `let _ = LayerSampling::High;`，零断言。
- **[低] `FilterSettings::default` 依赖 `serde_json::from_str("{}").unwrap()`** — `filters.rs:163-167`。常量输入必成功，但以 unwrap 承担序列化格式正确性。

### effects.rs

- 效果是真实算法而非近似占位：`extreme` 单调队列滑动窗口（508-548，即膨胀/腐蚀）；描边=形状与其膨胀/腐蚀的差环（679-699）；外/内发光、内外阴影=coverage 与模糊 coverage 的组合（652-676、717-747）；展开顺序 shadow→外发光→外描边→像素→color overlay→内发光→内阴影→内描边（652-752）；100MP 结果 guard（587-589）；`effect_margin`（757-772）与 `placed_transform`（775-791）公式自洽（margin blur×3 ≥ 核半径 2×blur，有余量）。
- **[中] 蒙版尺寸不变量仅 `debug_assert`** — `crates/core/src/effects.rs:595`。release 下 `m.data[i]`（596-609）对更小的 mask 直接越界 panic（与 render.rs 发现 1 同一事故的两端）；更大则静默错位。
- **[低] coverage 采样为最近邻** — `effects.rs:553-569`（`sx = (u * width) as usize`）。内联注释声明对齐 Swift `BrushRaster.draw`（CPU 路径），与旧报告 F 一.10 的记载一致；代价是层被缩小显示时效果边缘锯齿。
- **[低] 投影渲染测试不断言任何像素** — `effects.rs:865-884`，883 行 `let _ = result;`，仅断言 inset。

### raster.rs

- 核心算法真实：`halve` 为 Lanczos + 尺寸向上取整（421-428，注释 423-424 也改口承认 Lanczos）；`downsample_level` 严格 `factor < 0.5` 才启用（413-419，测试 1011-1034 固化）；`resample` 缩小末遍双线性、放大 High 走 Lanczos（387-394）；`draw_affine` 有整数对齐快速路径（645-650）与预缩小（629-641）；premultiply/unpremultiply（150-169）与 `source_over_u8`（592-607）正确。
- **[中] `draw_affine` 无条件克隆整张源图** — `crates/core/src/raster.rs:627`（`let mut source = src.clone();`）。克隆仅在 High 预缩小时被修改（634-641）；Smooth/Nearest 路径纯浪费。与 render.rs:427 的克隆叠加，见前。
- **[中] `lanczos_sample` 每像素 36 tap 现场计算 sin/cos** — `raster.rs:432-468`（内层 `lanczos3` 528-538 每权重两次 `sin`）。High 放大路径（`draw_affine`:662-663）逐像素触发，大图极慢；权重不缓存、无水平/垂直分离。另 `lanczos_resize`（471-525）对 `sum` 无 ≈0/负值护栏（493-496、517-520），NaN 经 `as u8` 饱和为 0，不 panic 但可产生局部异常像素。
- **[低] 高斯模糊 O(w·h·8σ) 无加速近似** — `raster.rs:678-717`，半径 4σ（682），σ 上限 250（滤镜 clamp）→ 单边 2001 tap，大图最坏分钟级。真实卷积，但无 box 三遍近似。
- **[低] `gaussian_blur_rgba_unclamped` 为死代码** — `raster.rs:723-770` 全工程无调用（grep 证实）；"CI 透明边界"语义实际由 `filters.rs:371-387` 的 pad+clamp 等效实现。
- **[低] `halve` 一行注释仍是 "box-filter halving step"** — `raster.rs:421` vs 427 行 Lanczos 实现；测试名 `halve_averages_quads`（976-984）同样名不副实（注释漂移，非行为错误）。
- **[低] `clamp_mirror_index` 名为 mirror 实为钳边复制** — `raster.rs:803-814`。
- **[低] `draw_affine` 开头死存储** — `raster.rs:619` 的 `inverse` 在 642 行被无条件覆盖。
- **[低] `draw_stretched_nearest` 无半像素中心对齐** — `raster.rs:550-555`（`u = (x - min)/w`，无 `+0.5`），与 `draw_over`（577-581）的中心采样约定不一致，映射有半像素系统偏移。
- **[低] `set_straight_f32` 无调用、`straight_f32` 仅测试使用** — `raster.rs:95-97`、89-92（grep 全工程证实）。

### export.rs

- PNG/JPEG/matte 压平/DPI 注入均为真实实现且测试充分：`flatten_on_matte` 预乘数学正确（42-66）、`inject_png_dpi` 手写 pHYs+CRC 正确（88-113）、`patch_jfif_density` JFIF 布局偏移正确（164-182）。
- **[低] 编码错误静默吞掉** — `export.rs:70-77`（`write_to(...).ok()?` → None）、`130-152`（149 行同样）。导出失败时调用方拿不到原因。
- **[低] unpremultiply 用整数截断** — `export.rs:206`（`(v * 255) / alpha` 截断而非四舍五入），1/255 级精度损失；210 行 `expect` 由构造保证不触发。

### raw.rs

- 已真实接线：`is_raw`/`decode_raw`（16-33）由 `session.rs:5779-5781` 的 `decode_image` 调用，DNG 走 rawler 默认 develop。与注释声明一致（7 行明确说 develop 滑杆未移植）。
- **[低] 解码失败静默折叠为 None** — `raw.rs:28`（`.ok()?`），导入端只报通用"无法读取"。

### effects_session.rs

- 完整实现，未发现缺陷：添加效果带默认外观与背景色（62-110）、取消只还原本 kind 且新增效果被移除（151-186，测试 394-432）、复制前先提交目标层的编辑防止 Cancel 波及（278-280）、拒绝 folder/无像素/非法值并归一化空效果（190-204）。App 侧调用齐全（`crates/app/src/layers_panel.rs:202`、`sheets.rs:200,617-623` 等）。

## 与旧报告 F 的矛盾点（旧报告论断 vs 当前代码）

共 **18 条**明确矛盾（旧报告说"缺失/错误"，当前代码已实现/已修复）：

1. F 二.1"调整图层作用于自身像素" → 矛盾：现为作用于合成内容之下的画布，带自身蒙版裁剪与有效不透明度混合（`render.rs:542-610`，测试 1209-1286）。
2. F 二.2"`composite_pixel` 缺 αs(1−αb)Cs 项" → 矛盾：完整 PDF 式（`blend.rs:369-372`，测试 475-485）。
3. F 二.3"HardMix 用 VividLight，cs>0.5 时错" → 矛盾：现为 Cb+Cs 阈值（`blend.rs:224-231`，测试 448-457）。
4. F 二.4"Divide 零源黑底得黑" → 矛盾：cs≤0 恒为白（`blend.rs:235-242`，测试 468-472）。
5. F 二.5"蒙版+效果双重蒙版/越界采样错位" → 矛盾：效果烘焙蒙版后 `own_mask = None`，效果图以无蒙版绘制（`render.rs:440-459`，测试 1330-1375）。
6. F 二.6"剪贴源 coverage 不含效果/蒙版/不透明度" → 矛盾：`render_coverage` 完整 `draw_layer_full`（`render.rs:326-347`）。
7. F 二.7"剪贴层各自乘 alpha、不要求相邻同父" → 矛盾：`prepare_stacks` 强制相邻+同父，整栈共享 surface 与 base 混合模式（`render.rs:379-404、616-651`，测试 1305-1327）。
8. F 二.8"高斯模糊边缘复制、无 pad/trim" → 矛盾：`run_filter` 内建 grow→blur→trim（`filters.rs:371-387`）。
9. F 二.9"motion blur 均匀 box 核" → 矛盾：现高斯 taper 权重（`filters.rs:426-439`）。（子项"最近邻采样"仍成立，见上文。）
10. F 二.10"halve 为 2×2 box 平均、w/2 向下取整" → 矛盾：现为 Lanczos、(w+1)/2 向上取整（`raster.rs:421-428`，测试 874-894）。
11. F 二.11"scale≤0.5 即 halve，差一档" → 矛盾：`factor >= 0.5 → level 0`（`raster.rs:413-419`）。
12. F 二.12"插值不区分缩小 Low/放大 High" → 矛盾：缩小末遍强制双线性、放大 High 用 Lanczos（`raster.rs:387-394、657-666`，测试 1037-1086）。
13. F 一.3"无 DownsampleCache level 函数" → 矛盾：`downsample_level` 存在且逐条对齐 Swift 分档（`raster.rs:413-419`，测试 997-1008）。（层级 LRU 缓存仍无——每帧重算 halving，属部分修复。）
14. F 一.4"LiveMaskRenderer（剪贴栈/coverage/防环）完全缺失" → 矛盾：stacks/coverage/256 层防环均在（`render.rs:326-404、616-651`，防环在 335）。
15. F 一.7"无 EffectsPreviewCache" → 矛盾：`EFFECTS_CACHE` + 每轮失效（`render.rs:17-56、255`）。
16. F 一.11"文件夹不透明度缺失" → 矛盾：`effective_opacity` 逐祖先连乘（`render.rs:352-373`，测试 1289-1302）。
17. F 三.2"render_effects 无 100MP guard" → 矛盾：guard 在（`effects.rs:587-589`）——但其唯一测试因发现 2 从未运行。
18. F 三.3"mask_clip_image 无 limit、无 placement 缓存" → 矛盾：`MaskPlacementCache`（LRU+像素预算）与 `limit` 参数均在（`render.rs:69-225`）；limit 生产未接线（见上文中危项），属部分修复。

旧报告仍然成立的点（核实后无矛盾）：TiledLayerRenderer/瓦片机制仍缺失（render.rs 全图单次 `draw_affine`，无 Piece/patches 结构）；`run_filter` 的生产调用方仍全部在 app 层（`state.rs:804/1555`、`workspace.rs:937`；core 内 `session.rs:7772` 位于测试模块）；**seed 硬编码 7 依旧存在**（`state.rs:798/1565`，即 F 一.9 的种子问题未修）。另 F 一.2（RasterSnapshot）属 brush 域，本次未审。

## 总结论

- 该管线**没有 stub**：混合模式、滤镜、图层效果、重采样、导出、RAW 全部是真实算法，且旧报告 F 列出的 12 项行为差异中 10 项已在此后的迭代中修复。当前代码质量显著好于 2026-09-24 报告所描述的状态。
- 本次审查的**两个高严重度问题都是"接线/结构"问题而非算法问题**：① 1×1 实心蒙版（UI 一键创建）+ 任意启用效果会在合成时必然 panic，根因是 placement=None 的"mask 与层像素 1:1"这一不变量只在 debug_assert 里、且 `gray_of` 用直接索引；② render.rs 测试区一处合并事故使 3 个测试（含 100MP guard 的唯一测试）静默退出测试集。
- 中危集中在边界与声明不符：motion_blur 边界能量丢失与最近邻采样、Add Noise 固定种子、`mask_clip_image_limited` 的交互降采样未接线、整图克隆的性能陷阱、`lanczos_sample` 逐像素三角函数。
- 修复优先级建议：发现 1（panic）≥ 发现 2（恢复测试）> motion_blur 边界/种子 > 绘制路径克隆与 Lanczos 性能。
