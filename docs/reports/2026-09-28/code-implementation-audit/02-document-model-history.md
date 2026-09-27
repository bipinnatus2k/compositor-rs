# 02 · 文档模型 / 历史 / 画布几何 实现审查（2026-09-28）

审查范围：`crates/core/src/{document,canvas,history,adjustment,color_picker,geometry,lib}.rs`（约 4100 行）。
方法：全部文件逐行读完；所有结论来自本次实读代码并给出真实行号；旧报告 A/B 仅作交叉核对，不作为事实来源。所有死代码结论均在 `crates/` 下 grep 验证过调用点。未修改任何源码。

## 总览表

| 文件 | 主要功能块 | 实际状态 |
|---|---|---|
| document.rs | 颜色模型 / 参考线 / 网格 / 蒙版 / 文本 / 形状 / 图层 / 文档 / 层级遍历 / 工具枚举 | 基本完整但有缺陷（1 个从未运行的测试、2 个死 pub 方法、O(n²·depth) 可见性计算、无效断言） |
| canvas.rs | 画布尺寸 / 图像尺寸重采样 / 裁剪几何 / 裁剪吸附 / 修边 / 翻转变换 | 基本完整但有缺陷（**1 个高严重度蒙版重采样 bug**、残留调试代码、重复闭包、预算下溢风险） |
| history.rs | 快照式 undo/redo、事务嵌套、字节上限裁剪 | 基本完整但有缺陷（蒙版内存估算与实际存储不符，差 4 倍） |
| adjustment.rs | 调整种类枚举 / 设置存取 / 校验 / 应用分发 | 完整（region 与 sampling_margin 是无调用方的死接口） |
| color_picker.rs | 取色器会话 API（开/关/预览/提交） | 完整（GradientMap 分支是自认 stub 且不可达；can_edit_palette 恒 true 是自认 stub） |
| geometry.rs | CGPoint/CGSize/CGRect/CGAffineTransform | 完整实现（与 CoreGraphics 语义一致；2 个死 pub 方法） |
| lib.rs | 模块声明 | 声明正确；"In progress" 注释已过时 |

## 发现

### canvas.rs

- **[高] 图像尺寸重采样：就地非均匀蒙版被双重变换，蒙版内容整体错位/放大** — `crates/core/src/canvas.rs:359-384`。第 359 行先把 `layer.transform` 替换为新（缩放后）包围盒，随后第 374-377 行构建蒙版的源映射 `source_transform = LayerTransform { sampling, ..layer.transform }` 时读到的是**新** transform；而蒙版像素网格对应的是**旧**图层盒。`pixel_to_document`（`crates/core/src/raster.rs:271-279`）按 `transform.size / 图像尺寸` 计算缩放，再被第 378 行的 `scaling.concat(..)` 再乘一次 sx/sy —— 2 倍放大时蒙版内容被映射到 4 倍缩放、2 倍偏移的位置，绝大多数落在输出位图之外（蒙版重采样后近乎全黑/全空）。对照资产路径：第 331-342 行在赋值前捕获旧 transform，是正确的；蒙版路径漏了这一步。默认状态的蒙版（由选区创建、未单独移动过、尺寸 >1×1）恰好全部命中此分支。测试只覆盖均匀/已分离蒙版（`crates/core/src/canvas.rs:905-919`），此路径无任何测试。入口 `EditorSession::resize_image_to`（`crates/core/src/session.rs:5375-5393`）直通 app 的 Image Size 功能，可由用户触发。
- **[中] CropSnap::nearest 残留调试代码，拖拽热路径每次查环境变量** — `crates/core/src/canvas.rs:519-521`。`if std::env::var("CROP_DBG").is_ok() { eprintln!(...) }` 是调试脚手架遗留：`nearest` 在每次鼠标移动中对每个吸附目标调用一次（目标数随图层/画布边缘数增长），每次都做环境变量查找；设置该变量时会向 stderr 刷屏。应删除。
- **[中] CropSnap::apply 内定义了两个同名 `shift` 闭包，第一个是死代码且语义错误** — `crates/core/src/canvas.rs:551-562` 与 `563-575`。第一个闭包把 `nearest` 返回的目标**绝对坐标**当作位移 delta（错误语义），随即被第二个闭包遮蔽（第二个正确计算 `target - edge`，第 576-577 行调用的是第二个）。第一个若被启用会把 Move 吸附变成灾难性偏移；目前只是迷惑性死代码 + 编译期 unused 警告，应删除。另第 594 行 `_ => (false, false)` 是不可达的穷举臂（Move 已提前 return）。
- **[中] resize_canvas 填充预算存在 u64 下溢：debug 恐慌 / release 预算失效** — `crates/core/src/canvas.rs:190-196`。`used` 由现有图层资产像素数累加，若其超过 1 亿（加载期 store 校验可保证 ≤1 亿，但会话内的导入/画笔路径无此保证），`100_000_000 - used` 在 debug 下整型溢出 panic，release 下回绕成极大数导致预算检查恒过，随后第 207 行可分配最高 30000×30000 的 RGBA 位图（约 3.6 GB）。
- **[低] CanvasSizeDraft 全套是死代码** — `crates/core/src/canvas.rs:29-93`。全仓 grep 仅本文件（定义+测试）引用，app 的 Canvas Size 面板未使用；`#![allow(dead_code)]`（第 5 行）把警告压掉了。
- **[低] trim 把内部错误吞成"无内容可裁"** — `crates/core/src/canvas.rs:800`。`resize_canvas(document, &options).ok()` 把"图层越界"等 Err 静默转成 None，session 层（`crates/core/src/session.rs:5399-5409`）据此向用户报"没有可修剪的内容"，掩盖真实失败原因。
- **[低] 文件级 `#![allow(dead_code)]`** — `crates/core/src/canvas.rs:5`。掩盖了 CanvasSizeDraft 等真实死代码，建议移除后逐项清理。

### history.rs

- **[中] 蒙版内存按 1 字节/像素估算，但实际存储是 4 字节 RGBA——注释声明的表示与代码不符** — `crates/core/src/history.rs:165-168`（注释称"a mask is a DeviceGray image whose bytesPerRow is one byte per pixel"）与 `191-197`（按 `width * height` 计 1 字节/像素，第 194 行）。实际蒙版是 RGBA `Bitmap`：`crates/core/src/document.rs:264-270`（`LayerMask::solid` 构造 4 通道像素）、`crates/core/src/canvas.rs:213`（`chunks_exact_mut(4)` 证明 4 字节/像素）。Swift 原版蒙版确是 DeviceGray，估算在当时成立；Rust 移植改成 RGBA 后实际内存是估算值的 4 倍，`retained_byte_limit`（默认 256 MB，`history.rs:40`）对蒙版密集历史的实际约束力弱 4 倍。测试 `masks_retain_one_byte_per_pixel`（`history.rs:273-302`）把错误估算固化成了断言。
- **[低] is_modified 对不变量 unwrap** — `crates/core/src/history.rs:76-78`。`saved_revision` 类型是 `Option<Uuid>` 但所有赋值路径均为 `Some`（45-58、84-86、88-95），`unwrap()` 依 invariant 安全，但 Option 包装已无意义，属脆弱写法。
- **[低] EntryClone trait 完全等价于 derive(Clone)** — `crates/core/src/history.rs:219-231`。手写实现逐字段 clone，与 `#[derive(Clone)]` 无差别（Entry 因含 Snapshot 而未 derive 是历史原因），徒增噪音。
- **[低] trim 循环反复全量重算 retained_bytes** — `crates/core/src/history.rs:204-216`。while 循环每轮调用 `retained_bytes`（169-202，遍历全部条目×全部图层并重建 HashSet），最坏 O(条目²×图层数)；上限 100 条目时通常无感，属可接受的低效。

### document.rs

- **[中] effective_visible_ids 最坏 O(n²·depth)，在万层文档上单击即卡顿** — `crates/core/src/document.rs:701-728`。已建好 `parents` HashMap（702-703）却未使用，祖先上溯每步调用 `self.layer(id)`（687-689，线性扫描全表）：n 层 × 64 层深 × O(n) = O(64n²)。在 store 校验允许的 10,000 层上限下约 6.4e9 次比较。调用点为事件处理器（`crates/core/src/session.rs:2071、2254、4261`，如吸管/文本命中测试），非每帧，但一次点击即可秒级卡死。修复只需改用 map 上溯。
- **[低] 测试 palette_hex_roundtrip 缺少 #[test] 属性，从未运行** — `crates/core/src/document.rs:943-951`。位于 `mod tests` 内但无 `#[test]`（对照第 953 行的下一个测试），是无调用方的死函数；其中断言本可全部通过，等于白写的测试。
- **[低] 无效断言（恒真）** — `crates/core/src/document.rs:970`。`assert!((hsb.saturation - 0.5/0.5 + 1.0).abs() > 0.0 || hsb.saturation == 0.0)` 对任意 saturation ≥ 0 恒真（|s|>0 或 s==0 二者必居其一），不构成任何校验；真正有效的是第 971 行。
- **[低] CanvasDocument::valid_dimension 是死 pub 方法** — `crates/core/src/document.rs:679-685`。全仓仅定义+自身测试引用，session/app 用各自的校验。
- **[低] LayerMask::background_of 对空缩略图返回 1.0（全白）** — `crates/core/src/document.rs:305-321`。width/height 为 0 时 count=0，第 316 行 `0*2 >= 0*255` 成立返回 1.0；当前调用链不会传入空图，属边界瑕疵。
- **[低] NavigationTool::ALL 不含 Idle** — `crates/core/src/document.rs:830-869`。变体定义了 Idle（848-849）但不进 ALL 数组；遍历 ALL 的代码（如工具栏枚举）看不到它。与 Swift `allCases` 含 idle 的记载（旧报告 A 行为差异 16 前半）仍不一致，本审查未读 Swift 原文，仅按旧报告记录存疑。

### adjustment.rs

- **[低] apply 的 region 参数在生产代码中恒为 None，语义只覆盖位置相关调整** — `crates/core/src/adjustment.rs:321-371`。文档注释（321-322）宣称支持区域应用，但全仓仅两处调用且都传 None（`crates/core/src/render.rs:552`、`:875`）；region 只对 Grain（342-348）和 AddNoise（364-366）生效，其余分支静默忽略。行为正确但注释夸大了可用面，且是一条约定的死接口。
- **[低] sampling_margin 是死方法，注释描述的部分重绘 halo 能力无人使用** — `crates/core/src/adjustment.rs:270-278`。注释称"部分画布重绘需要超出脏矩形的采样边距"，但全仓无调用方（render.rs 的效果边距走 `placed_transform`，`crates/core/src/render.rs:456` 一带，与模糊蒙版无关）。仅测试引用。
- **[低] is_valid 对所有种类的设置做交叉校验** — `crates/core/src/adjustment.rs:280-319`。例如 Hue/Saturation 图层也会被未使用的 curves/levels 载荷判废（303-310），而渲染端以 `is_valid` 为门（`crates/core/src/render.rs:285`）；被无关字段污染的合法调整层会被整体跳过渲染。默认构造下各设置均合法，仅病态数据触发。
- **[低] AdjustmentKind::is_editable 仅测试引用** — `crates/core/src/adjustment.rs:81-83`。生产无调用点（由 `#![allow(dead_code)]`（第 3 行）压制警告）。

### color_picker.rs

- **[中] GradientMap 目标的提交分支是自认 stub，且该目标在核心中不可达** — `crates/core/src/color_picker.rs:167-173`。注释承认"not portable yet：核心没有 Gradient Map 可写入的 filter-edit 会话"。全仓 grep 确认没有任何代码构造 `ColorPickerTarget::GradientMap`（仅 `crates/core/src/session.rs:230` 为它生成标题字符串）。属于已声明的未实现功能：渐变映射取色器整体不可用。
- **[低] can_edit_palette 恒返回 true（自认 stub）** — `crates/core/src/color_picker.rs:24-27`。注释如实说明 busy/笔刷状态在 app 层，app 侧确有合取（`crates/app/src/color_picker_ui.rs:71`）；但核心内 `set_palette_color`/`open_*` 的守卫因此全部是空操作，若 app 层新增入口漏检即失去保护。
- **[低] 文本草稿身份是近似实现（已如实注释）** — `crates/core/src/color_picker.rs:9-17`。用 layer_id/document_id 代替 Swift 的每草稿 UUID，注释准确指出了"同一图层上取消并重开 Type 工具且取色器保持打开"这一例外窗口。

### geometry.rs

- **[低] CGPoint::hypot 是死 pub 方法** — `crates/core/src/geometry.rs:28-30`。全仓 grep `CGPoint::hypot` 零调用（各处用的是 `f64::hypot`）。
- **[低] CGRect::is_infinite 是死 pub 方法** — `crates/core/src/geometry.rs:123-128`。全仓仅定义处引用。
- **[低] inverted() 对奇异矩阵返回自身（与 CoreGraphics 一致）** — `crates/core/src/geometry.rs:320-325`。与 `CGAffineTransformInvert` 行为一致，属忠实移植；但调用方（如 `transform.rs:186` 的 `following`）拿到错误的"逆"时不会有任何信号，仅作记录。
- 其余核对：`concat` 语义（304-318）与文档声明一致（`p' = M_self·M_other·p`）；`intersection`（160-172）、`integral`（190-199）、`contains`（219-221）均与 CGRect 语义相符，未发现公式错误。

### lib.rs

- **[低] "In progress" 注释已过时** — `crates/core/src/lib.rs:28`。该注释覆盖其后全部 13 个模块（adjust/project/psd/render/session/tools 等），而其中多数（project v1-9 校验、render、session 操作）已具相当完整度；模块清单本身（19-41）与磁盘文件一一对应，无缺失。

## 与旧报告 A/B 的矛盾点

以下条目为旧报告论断与当前代码不符之处（旧报告位置 → 当前代码行号）。共 14 条（A 7 条、B 7 条）。

**报告 A（docs/agent-reports/A-document-session-history.md）**

1. A 行为差异 1（第 85 行）称"Rust history.rs:186 一律 width*height*4 → 蒙版更早触发上限" — 现代码已区分：蒙版按 1 字节/像素计（`history.rs:191-197`）。方向反了过来：因实际存储是 RGBA，现反而**低估** 4 倍（见上文 [中] 发现）。
2. A 行为差异 2（第 86 行）称"ImageLayer/ImportedImage 是 derive 的值相等（含像素数据）" — 现为手写 PartialEq，asset.image 按 Arc 身份比较（`document.rs:542-564`），LayerMask/LayerText 同（254-261、332-336）。已修复，A 过时。
3. A 行为差异 14（第 98 行）称"is_valid 完全没有 hsv_settings 的检查" — 现在 `adjustment.rs:288-302` 完整校验 range adjustments 与 band handles（含 NaN）。已修复，A 过时。
4. A 行为差异 16（第 100 行）称"Idle 单独定义了 label 'No tool (A)'" — 现在 Idle 与 Zoom 共用 "Zoom (Z)"（`document.rs:905-907`），注释声明对齐 Swift 的落空链。已改；但"ALL 不含 Idle"的前半仍属实（见上文 [低]）。
5. A 缺失 36（第 65 行）称"颜色选择器全套缺失" — 现在 `color_picker.rs` 全文实现了 open/close/cancel/preview/type_color/effect 目标及 undo（仅 GradientMap 目标是 stub）。A 过时。
6. A 第四.多出（第 124 行）称"history.rs:381 transform_not_saved_blocks_nothing 是无效断言空测试" — 现 `history.rs` 已无该测试（现 12 个测试中无此名），引用失效。
7. A 测试缺口说明（第 130 行）称"crates/core/tests 为空" — 现有 clipboard_group_tests.rs、selection_edit_tests.rs、transform_session_tests.rs、tool_flow_tests.rs、adjustment_layer_tests.rs、type_tool_tests.rs 等。A 过时。

**报告 B（docs/agent-reports/B-transform-canvas-project.md）**

1. B 缺失 6（第 21 行）称"fill（Canvas Extension）整套缺失、无使用者" — 现在 `canvas.rs:187-250` 完整实现：像素预算、图层数预算、颜色校验、底层的旧画布区透明化、测试覆盖（`canvas.rs:931-950`）。已实现，B 过时。
2. B 缺失 7（第 22 行）称"resize_canvas 无任何校验、同尺寸时清空 selection" — 现在 `canvas.rs:146-168` 有尺寸/anchor/offset 校验与同尺寸零偏移的保选区早退。已实现，B 过时。
3. B 行为差异 1（第 28 行）称"旋转层做线性缩放并保留 rotation/flip" — 现在 `canvas.rs:301-319` 按 4 角投影包围盒（floor/ceil）重建轴对齐 transform，rotation/flip 归零。已修复，B 过时。
4. B 行为差异 2（第 29 行）称"1×1 均匀 mask 被放大、placed mask 线性缩放 origin/size" — 现在 `canvas.rs:360-368` 均匀蒙版保留像素、placed 蒙版用 `placing(unit_to_document ∘ scaling)`。已修复；但重写引入了 B 未见的**新 bug**（就地非均匀蒙版双重变换，见上文 [高]）。
5. B 行为差异 6（第 33 行）称"resolution 与总像素预算不校验、同尺寸无捷径" — 现在 `canvas.rs:268-287` 三者齐备。已修复，B 过时。
6. B 行为差异 7（第 34 行）称"硬编码 LayerSampling::High、API 无 sampling 参数" — 现在 `resize_image` 接受 sampling 参数（`canvas.rs:261-267`，第 319/334/375 行下发）。已修复，B 过时。
7. B 缺失 2（第 17 行）称"CropSnap 无对应、drag_crop 不吸附" — 现在 `canvas.rs:506-654` 完整实现 CropSnap，且已接线（`crates/core/src/session.rs:5266` 调用 `snap.apply`）。已实现，B 过时。

未矛盾的旧论断（抽查）：B 简化 2"trim 经 draw_live_composite"（现 `canvas.rs:787`，属实）；A 简化 1"LayerMask 为 RGBA 表示、只移植 placement_moving_layer/background_of"（现 `document.rs:243-321`，属实）。

## 总结论

- 7 个文件中没有一个是从零 stub：数据模型、历史内核、几何原语、裁剪/修边数学、取色器会话都是真实实现且有测试。两处 `#![allow(dead_code)]`（canvas.rs:5、adjustment.rs:3）之下埋着约 8 个死 pub 项，建议清理。
- 唯一的高严重度问题是 canvas.rs 图像尺寸重采样对**就地非均匀蒙版**的双重变换（`canvas.rs:359-384`）：这是 2026-09-24 之后 ImageResizer 重写（修复了旧报告 B 列出的一批偏差）时引入的新缺陷，常见蒙版状态（选区创建、未移动）在 Image Size 后蒙版内容会大面积错位失效，且该路径零测试覆盖。
- 中严重度集中在"估算/守卫与实现脱节"：历史蒙版内存按 Swift 的灰度假设估算而实际是 RGBA（差 4 倍）、万层文档的 O(n²) 可见性计算、resize_canvas 预算下溢、CropSnap 的调试残留与重复闭包、GradientMap 取色器 stub。
- 旧报告 A/B 关于本域的核心"缺失"论断已大面积过时（14 处矛盾）：canvas.rs 的 fill/校验/重采样/吸附、color_picker.rs 整体、adjustment 的 HSV 校验均在 09-24 后补齐或重写。引用这两份报告时必须以当前代码为准。
