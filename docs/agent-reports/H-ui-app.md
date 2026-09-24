> 子代理原始报告（域 H：UI / 应用壳），逐字保留，2026-09-24。

# UI / 应用壳功能域：Rust 版 vs Swift 版 深度对比报告

## 总体结论

Rust 版（crates/app，共 12 文件 5610 行）实现了 Swift 版 UI 壳的**骨架**：工具栏 + 每工具选项头 + 画布视口 + 图层面板 + 少量 sheet + 菜单 + 一部分快捷键。但相对 Swift 版（UI 目录 41 文件 + 应用壳约 7900 行）存在**大面积缺失**：剪贴板四件套（Cut/Copy/Copy Merged/Paste）、导出 JPEG、Curves 等 6 个图像调整面板、Camera Raw 的 9 个子面板、图层效果/调整层面板、颜色选择器、PSD 转换表、RAW 开发表、多项目多窗口、原生文件对话框、快捷键自定义系统等全部缺失。SwiftUI 文件内嵌的业务逻辑（CanvasThumbnail 渲染、标尺刻度、HSB 换算等）多数已有 core 层移植，但**入口 UI 层缺失或不等价**。Rust 侧 app crate **没有任何 #[test]**，Swift 的 7 个 UI 相关测试文件共 30 个用例，Rust 仅移植了其中 3 个（且在 core 而非 app）。

说明：Swift 的 ⌘ 在 Rust/GPUI 里对应 Ctrl，下文按此映射对比。

---

## 1. 快捷键对比（KeyboardShortcuts.swift vs keys.rs）

Swift 键位表：`Compositor/Compositor/UI/KeyboardShortcuts.swift:74-124`（ShortcutDefinition.all）。Rust：`crates/app/src/keys.rs:45-123`（bindings()）。

### 1.1 Swift 有而 Rust 缺失的键位（【缺失】）

| 功能 | Swift 键 | Swift 位置 | Rust 情况 |
| --- | --- | --- | --- |
| Save As | ⇧⌘S | KeyboardShortcuts.swift:77 | 无 Save As 动作，无键 |
| Export JPEG | ⌥⇧⌘S | KeyboardShortcuts.swift:78 | 完全缺失（无动作无 UI） |
| Close Project | ⌘W | KeyboardShortcuts.swift:79 | CloseDocument 动作存在（keys.rs:14）但**未绑定键**（keys.rs 中无 ctrl-w） |
| Show Transform Controls | ⌘H | KeyboardShortcuts.swift:81 | 无此动作 |
| Cut / Copy / Copy Merged / Paste | ⌘X/⌘C/⇧⌘C/⌘V | KeyboardShortcuts.swift:82-84 | 无任何剪贴板动作/键 |
| Fill with Foreground | ⌥⌫ | KeyboardShortcuts.swift:84 | FillForeground 动作在菜单（menu.rs:47）但**无键** |
| Fill with Background | ⌘⌫ | KeyboardShortcuts.swift:85 | 同上，无键 |
| Content-Aware Fill | ⇧⌫ | KeyboardShortcuts.swift:85 | 完全缺失 |
| Select Subject | ⌥⌘A | KeyboardShortcuts.swift:87 | 缺失 |
| Curves | ⌘M | KeyboardShortcuts.swift:88 | 缺失（core 有 curves 内核，无入口） |
| Invert Pixels / Mask | ⌘I | KeyboardShortcuts.swift:89 | 缺失（core session 无 invert_pixels） |
| Toggle Clipping Mask | ⌥⌘G | KeyboardShortcuts.swift:92 | 缺失（core 无 toggle_clipping_mask） |
| Move Layer Up / Down | ⌘] / ⌘[ | KeyboardShortcuts.swift:93 | 面板按钮有（layers_panel.rs:170-181）但无键 |
| Snap 总开关 | ⇧⌘; | KeyboardShortcuts.swift:96 | SnapGuides 等 4 个动作存在（keys.rs:32）但**均无键** |
| Lock Guides | ⌥⌘; | KeyboardShortcuts.swift:97 | 缺失（无 lock 概念） |
| Select tool (A) | A | KeyboardShortcuts.swift:99 | 缺失（Rust 套索只绑 L，keys.rs:50） |
| 画笔硬度 − / + | ⇧[ / ⇧] | KeyboardShortcuts.swift:110 | 缺失 |
| 混合模式 − / + | ⇧− / ⇧= | KeyboardShortcuts.swift:111 | core 有 cycle_blend_mode 但未绑定 |
| 循环形状种类 | ⇧U | KeyboardShortcuts.swift:112 | 缺失（Tab 只循环当前工具模式） |
| 不透明度数字 0–9（两位精确） | 0–9 | KeyboardShortcuts.swift:113 | 缺失（app 中无任何数字键处理，已 grep 验证） |
| 移动选中像素（⌘方向 / ⇧⌘方向） | ⌘←→↑↓ 等 8 条 | KeyboardShortcuts.swift:114-116 | 缺失（Rust 的 nudge 只移图层/选区，workspace.rs:514-526） |
| 完成文本编辑 | ⌘⏎ | KeyboardShortcuts.swift:118 | 缺失（Rust 文本层无编辑状态） |
| Tracking/Leading ± （⌥方向 及 ⌥⇧方向 ×10） | 8 条 | KeyboardShortcuts.swift:119-122 | 缺失 |
| 切换 Levels 预览 | ⌥P | KeyboardShortcuts.swift:124 | 缺失（Rust 无 preview 开关） |
| 临时手型（按住空格） | Space | KeyboardShortcuts.swift:104 | 缺失（keys.rs/canvas.rs 均无 space 处理） |
| Delete 删除选区/效果/套索点 | ⌫ | KeyboardShortcuts.swift:105 | Rust delete/backspace 只绑 DeleteLayer（keys.rs:88-89） |

### 1.2 【行为差异】

- **Export PNG 键位**：Swift ⇧⌘E（KeyboardShortcuts.swift:78、CompositorApp.swift:69）；Rust 是 `ctrl-shift-s`（keys.rs:72）——键不同。
- **Show Guides**：Swift ⌘;（CompositorApp.swift:112）；Rust `ctrl-shift-r`（keys.rs:100）。
- Zoom In Rust 额外绑了 `ctrl-+`（keys.rs:102），Swift 只有 ⌘=（可算【多出】，无害）。

### 1.3 整个子系统【缺失】：快捷键自定义

Swift 有完整可自定义系统：`ShortcutSettings`（KeyboardShortcuts.swift:129-213）——覆盖表存 UserDefaults（`keyboardShortcuts.v1`，:134）、冲突校验 `problem(in:)`（:165-180，含 macOS 保留键 ⌘Q/⌘,/⌃⌘M）、录制器 `ShortcutRecorder`（:283-319）、`canvasEvent/textEvent` 重映射（:184-212）。Rust 只有静态 `SHORTCUTS_TEXT` 纯文本说明（sheets_ui.rs:1028）。

---

## 2. 菜单对比（CompositorApp.swift vs menu.rs）

Rust 菜单为窗口内自绘菜单栏（menu.rs:13-117），含 File/Edit/Image/Layer/Select/Filter/View/Window/Help 九个菜单。缺失项如下（Swift 位置以 CompositorApp.swift 行号计）：

**File**（Swift :47-80 vs menu.rs:34-42）【缺失】：Import Images…（:58）；Save As…（:64）；Export JPEG…（:71）。

**应用菜单**（:83-85）【缺失】：Check for Updates…（Sparkle）。（跨平台可豁免）

**View 类命令**（Swift 在 :86-136）：
-【缺失】Fit Canvas / Actual Pixels / Zoom In / Zoom Out 菜单项（:87-98；Rust 仅工具键 + Zoom 工具头按钮 tools.rs:445-451）。
-【缺失】总 Snap 开关 `snappingEnabled`（:101-102，与 snapTo* 是两个不同状态）。
-【缺失】Show Transform Controls 开关（:103-105）。
-【缺失】Lock Guides（:131-132）、Clear Guides（:133-134；core 有 `clear_guides` 但无菜单调用）。

**Edit**（Swift :146-192 vs menu.rs:43-53）【缺失】：Cut、Copy、Copy Merged、Paste、Clear Selection Pixels（:188）、Content-Aware Fill…（:190）。Rust 把 Free Transform 放进了 Edit 菜单（menu.rs:50，Swift 在 Layer 菜单 :277）——重组，可接受。

**Select**（:193-229 vs menu.rs:77-85）【缺失】：Layer's Pixels（:211）、Subject（:215）、Mask's Black Areas（:218）。

**Image**（:230-260 vs menu.rs:54-63）【缺失】：Curves…（:231）、Levels…（:233）、Hue/Saturation…（:235）——注意这三个 **sheet 在 Rust 存在但菜单没入口**，只能靠快捷键；Black & White / Color Balance / Exposure / Gradient Map / Grain（:237-240）；Invert (Pixels/Mask)（:241）。【多出】Rust 的 Crop to Selection（menu.rs:59，workspace.rs:283-290，Swift 无此菜单项）。

**Filter**（:261-266 vs menu.rs:86-93）【缺失】：Remove Background（FilterKind 有此 case，core filters.rs:26）；Swift Filter 菜单遍历 `FilterKind` 全部非调整项（Document/Filters.swift:7-27），Rust 硬编码 5 项。curves/exposure/gradientMap/grain/blackWhite/colorBalance 六个调整在两侧都不在 Filter 菜单，但 Swift 放在 Image 菜单而 Rust 哪都没有。

**Layer**（:267-318 vs menu.rs:64-76）【缺失】：New Adjustment Layer 子菜单（:268-272）、Edit Adjustment…（:273）、Transform Layer/Selection（:277，键有菜单无）、Layer via Copy 动态标题（:279）、Create/Release Clipping Mask（:282）、Move Out of Folder（:290）、Rename Layer…（:294，Rust 靠双击重命名）、Show/Hide Layer（:296）、Move Layer Up/Down（:300-303）、动态 Merge 标题（:305）、动态 Delete Layer/Mask/Effect（:314）。

**Window/Help**【多出】：Rust 的 Reset Panel Layout（menu.rs:110）是**空实现**（workspace.rs:439-441 只显示 "Panel layout is fixed in this build"）；About Compositor 用 sheet 实现（sheets_ui.rs:608-620）。

菜单项禁用逻辑：Swift 每项有精细 `disabled(...)` 条件（如 :63, :164, :207-217）；Rust PopupMenu item 只有 `disabled(!entry.enabled)` 机制（ui.rs:85-88）且基本未用，菜单永远可点、靠 handler 内部兜底——【简化】。

---

## 3. Sheet / 面板 / 控件逐项对比

### 3.1 两侧都有但参数不同的（【行为差异】）

| Sheet | Swift | Rust | 差异 |
| --- | --- | --- | --- |
| New Canvas | 默认 1920×1080，合法 1–30000 px（NewCanvasSheet.swift:9-10, 28），**剪贴板尺寸建议**（:46-57, 59-74） | 默认 1920×1080（sheets_ui.rs:114-115），只 `max(1)` 无上限（:645-650） | 【缺失】剪贴板建议；【缺失】上限校验；【多出】Preset 下拉 + Name 输入（Swift 无） |
| Image Size | 单位 Pixels/Percent/Inches/Centimeters（ImageSizeSheet.swift:13）、Resample 开关 + Sampling 选择（:89-106）、分辨率 1–9600、1e8 像素上限（:24-27） | 只有 Percent 勾选 + 分辨率文本框（sheets_ui.rs:252-307, 658-676），无校验 | 【缺失】单位制、Resample/Sampling、全部范围校验 |
| Canvas Size | 扩展色 Transparent/Foreground/Background/Black/White/Custom+取色器（CanvasSizeSheet.swift:26-38, 97-102）、锁定比例（:60）、字节量提示（:45） | 仅宽/高/Relative/Anchor（sheets_ui.rs:308-337），`CanvasSizeOptions::new` 未设 fill（:693-695） | 【缺失】扩展色/自定义色、锁定比例 |
| Trim | TrimOptions 无 tolerance（TrimSheet.swift:51-57），全不选时禁用 OK（:62） | 有 Tolerance 输入默认 0、min(255)（sheets_ui.rs:604-606, 728），无 sides 全关校验 | 【多出】tolerance；【缺失】禁用逻辑 |
| Levels | 通道直方图 256 bins 按通道着色（LevelsSheet.swift:97-112）、渐变条上可拖三角手柄+γ 反解（:113-143）、数字输入框（:89-96）、取色管 Sample black/gray/white（:45-55；Document/LevelsAutomatic.swift:3）、Auto 三模式（Document/LevelsAutomatic.swift:4-5）、Preview 开关+Reset（:68-74） | 直方图 1024 bins 灰色、开 sheet 时算一次不随通道更新（sheets.rs:145-151, sheets_ui.rs:993-1026）、5 个普通滑杆（0–254/0.1–9.99/1–255/0–255/0–255，sheets.rs:139-143）、Auto 仅 1 个按钮（0.5%/99.5% 分位，sheets.rs:384-422） | 【缺失】手柄、数字框、取色管、三档 Auto、Preview、Reset；【行为差异】直方图粒度/着色 |
| Hue/Saturation | Range 选择器+色带编辑器 SpectrumEditor+反向应用+取色管+目标调整（HueSaturationSheet.swift:26-96, 130-193）；Colorize 时 hue 0–360/sat 0–100 并取 colorizeStart（hue 0, sat 25）（:10-11, 41-44；Document/HueSaturation.swift:205） | 固定三滑杆 −180–180/−100–100/−100–100 + Colorize 勾选（sheets.rs:157-161, sheets_ui.rs:390-413）；colorize 只做 `saturation.max(1.0)`（sheets.rs:285-294） | 【缺失】Range/色带/取色管；【行为差异】colorize 初值与量程 |
| Filters 通用 | 半径等滑杆为**对数**滑杆+精确数字框（FilterSheet.swift:141-154）、Preview 开关、selection 限定提示（:102-109） | 线性滑杆、无数字框、无 Preview 开关 | 【简化】 |
| Gaussian Blur | 默认 radius=1（Document/Filters.swift:39），范围 0.1–250 | 默认 **4.0**（sheets.rs:170） | 默认值不同 |
| Motion Blur | 默认 distance=10（Filters.swift:43） | 默认 **20**（sheets.rs:176） | 默认值不同 |
| Add Noise | 默认 amount=10（Filters.swift:45），Distribution 为 segmented | 默认 **12**（sheets.rs:182），Gaussian 用勾选框 | 默认值不同（范围/勾选语义一致） |
| Camera Raw | 10 个可折叠分区（Light/Color/Effects/Curve/Color Mixer/Color Grading/Detail/Optics/Geometry/Calibration，CameraRawControls.swift:370-382）+直方图/矢量示波器（:32-60）+阴影/高光剪裁指示（:67-81）+白平衡预设与吸管（:184-217）+每分区预览“眼睛”（:272-293）；CameraRawSlider 带彩色渐变轨道和双击复位（CameraRawSlider.swift:1-115, 186-198） | 单列 14 个滑杆：温度/色调/曝光(−5–5)/对比/高光/阴影/白色/黑色/鲜艳度/饱和度/纹理/清晰度/去雾/暗角（sheets.rs:189-213, sheets_ui.rs:468-487） | 【缺失】后 6 个分区全部控件（Glow 5 滑杆+样式、Vignette 的样式/中点/圆度/羽化/高光、Grain 3 项、Curve/Mixer/Grading/Detail/Optics/Geometry/Calibration）、直方图/示波器、剪裁指示、白平衡吸管、双击复位、彩色轨道；量程本身一致（exposureRange −5…5、toneRange −100…100，Document/CameraRaw.swift:48-49 vs sheets.rs:190-205） |
| Selection Amount (Feather/Expand/Contract) | 滑杆+数字框，feather ≤250 其余 ≤500（LassoControls.swift:199-204, 210-212） | 只有数字框（sheets_ui.rs:497-503），无范围校验 | 【简化】 |
| Export | Export PNG 经 NSSavePanel；Export JPEG 有质量滑杆(0–1 步 0.01 记忆上次)+背景色+实时预览+字节数（JPEGExportSheet.swift:8-66） | 路径文本框 sheet（sheets_ui.rs:504-517, 709-712, 783-807），只有 PNG | 【缺失】JPEG 导出、质量记忆；【简化】无原生对话框 |
| Open/Save | NSOpenPanel / `.comp` 包 + PSD 打开（PSDConversionSheet.swift:20-56）+ RAW（RawDevelopSheet.swift:20-89） | 路径文本框（sheets_ui.rs:717-720, 829-897） | 【缺失】PSDConversionSheet、RawDevelopSheet、原生对话框 |

### 3.2 Swift 有、Rust 完全没有的面板（【缺失】）

- **ColorPickerSheet**（ColorPickerSheet.swift:7-157 + FloatingPanel 载体 :165-185）：饱和度/亮度场、色相条、RGB/Hex 输入、画布取色。
- **EffectsSheet**（EffectsSheet.swift:4-192）：描边/投影/颜色叠加/内阴影/外发光/内发光 6 种图层效果编辑（core effects.rs 有内核但无 UI、无入口菜单）。
- **CurvesControls**（CurvesControls.swift:3-69）：可加/拖/删点的曲线编辑器。
- **PSDConversionSheet / RawDevelopSheet**（见上）。
- **CameraRaw 三个子面板文件**（CameraRawColorControls.swift 485 行、CameraRawDetailOpticsControls.swift 183 行、CameraRawGeometryCalibrationControls.swift 140 行）。
- **浮动面板系统** FloatingPanelController（FloatingPanel.swift:16-174）：非模态可移动面板、位置记忆、Camera Raw 停靠主窗右侧（:125-136）。Rust 全部为模态居中 overlay（sheets_ui.rs:123-145）。
- **图层列表右键菜单/效果行/蒙版缩略图**（NativeLayerList.swift，1101 行，详见 §5）。

### 3.3 工具选项头逐工具对比（ContentView.swift:25-76 vs tools.rs:132-559）

- Brush/Healing/Clone/Blur：Rust 有 Size(1–300)/Hardness/Opacity/Smoothing 滑杆 + Paint/Erase、Heal 模式、Blur 模式、Clone Aligned/All Layers（tools.rs:157-291）。【行为差异】画笔直径上限 300（state.rs:159, 267）vs Swift 2000（BrushControls.swift:42,48；Document/BrushStroke.swift:175）。【缺失】蒙版选中时的 Black/White 绘制切换（BrushControls.swift:82-86）、前景色色板按钮（:87-104）、Mask 提示。
- Marquee/Lasso/Wand：Rust 有 kind/mode 下拉 + 容差滑杆 + Contiguous + All Layers（tools.rs:292-393）。【缺失】选区模式（加/减/交）显示与点击设置（LassoControls.swift:40-45）、Anti-alias 开关（:49-52）、头部内嵌 Expand/Contract/Feather 快捷控件（:53-72）、Object 模式的 Edge 参数（:113-137）、Wand Sample Size（:97-101）。
- Move：【缺失】TransformInspector 全部字段（X/Y/W/H/Scale%/角度/Sampling/Flip H/V/锁定比例/Cancel/Apply，TransformInspector.swift:9-50）。Rust 只有一个**不做事的** Auto-select 勾选框（tools.rs:500-507，handler 为空）。
- Eyedropper：【缺失】Sample Ring 开关（ContentView.swift:51-58）。
- Hand/Zoom：【缺失】缩放百分比输入框 0.1–3200%（NavigationToolHeader.swift:13-29）；Rust Zoom 头只有 4 个按钮（tools.rs:445-451）。
- Type：【缺失】字体选择器/字号/颜色/对齐/Tracking/Leading/Edit Text（TypeControls.swift:16-81）；Rust 只有一行提示文字（tools.rs:512-514），文本层用内置 5×7 点阵字体（text_bitmap.rs:11-88）。
- Shape：【缺失】线宽滑杆（1–100 显示，上限 5000）与圆角滑杆（ShapeControls.swift:17-47）；Rust 只有形状下拉（tools.rs:515-556）。
- Gradient：Rust 有形状下拉/Fg-Bg/反向/不透明度（tools.rs:394-444）。【缺失】渐变色板预览（GradientControls.swift:42-60）、样式下拉（fg/bg 快速选择改为勾选框，语义近似）。
- Crop：比例下拉五项与 Swift 一致（Free/Original/1:1/4:3/16:9，tools.rs:452-496 vs CropControls.swift:11-14）；【缺失】尺寸实时显示（CropControls.swift:15-17）。
- 工具栏（rail）：16 工具与字母一致（tools.rs:21-38 vs ContentView.swift:276）；【缺失】图标随模式变化（多选套索→多边形图标、魔棒→对象图标、椭圆虚线圈，ContentView.swift:281-284）、A("Select tool") 空档工具 Idle 的头部（ContentView.swift:68-74）、调色板"重置 D"小按钮与蒙版弹层（ColorPaletteControls.swift:26-37, 40-48）；Rust 色板点击=交换（tools.rs:112-125）而非打开取色器。【行为差异】

### 3.4 图层面板（LayersPanel.swift + NativeLayerList.swift vs layers_panel.rs）

Rust 有：标题、Blend 下拉（24 模式分组带分隔，layers_panel.rs:188-211，与 BlendModePicker.swift:11-14 分组一致）、Opacity 滑杆+数值、行（眼睛/30px 缩略图/名称双击重命名/M 徽标/G 徽标）、底部 7 个按钮（New/Mask/Group/Dup/Del/Up/Down）。

-【行为差异】**缩略图渲染**：Swift 是“整画布形状框住图层像素”+棋盘格底+按 transform 摆放（CanvasThumbnail.swift:10-43），蒙版缩略图用边缘灰度填充（:37-62）；Rust 是内容裁剪式缩略图（core render.rs:459-472 `layer_thumbnail`），无棋盘格、无画布框、无蒙版缩略图（只有 "M" 徽标 layers_panel.rs:323-334）。
-【行为差异】面板宽度：Swift 可拖拽 202–352 且跨启动记忆（LayersPanel.swift:7、ContentView.swift:6, 104、PanelResizeEdge :337-356）；Rust 固定 248（layers_panel.rs:42）。
-【行为差异】混合模式悬停预览：Swift 菜单划过即预览（BlendModePicker.swift:44-61）；Rust 无。
-【缺失】多选（⌘/⇧点击）、拖拽排序/入组/跨项目拖拽、Option 拖拽复制（NativeLayerList.swift:150-256）、组折叠箭头与层级缩进（:467, 792）、蒙版缩略图 + 链接开关（:469, 764）、效果行（眼睛+拖拽，:849-941）、右键菜单全套（:604-616, 769-806）、⌘点缩略图载入选区（:773）、裁剪蒙版光标区（:310-343）、行内 keyDown（:422）、空状态插画（LayersPanel.swift:23-30）、图层计数角标（:14）。
-【缺失】底部 Layer Effects 菜单和 Adjustment Layer 菜单（LayersPanel.swift:41-54）。
- 不透明度百分比输入框（LayerAppearanceControls.swift:20-31）在 Rust 只有文本标签（layers_panel.rs:100）【简化】。

---

## 4. SwiftUI 内嵌业务逻辑在 Rust 的对应位置

| Swift 逻辑 | 位置 | Rust 对应 | 结论 |
| --- | --- | --- | --- |
| CanvasThumbnail 渲染（棋盘格/摆放/边缘灰度） | CanvasThumbnail.swift:10-78 | core render.rs:459 `layer_thumbnail`（内容裁剪）；无 fittedSize/checkerboard/edgeTone | 【简化/缺失】 |
| SliderSnap 轨道点击吸附（method swizzle） | SliderSnap.swift:10-42 | 无（gpui slider 自带行为） | 【缺失】 |
| CanvasRulers 刻度计算 majorStep/label | CanvasRulers.swift:174-184 | canvas.rs:1205-1214 `nice_tick`（候选集 12 个 vs Swift 18 个，目标 50px vs 70px；无 label、无 minor/mid 分级刻度） | 【简化】 |
| 标尺拖拽创建参考线 | CanvasRulers.swift:132-152 | core session 有 `add_guide/clear_guides`，但 app 无任何调用（grep 验证）；参考线只读绘制（canvas.rs:108-116, 910-913） | 【缺失】 |
| TransformInspector 字段换算（scalePercent/resize 锁比/rotation 取模） | TransformInspector.swift:26-32, 63-74 | UI 无字段；core transform.rs 有变换内核（含 scaled 等，见 README 对照表） | UI 侧【缺失】 |
| ColorPickerSheet HSB 换算 + Hex | ColorPickerSheet.swift:49-69, 104-145 | core document.rs:87-115 `PickerHSB`、:57/:67 `hex/from_hex`（含测试 document.rs:787-816）——内核已移植，UI 无 | 内核✓ UI【缺失】 |
| ArrowStepper（字段 Up/Down 步进 ×10） | ContentView.swift:375-447 | 无 | 【缺失】 |
| NewCanvasSheet 剪贴板尺寸 | NewCanvasSheet.swift:59-74 | 无 | 【缺失】 |
| PanelResizeEdge 拖拽调宽 | ContentView.swift:337-356 | 无 | 【缺失】 |
| 状态栏每工具操作提示长文案 | ContentView.swift:327 | 无（状态栏显示文档尺寸/缩放/工具名/消息/撤销名，workspace.rs:528-594） | 【简化】 |
| ProjectTab 标签宽度计算/滚动露出/拖拽槽 | ProjectTabs.swift:59-198, 200-216 | 无 | 【缺失】 |
| 相机原始滤镜内核（CameraRawSettings 全量字段） | Document/CameraRaw*.swift | core adjust/camera_raw.rs（README 称 full pipeline，含 geometry/calibration/mixer/grading） | 内核✓ UI 只暴露 14 参数 |

## 5. 多窗口 / 多项目

- Swift：`ProjectWorkspace`/`ProjectTab`/`ProjectTabStrip`（ProjectTabs.swift:6-304）多标签多项目、未保存圆点、标签拖放（文件/图层拖到标签或 New 槽，:282-304）、`ProjectWindowBridge`（ProjectWindowBridge.swift:5-62）关闭拦截 windowShouldClose、`handlesExternalEvents`（CompositorApp.swift:15）Finder/Dock 打开、首启动满屏布局（:18-20）。
- Rust：`main.rs:30-49` 固定开**一个**窗口（1560×920），无标签条、无多项目、无拖放导入（grep 无 on_drop）、无关闭确认——`CloseDocument` 直接清空 document（workspace.rs:236-244）；无未保存标记 UI（core 有 `is_modified`/`mark_saved` 未接 UI）。文件打开用文本路径 sheet。【整体缺失】

## 6. 持久化差异

Swift 有 `@AppStorage layersPanelWidth`（ContentView.swift:6）、`jpegExportQuality`（JPEGExportSheet.swift:8）、`keyboardShortcuts.v1`（KeyboardShortcuts.swift:134）、FloatingPanel 位置表（FloatingPanel.swift:18）。Rust 无任何 UI 状态持久化。【缺失】

## 7. 测试缺口清单

Rust app crate **0 个 #[test]**（grep 验证）；core 有内核测试。Swift UI 相关测试对照：

| Swift 测试文件 | 用例（行号） | Rust 对应 |
| --- | --- | --- |
| CanvasThumbnailTests | thumbnailsTakeTheCanvasShape(:24)、layerPixelsSitWhereTheyAreOnTheCanvas(:30)、masksFillTheCanvasWithTheirEdgeTone(:46) | 【缺失】（render.rs 无 thumbnail 测试） |
| CanvasEntryTests | eyedropperShortcutSelectsTool(:7)、commandZoomUpdatesOnKeyDownAndRepeat(:19)、clipboardSuggestsImagePixelsAndIgnoresText(:43)、mountingCanvasGivesItKeyboardFocus(:56) | 【缺失】 |
| FloatingPanelTests | hueSaturationPanelSurvivesALayoutPass(:25)、colorPickerPanelSurvivesALayoutPass(:42)、adjustmentEditorsUseMovableNonmodalPanels(:54-55)、dockedPlacementLeavesTheSavedFilterPosition(:82)、theDockedPanelFitsTheGradingWheels(:110)、cameraRawDocksToTheDocumentWindow(:126) | 【缺失】（无浮动面板概念） |
| SliderSnapTests | clickingTheTrackSnapsTheKnobAndStillEditsTheValue(:37) | 【缺失】 |
| ColorPickerTests | hexParsesFullShorthandAndRejectsInvalid(:7)、hsbRoundTripsEightBitColors(:16)、graysAndBlackKeepPreviousHueAndSaturation(:23) | ✓ core document.rs:787 `palette_hex_roundtrip`、:798 `picker_hsb_roundtrip`、:808 `picker_hsb_keeps_hue_through_gray` |
| ColorPickerTests | canvasSamplingReadsCompositeAndCommitsOnlyOnOK(:33)、pickerReopensWhereItWasLastLeft(:63) | 【缺失】（core render.rs:440 附近有取样函数但无测试） |
| CursorTests | leavingTheCanvasRestoresTheArrowWithEveryTool(:15)、aDragReleasedOutsideTheCanvasRestoresTheArrow(:33)、theLayerListShowsTheArrowUnlessAModifierCursorApplies(:51)、duplicateAndDistortCursors(:86)、hiddenTransformControlsLeaveOnlyMoving(:135)、optionOverALayerRowOffersDuplicatingIncludingOverThumbnails(:172) | 【缺失】（Rust 无自定义光标系统） |
| CameraRawSliderTests | colorTracksRunFromTheCoolOrMutedEndToTheWarmOrStrongEnd(:7)、temperatureBarIsBlueOnTheLeftAndADoubleClickHitsOnlyTheKnob(:24)、trackClickValueMatchesTheClickedPosition(:52) | 【缺失】（无渐变轨道滑杆） |

## 8. Rust 侧【多出】项汇总（便于甄别）

1. New Canvas sheet 的 Preset 下拉与 Name 字段（sheets_ui.rs:166-211）；Trim sheet 的 Tolerance（:604）；Image 菜单 Crop to Selection（menu.rs:59）；Window>Reset Panel Layout（空实现，workspace.rs:439）；About sheet（sheets_ui.rs:608）；`ctrl-+` 双绑定 ZoomIn（keys.rs:102）；启动即创建 1280×800 文档而非 Swift 的欢迎页（state.rs:154-155 vs ContentView.swift:306-310）。

---

### 关键文件索引
- Swift：`Compositor/Compositor/CompositorApp.swift`、`Compositor/Compositor/ContentView.swift`、`Compositor/Compositor/UI/*.swift`（41 文件）、`Compositor/Compositor/Document/{Filters,CameraRaw,Levels,HueSaturation,LevelsAutomatic,BrushStroke}.swift`
- Rust：`crates/app/src/{main,workspace,menu,keys,tools,canvas,state,sheets,sheets_ui,layers_panel,ui,text_bitmap}.rs`、`crates/core/src/{filters,render,document,session}.rs`
- 测试：`Compositor/CompositorTests/*.swift`（59 文件）；Rust app crate 无测试，core 各模块 #[cfg(test)]
