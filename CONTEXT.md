# compositor-rs

Compositor（macOS 图像编辑器）向 Rust 的 1:1 复刻。本表裁定项目领域语言：
Rust 侧命名跟随原版 Swift，保持可对照；同义异名一律以本表为准。

## Language

### 文档与会话

**CanvasDocument**:
一张打开的画布文档：宽高、分辨率、图层树、参考线、调色板。对应原版 `CanvasDocument`。

**ImageLayer**:
位图图层：导入资产 + `LayerTransform` + 外观/调整/蒙版。对应原版 `ImageLayer`。

**LayerTransform**:
图层的仿射放置（平移/缩放/旋转/斜切）。对应原版 `LayerTransform`。

**EditorSession**:
文档之上的操作状态机：当前工具、活动图层、选区状态、撤销守卫。对应原版 `EditorSession`。
_Avoid_: Session（单独使用时）

**DocumentHistory**:
快照式撤销栈，按条数与字节限量、支持可嵌套事务。对应原版 `DocumentHistory`。
_Avoid_: undo stack（代码命名中）

**ProjectWorkspace**:
多项目签页的工作区（标签条 + 浮动面板的宿主）。对应原版 `ProjectWorkspace` / `ProjectController`。

### 几何与坐标

**Document space**:
画布坐标系，图层 `LayerTransform` 所在的空间；像素网格经 `pixel_to_document`
仿射映射到它。

**Canvas Size vs Image Size**:
两个不同操作：Canvas Size 改画布边界（裁切/扩展画布），Image Size 缩放
图像内容。对应原版 `CanvasResizer` / `ImageResizer`，UI 命名跟随原版。

**Guides**:
用户放置的对齐参考线；与自动显示的 grid 区分。

### 工具与变换

**Auto-select**:
Move 工具按下时按指针位置拾取所在图层为活动层；默认关闭，按住 Cmd 临时反转。
对应原版 `transformAutoSelect`。

**Hover regions**:
变换框边缘、角点、旋转手柄的悬停命中区域，决定光标形态与按下后的拖拽语义。
对应原版 TransformTests 的 hoverRegions。

### 选区

**Selection**:
多边形选区引擎：geo 布尔运算 + 亚像素精确面积覆盖 + 羽化（σ/2 高斯）。
对应原版 `Selection`。

**FloatingSelection**:
从文档浮出的选区内容，可独立变换后再提交/粘贴。对应原版 `FloatingSelection`。
与 `Selection` 是两个概念：前者是"内容"，后者是"区域"。

**SelectionEdits**:
对选区内像素的编辑：填充/清除/反相、由选区建蒙版、由选区建调整层。对应原版 `SelectionEdits`。

### 外观与调整

**LayerAppearance**:
图层的外观属性组（混合模式、不透明度、图层效果）。对应原版 `LayerAppearance`。

**LayerAdjustment / adjustment layer**:
挂在图层上、可随时改参数的调整（预览/提交/取消，绑定其所属图层）。对应原版
`LayerAdjustment` 与 `AdjustmentEditing`。_Avoid_: filter（调整不是滤镜）。

**Filters vs Effects**:
Filters 是像素滤镜（模糊/噪声/镜头校正等一次性运算）；Effects 是图层效果
（stroke/shadow/overlay/inner shadow/glow）。两个词不互换。

**Blend modes**:
24 种 PDF/Photoshop 公式混合模式，含 CoreImage 补齐的 10 种。命名用原版表。

### 渲染与光栅

**Raster / bitmap**:
premultiplied RGBA8 位图；灰度蒙版单独称 gray mask。对应原版 CGContext 位图。

**Dab**:
笔刷笔画离散出的最小着色单位，由向心 Catmull-Rom 插值与间距控制生成。

**DownsampleCache**:
图层降采样缓存（缩放视图时的性能层）。对应原版 `DownsampleCache`。

### 文件

**`.comp`**:
本项目工程文件包（manifest.json + PNG 资产），manifest v1–9 语义。
对应原版 `ProjectStore`。

**PSD**:
Photoshop 文档导入/导出（8-bit RGB 子集）。对应原版 `IO/PSD/*`。
