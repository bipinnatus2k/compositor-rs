# 子代理原始对比报告（2026-09-24）

本目录保存全工程 Swift↔Rust 对比 review 时 8 个并行子代理输出的**逐字原始报告**。
汇总、去重与优先级整理版见 [`../rust-vs-swift-diff-report.md`](../rust-vs-swift-diff-report.md)；
行号以原始报告为准（关键结论已抽查复核）。

| 域 | 文件 | 覆盖范围 |
| --- | --- | --- |
| A | [A-document-session-history.md](A-document-session-history.md) | 文档模型 / EditorSession / 历史 / 图层操作（DocumentHistory、LayerGroups、LayerMerge、LayerMask、LiveLayerMask、LayerAppearance、LayerAdjustment、ProjectWorkspace、ColorPalette、FloatingSelection、Guides、ToolDefaults ↔ document.rs、session.rs、history.rs、adjustment.rs） |
| B | [B-transform-canvas-project.md](B-transform-canvas-project.md) | 图层变换 / 几何 / 画布尺寸 / 项目文件格式（LayerTransform、CanvasSize、ImageTrim、Crop、LayerFlip、Guides、Distort、CanvasResizer、ImageResizer、ProjectStore ↔ transform.rs、geometry.rs、canvas.rs、project/） |
| C | [C-adjustments.md](C-adjustments.md) | 图像调整（CameraRaw*、Curves、Levels(+Automatic)、HueSaturation、ImageAdjustments、PixelAdjust、PixelInvert、GuidedMatte + AdjustPixels.c / LevelsPixels.c ↔ adjust/、adjustment.rs） |
| D | [D-selection.md](D-selection.md) | 选区系统（Selection、FloatingSelection、SelectionEdits、SelectionClipboard、MagicWand、MaskTracing、ObjectSelection、WandPixels.c ↔ selection.rs、tools/wand.rs） |
| E | [E-tools.md](E-tools.md) | 绘画/修复/变形工具（BrushStroke、BlurTool、CloneStamp、SmudgeLiquify、ContentFill、SubjectRemoval、ShapeTool、TypeTool、Gradient + BrushPixels.c / HealPixels.c / ContentFill.c / NoisePixels.c ↔ tools/、render.rs 形状部分） |
| F | [F-render-effects-filters.md](F-render-effects-filters.md) | 渲染合成 / 图层效果 / 滤镜 / 混合模式 / 光栅基础（LayerRenderer、TiledLayerRenderer、RasterSnapshot、DownsampleCache、EffectsPreviewCache、MetalLayerEffects、SeparableBlend、LiveMaskRenderer、LayerEffects、Filters ↔ blend.rs、effects.rs、filters.rs、render.rs、raster.rs） |
| G | [G-io-psd.md](G-io-psd.md) | 文件 IO（导入/导出/RAW/拖放）与 PSD 格式（ImageImporter/Exporter、RawImporter、PSDReader/Types/ChannelCoder/DocumentBuilder/Vector ↔ psd/、app 导入导出路径） |
| H | [H-ui-app.md](H-ui-app.md) | UI / 应用壳（CompositorApp、ContentView、UI/ 全部 41 文件 ↔ crates/app 全部 12 文件） |
