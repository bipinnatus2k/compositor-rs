> 子代理原始报告（域 G：文件 IO / PSD），逐字保留，2026-09-24。

# 文件 IO（导入/导出）与 PSD 格式：Swift ↔ Rust 深度对比报告

## 总体结论

**库级 PSD 代码对等度非常高**：`crates/core/src/psd/` 对 `Compositor/Compositor/IO/PSD/` 五个文件的解析段、压缩、色彩模式限制、blend 签名映射、图层/分组/蒙版/调整层逻辑、vector mask 解析、转换报告文案逐行对齐，仅 4 处小差异（图层名编码、CG 光栅化替换、live-shape 模型、蒙版失败分支）。**真正的重大缺口全部在集成层**：Rust app（`crates/app`）**完全没有调用 `PsdReader`**，用户实际无法导入 PSD（`image::open` 会失败）；此外 HEIC/TIFF、EXIF 方向、色彩管理、导入尺寸预算、JPEG 导出、RAW 导入、文件拖放、Open With 整体缺失。测试方面 Swift 的 ExportTests(3)/JPEGExportTests(2)/ImageImportTests(8) 在 Rust 侧零对应，PSDRoundTripTests 21 例移植了 19 例。

以下路径均为仓库根相对路径，行号已用 Read/grep -n 核实。

---

## 一、PSD Reader（`crates/core/src/psd/reader.rs` vs `Compositor/Compositor/IO/PSD/PSDReader.swift`）

### 已验证一致的部分
- **文件头**：`8BPS` 签名（失败报 unreadable）、version==1 否则 unsupportedVersion、跳 6 字节、通道数丢弃、画布 1...30000 且 ≤100MP（PSDReader.swift:29-45 ↔ reader.rs:38-64）。
- **色彩模式/位深**：双方都只接受 `depth==8 && mode==3`（8-bit RGB）。灰度/索引/CMYK、1/16/32 位均拒绝——这是双方共同的限制，非差距（PSDReader.swift:44-45 ↔ reader.rs:59-64）。
- **Color Mode Data 段**：读长度后跳过（PSDReader.swift:46 ↔ reader.rs:66-67）。
- **Image Resources**：仅解析 `8BIM` 资源，非 8BIM 即 break；仅 id 1005 ResolutionInfo 按 16.16 定点解析，非有限/`<1` 回退 72，clamp 1...9600（PSDReader.swift:50-66 ↔ reader.rs:71-97）。ICC 等其它资源双方都忽略。
- **Layer & Mask Information**：`layerSection < 4` 返回空文档、`layerInfoLength` 丢弃、负 count 取 abs（首层标识）、count ≤ 10000（PSDReader.swift:68-77 ↔ reader.rs:100-115）。
- **Layer Records**：通道数 ≤56、通道 (id,length) 表、`8BIM` 签名（否则 truncated）、blend key、opacity、clipping、flags bit1=hidden、extra 段内 mask(rect/default/flags: bit1=disabled, bit0反转=linked, bit3=fromRender)/blending ranges/Pascal 名/`8BIM`+`8B64` 附加段循环，`luni` 覆盖名、`iOpa` 覆盖 fill、`lsct`/`lsdk` 分组类型（PSDReader.swift:113-181 ↔ reader.rs:138-228）。
- **Channel Data**：只解码 `{-1,0,1,2,-2}`（PSDReader.swift:197 ↔ reader.rs:231），跳过的通道仍按 length 推进游标（Swift defer ↔ reader.rs:263 注释），蒙版/图层双预算校验（30000/边、剩余像素），RGB 缺平面补黑、A 缺省补 255，长度不足报 truncated（PSDReader.swift:199-241 ↔ reader.rs:233-290）。
- **assemble**：lsct type 3=开组/1·2=收组、groups 栈分配父子、组 blendKey `pass`/`norm`→`pass`、组 bounds=画布、fill-opacity 乘积（有 lfx2/lrFX/lmfx 且 fill≠255 时只用 opacity）、vector live→raster 两级回退并扣减像素预算、maskFromRender 丢弃蒙版、adjustment 覆盖 kind、未闭合组报 truncated（PSDReader.swift:243-295 ↔ reader.rs:292-379）。
- **kind 判定**：text(TySh/tySh/txt2)、vector(vmsk/vsms/vogk)、smartObject(SoLd/SoLE)、effects(lfx2/lrFX/lmfx)、adjustment（16 个 key 完全一致，PSDReader.swift:297-310 ↔ reader.rs:381-406）。
- **PSDAdjustments**：levl(292 字节、4 通道×10 字节、gamma/256)、curv(version 1/4、≤4 通道、钳 0-255、排序补端点)、hue2/hue␣(colorize@2、7 个 range×6 字节)（PSDReader.swift:365-448 ↔ reader.rs:490-629）。
- **Blend 签名映射**：24 项四字符码逐一一致（含 `mul `/`div `/`hue `/`lum `/`sat ` 带空格）；dissolve/darker/lighter 有意缺失（PSDTypes.swift:62-92 ↔ types.rs:187-215）。
- **错误枚举**：PsdError 覆盖 PSDError 5 例 + ImageImportError 2 例，文案逐字一致（PSDTypes.swift:5-16 ↔ types.rs:13-46）。

### 差异
1. **【行为差异】图层名 Pascal 串解码**：Swift 用 `macOSRoman` 解码、Latin-1 兜底（PSDReader.swift:154）；Rust 用 `String::from_utf8_lossy`（reader.rs:186，注释 184-185 自认近似）。非 ASCII 单字节名（如 Latin-1 `0xE9`=é）在 Swift 正常、Rust 变 U+FFFD。`luni` UTF-16 路径两边等价（PSDReader.swift:183-194 ↔ reader.rs:469-484）。
2. **【缺失】`PSDReader.matches(_ url:)`**：Swift 有从文件读 4 字节魔数的 URL 版本（PSDReader.swift:10-22），Rust 只有 `matches(data)`（reader.rs:28-30）。
3. **【缺失】`read(from url:)` 文件入口**：Swift 支持 `.mappedIfSafe` 文件加载（PSDReader.swift:24-27），Rust 无文件加载版本（bytes-only）。
4. **【行为差异，等价】layerSectionEnd 跳转**：Swift 读完后 `cursor.offset = layerSectionEnd`（PSDReader.swift:86）；Rust 直接丢弃 cursor 并留注释（reader.rs:127-129）。因后续无读取，行为等价。
5. **【缺失，最严重】PSD 导入未接入 app**：`crates/app` 全目录 grep 无任何 `psd`/`8BPS` 引用。`open_document`（crates/app/src/sheets_ui.rs:829-897）对 `.psd` 走 `image::open`（workspace `Cargo.toml:13` 只启用 `features=["png","jpeg"]`）→ 必然失败，只显示 "Open failed"。Swift 的分发逻辑在 `EditorSession.importImages`（Compositor/Compositor/Document/EditorSession.swift:741,764-776：`PSDReader.matches` → 确认对话框 → `makeImport`）。README.md:41/63 声称 "PSD reader/writer" 与 "Photoshop PSD files (8-bit RGB) are imported"，后者目前与 app 实际行为不符。

---

## 二、PSD 写出（fixture 即 writer）

Swift app **从不写 PSD**（PSDFixture.swift:5 明言 "Compositor does not write PSD"）；唯一的字节级写出器是测试 fixture。Rust `crates/core/src/psd/fixture.rs` 是其移植，README 的 "reader/writer" 措辞对应于此。

- **一致**：文件头（channels=4、depth 8、mode 3）；RLE 编码 `packBits` 字节级一致（PSDFixture.swift:308-330 ↔ channel_coder.rs:129-155，含 128-run 上限、-128 不发射）；`encode_plane` 每行 2 字节大端计数（PSDFixture.swift:247-264 ↔ channel_coder.rs:159-172）；记录写出（坐标 clamp i32、blend key 补空格截 4、opacity×255 四舍五入、clipping/visible flags，PSDFixture.swift:135-156 ↔ fixture.rs:228-251）；extra（20 字节蒙版块+flags、blending ranges=0、Pascal 名 ≤255+4 对齐、luni、lsct 3/1 与 divider 名 `</Layer group>`，PSDFixture.swift:158-211 ↔ fixture.rs:253-314）；分辨率资源 1005 fixed-point clamp 1...9600（PSDFixture.swift:213-228 ↔ fixture.rs:316-331）；layer info 长度回填与整体偶数填充（PSDFixture.swift:38-76 ↔ fixture.rs:97-126）。
- **【简化】全局 composite**：Swift `appendComposite` 经 `BrushRaster.context` 把图像重绘到 width×height 再拆平面（PSDFixture.swift:230-245）；Rust 尺寸不符时用 `resample` 近似（fixture.rs:333-340，有注释说明）。
- **【简化】平面拆分/灰度平面**：Swift 经 CG context 重绘后拆 RGBA、蒙版经灰度 context（PSDFixture.swift:267-306）；Rust 直接从 premultiplied 位内存拆并手工反预乘（fixture.rs:356-381）、蒙版直接取 `GrayMask.data`（fixture.rs:144-146）。

---

## 三、PSDVector（`crates/core/src/psd/vector.rs` vs `PSDVector.swift`）

- **一致**：`live()` 的 fillEnabled/strokeEnabled 描述符解析（vstk `bool` 字段 → SoCo 存在性兜底）、SoCo RGB（`doub` 字段、>1 按 0-255 归一）、vogk origination（keyOriginType 1/2=矩形 2=圆角、5=椭圆、keyOriginShapeBBox 四个 UntF、四半径一致才认圆角、hi-lo>0.5 拒绝）、sharpRect 四锐角推断（26 字节记录循环、type 0/3 长度记录、锚点距离 0.5）、`pixelSize`（非有限→None、>30000/超预算→TooLarge、maxShapePixels=1e8，ShapeTool.swift:68 ↔ vector.rs:19）、`path()`（26 字节记录、type 0/3/1/2/4/5、固定点 /0x1000000×画布、y 在前 x 在后、闭合语义）、bbox 含控制点（CGPath.boundingBoxOfPath 语义，vector.rs:432-466）、stroke>30000 报 TooLarge（PSDVector.swift:20-170,172-281 ↔ vector.rs:49-352,375-500）。
- **【简化】光栅化引擎替换**：Swift 走 Core Graphics（`setShouldAntialias(true)`、winding fill、miter join/limit 10/butt cap，PSDVector.swift:62-79）；Rust 自写 scanline 光栅器（8 样本垂直 box AA 的非零环绕填充 vector.rs:726-826，描边折线 outline vector.rs:620-722；模块注释 vector.rs:6-8 明示替换）。抗锯齿与边缘不逐像素一致。
- **【行为差异】闭合路径收口角**：CG 对 `closeSubpath` 的路径在收口顶点计算 miter join；Rust 把闭合子路径转为重复首点的折线并按开放折线描边（vector.rs:160-169 注释），收口顶点只有两个 butt 端对接、无 joint，单角几何与 Swift 不同。
- 其余（`rgb`/`bool`/`unit`/`int32`/`double`/`offset`/`ascii` 字节探测）逐一对齐（PSDVector.swift:211-281 ↔ vector.rs:299-356 + types.rs:289-307）。

---

## 四、PSDDocumentBuilder（`crates/core/src/psd/document_builder.rs` vs `PSDDocumentBuilder.swift`）

- **一致**：6 类 kind 的转换提示文案逐字一致；组 pass-through 提示；非组不支持 blend 提示（key trim 后内插）；adjustment 两分支提示；不可用 adjustment 跳层；组/调整/位图/无图像四分支建层；bounds.size 为 0 时回退图像尺寸；clipping `baseForParent` 算法及失败提示；蒙版 enabled/linked（PSDDocumentBuilder.swift:23-122 ↔ document_builder.rs:46-241）。
- **【缺失（不可达分支）】蒙版转换失败报告**：Swift `try? LayerMask.asset(from:)` 失败时追加 "The layer mask couldn't be converted to 8-bit grayscale and was skipped."（PSDDocumentBuilder.swift:95-99；失败源是 LayerMask.swift:23-26/35-36 的单色校验）；Rust `layer_mask_asset` 无失败路径，该消息未移植（document_builder.rs:190-199 有注释说明）。实际不可达，属可接受简化。
- **【行为差异】live shape 模型**：Swift `LayerShape = style + CGImage`（像素身份，ShapeTool.swift:37-40、`liveShape` 49-53），导入测试断言 `liveShape?.image === asset?.image`（PSDRoundTripTests.swift:322）；Rust `document::LayerShape` 是纯几何模型，`LayerShapeStyle::to_layer_shape()` 合成 `line_width: 4.0, filled: true`（types.rs:246-258），像素靠图层自身 asset（mod.rs:583-589 的测试改为断言 asset 存在）。
- 缩略图 96px 长边上限一致（PixelAdjust.thumbnail ↔ raster.rs:242-247；蒙版缩略图 LayerMask.swift:37 ↔ document_builder.rs:251-262）。

---

## 五、ImageImporter / ImageExporter / RawImporter

### ImageImporter（Swift `ImageImporter.decode` ↔ Rust app `open_document`）
Rust 没有对应 `ImageImporter`，只有 sheets_ui.rs:855-866 的 `image::open` + 手工预乘。
1. **【缺失】格式白名单**：Swift 仅放行 jpeg/png/heic/tiff，其余报 `.unsupported`（ImageImporter.swift:37-39）；Rust 无白名单，且 image crate 只启用 png+jpeg——HEIC/TIFF 反而不支持，GIF 等也因 feature 未开而失败，错误文案不区分。
2. **【缺失】EXIF 方向**：Swift 读 `kCGImagePropertyOrientation` 并用 CIImage 旋转（ImageImporter.swift:50-51）；Rust 完全无 EXIF 处理（无 exif 依赖）。
3. **【缺失】色彩管理**：Swift 强制渲染到 sRGB（ImageImporter.swift:52）；Rust `to_rgba8` 直接丢弃 ICC（Display-P3 数值原样当作 sRGB）（sheets_ui.rs:857）。
4. **【缺失】导入尺寸上限**：Swift 30000/边 + `remainingPixels` 预算（ImageImporter.swift:44-46）；Rust 无任何限制（sheets_ui.rs:855-866）。
5. **【行为差异】16→8 位降采样换算**：双方都降到 8 位，但 CI `.RGBA8` 渲染与 image crate 的缩放不同，无测试锁定。
6. **【行为差异】错误语义**：Swift 三类 `ImageImportError` 带固定文案（ImageImporter.swift:15-24）；Rust 把 image crate 错误字符串直接塞状态栏（sheets_ui.rs:895）。
7. **【缺失】导入会话语义**：Swift `EditorSession.importImages`（EditorSession.swift:720-780）有多文件串行队列、全文档 usedPixels 预算累计、drop 点居中放置、security-scoped resource、`psdOnly` 决定 undo 名（"Import Photoshop File"/"Import Images"）、失败聚合到 `importError`；Rust 单文件、固定放 `CGPoint::ZERO`（sheets_ui.rs:874-877）、undo 名 "Open"（sheets_ui.rs:879）、无错误聚合。

### ImageExporter（Swift `ImageExporter` ↔ Rust `export_png`）
1. **【缺失】JPEG 导出整体**：`JPEGOptions`（quality/RGB 哑光色）、`jpeg()`（不透明底填充、quality 0-1、DPI、1000px 预览 `JPEGResult`）、`ExportRaster`（ImageExporter.swift:95-123,141-154）在 Rust 全库无对应（grep 零命中）；Rust 只有 PNG（sheets_ui.rs:783-807）。
2. **【缺失】导出尺寸 guard**：Swift render 前校验 1...30000/边、≤100MP 报 `.tooLarge`（ImageExporter.swift:22-23）；Rust `export_png` 无 guard（sheets_ui.rs:783-798）。
3. **【缺失】DPI 元数据**：Swift PNG/JPEG 写 `kCGImagePropertyDPIWidth/Height`（ImageExporter.swift:80-82,111-113）；Rust `image.save` 不写 pHYs，分辨率信息丢失（sheets_ui.rs:802）。
4. **【行为差异】原子写**：Swift 经 `NSFileCoordinator` + `.atomic` 覆盖写（ImageExporter.swift:130-138）；Rust `image.save` 直接覆盖（sheets_ui.rs:802）。
5. **【行为差异】色彩空间标注**：Swift 显式 sRGB bitmap context（ImageExporter.swift:25-30）；Rust 假定数据即 sRGB，PNG 无 sRGB chunk。
6. **【缺失】`ExportError` 分类与文案**（tooLarge/render/encode，ImageExporter.swift:6-15）；Rust 用状态栏字符串。
7. 非预乘转换（unpremultiply）Rust 已实现（sheets_ui.rs:790-798），与 Swift CG 导出语义等价。

### RawImporter
- **【缺失】整个 RAW 导入域**：`RawDevelopSettings`、`Queue` actor（CIRAWFilter 缓存/scaleFactor/draft mode）、`matches`(.rawImage)、`asShot`、`develop`、`pixelSize`（RawImporter.swift:10-32,45-69,73-76,79-83,87-103,117-123）在 Rust 全部无对应。`crates/core/src/adjust/camera_raw.rs` 是 Camera Raw **滤镜**（作用于已解码位图），与 RAW 文件导入无关。Swift 的 RAW 分发在 EditorSession.swift:753-763，Rust 无。

### CompositorApplicationDelegate
- **【缺失】"Open With"/Dock 拖放/启动时打开文件**：`application(_:open:)` → `workspace.receive(urls)`（CompositorApplicationDelegate.swift:14-22）；Rust main.rs 无命令行参数或事件打开入口，唯一入口是 Open sheet（sheets_ui.rs:829）。

---

## 六、ImageFileDrop（URL 处理与错误语义）

- **【缺失】拖放导入整体无对应**。Swift `ImageFileDrop.importProviders`（ImageFileDrop.swift:7-33）：按 pasteboard 顺序解析 `UTType.fileURL`（URL 或 Data→`URL(dataRepresentation:)`）、`isFileURL` 校验、非文件项回退 `temporaryFile`（按 png/jpeg/heic/tiff/photoshop/raw/image 顺序探测、拷入唯一临时目录以保留原名、失败计 unreadable）、`unreadable && !providers.isEmpty` 时聚合追加错误消息（ImageFileDrop.swift:29-32）；以及错误语义"导入器校验内容而非信任扩展名"。`crates/app` 无任何 drop/drag-drop 文件处理（grep 仅命中画布工具拖拽）。

---

## 七、测试缺口清单

**PSDRoundTripTests.swift（21 例）→ `crates/core/src/psd/mod.rs` tests（19 例已移植）**

| Swift 用例（行号） | Rust 对应（mod.rs 行号） |
|---|---|
| roundTripLayersOrderVisibilityOpacityAndBlend (50) | round_trip_layers… (80) |
| roundTripGroupsMasksAndClipping (81) | (121) |
| importedGroupsFollowPhotoshopLsctOrder (111) | (160) |
| oversizedLayerBoundsAreRejected (128) | (190) |
| unusedSpotChannelsAreSkippedBeforeDecode (142) | (213) |
| unsupportedCompressionOnColorChannelsIsStillRejected (159) | (231) |
| matchesRequiresPhotoshopMagic (172，经临时文件) | (247，改为内存字节，见 mod.rs:246 注释) |
| unsupportedBlendProducesConversionReport (186) | (256) |
| softLightImportsWithoutConversion (199) | (279) |
| folderOpacityImportsOntoTheFolder (213) | (299) |
| unsupportedHeadersAreRejected (230) | (328) |
| vectorMaskIsRasterizedWithFillAndStroke (274) | (505) |
| photoshopShapeExtrasRasterizeInPlace (292) | (539) |
| fillEllipseImportsAsALiveShape (305) | (558)；断言从 `liveShape?.image === asset?.image`(322) 改为 asset 存在(589) |
| strokedRectangleImportsAsALiveShapeAndReportsTheStroke (325) | (594) |
| fourSharpCornersInferARectangleWithoutOrigination (344) | (632) |
| hugeOriginationSizeIsRejectedWithoutTrapping (357) | (656) |
| nonFiniteOriginationSizeIsIgnored (367) | (674) |
| hugeStrokeWidthIsRejectedWithoutTrapping (375) | (691) |

**未移植（2 例，均依赖 app 会话层）**：
- `importCreatesDocumentAndExistingCanvasGetsAGroup`（PSDRoundTripTests.swift:237）——依赖 `session.importImages` 的 PSD 分发与“二次导入建组”。
- `cancelledConversionLeavesTheDocumentUnchanged`（PSDRoundTripTests.swift:257）——依赖 `confirmConversions` 确认回调。

**ExportTests.swift（3 例，全部无 Rust 对应；crates/app 零测试）**：
- pngPreservesDimensionsAlphaOrientationAndTransforms (24)
- orderVisibilityClippingAndAtomicOverwrite (41)
- blankCanvasAndOversizedCanvas (66)（空画布 + 30000² 拒绝——Rust 连该 guard 都没有）

**JPEGExportTests.swift（2 例，功能本身缺失，无对应）**：
- transparencyUsesChosenMatteAndProducesOpaqueSRGB (8)
- qualityChangesBytesAndDecodedPixels (31)

**ImageImportTests.swift（8 例，全部无直接对应）**：
- supportedFormats (27，参数化 png/jpeg/tiff/heic——Rust 无 importer，且 tiff/heic 不支持)
- orientationAndColorConversion (39)
- pngPreservesTransparency (48)
- limitsAndInvalidFiles (62)
- importPlacementAndPartialFailure (84)
- dropPositionUsesDocumentCoordinates (101)
- queuedImportsAreNotLost (116)
- fileDropProvidersReachImporterInOrder (130)

**Fixture**：`PSDFixture.swift` ↔ `psd/fixture.rs` 完整移植；`PSDVectorFixtures.swift` ↔ `mod.rs` `vector_fixtures`（381-421，base64 串一致）。

**【多出】Rust 独有测试**（Swift 无同名用例）：reader.rs:701-729（cursor 大端/截断、unicode NUL 修剪、adjustment keys）、channel_coder.rs:179-218（RLE 随机行往返、128-run/-128、compression=2 拒绝）、vector.rs:828-900（shape_image 填充、bbox+flatten、rgb 描述符）。

---

## 附：优先级建议（按影响排序）
1. 【缺失】app 未接线 PSD 导入（`open_document` 增加 `PsdReader::matches` 分发）——README 声明目前不成立。
2. 【缺失】JPEG 导出、EXIF 方向、导入尺寸预算、HEIC/TIFF。
3. 【缺失】拖放（ImageFileDrop）、Open With、RAW 导入。
4. 【行为差异】图层名 macOSRoman 解码、PNG DPI 元数据、原子写。
5. 【简化】CG→scanline 光栅化（vector.rs 已声明）、composite 重绘→resample（fixture.rs:335）、live-shape 几何化（types.rs:246）。
