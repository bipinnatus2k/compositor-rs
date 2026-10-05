# 可用成熟 Crate 替换的手写逻辑盘点

> 2026-10-01 · 基于 `dev-ui-fix` 分支全量扫描(crates/core 约 21k 行,crates/app 约 21k 行)。
> 每一条均给出精确代码位置、替换 crate、收益与风险。风险判断的核心约束:本项目以
> "与 Swift/Core Image 行为逐位对齐" 为验收标准(`tests` 目录大量断言锁定像素级行为),
> 任何替换都必须让现有测试原样通过,否则宁可保留手写实现。

## 0. 一个重要前提:gpui-kit 已经把这些 crate 拉进了依赖树

`Cargo.lock` 显示以下候选 crate **已经在编译**,只是本项目没有直接依赖它们:

| crate | 锁文件版本 | 来源 |
|---|---|---|
| `euclid` | 0.22.14 | gpui-kit 几何 |
| `fontdb` | 0.23.0 | gpui-kit 字体 |
| `cosmic-text` | 0.19.0 | gpui-kit 文本 |
| `lyon` | 1.0.19 | gpui-kit 路径 |
| `tempfile` | 3.27.0 | 传递依赖 |
| `thiserror` | 2.0.20 | 传递依赖 |
| `crc32fast` | 1.5.2 | 传递依赖 |
| `rand` | 0.8 / 0.9 / 0.10 | 传递依赖 |
| `same-file` | 1.0.6 | 传递依赖 |
| `dirs` | 5.0.1 / 6.0.0 | 传递依赖 |
| `png` | 0.17.16 / 0.18.1 | image 0.25.10 |

采用它们几乎不增加编译成本,只需要在 `crates/app/Cargo.toml` / `crates/core/Cargo.toml` 显式声明。

---

## 1. 第一优先级:直接替换,低风险高收益

### 1.1 `dirs` — 两份重复的手写 `config_dir`

- 位置:`crates/app/src/shortcuts.rs:577-595`、`crates/app/src/settings.rs:310-333`
- 现状:两处**完全重复**的手写实现(Windows `%APPDATA%` / macOS / XDG 分支),且代码注释自认
  "hand-rolled to avoid the dependency"。
- 替换:`dirs::config_dir()`,一处调用替换约 20 行,并消除双份漂移风险。
- 风险:无。`dirs` 已在依赖树中(5.x/6.x),行为与手写分支一致。

### 1.2 `tempfile` — 两份手写原子写 + 四份测试临时目录

- 原子写位置:`crates/app/src/shortcuts.rs:527-545`、`crates/app/src/settings.rs:193-209`
  (临时文件 + rename,注释提到模仿 `MoveFileEx`)。
- 测试临时目录位置:`shortcuts.rs:670-685`、`settings.rs:346-360`(时间戳+计数器版)、
  `project.rs:921-927`、`tabs.rs:797-804`(uuid 版),各约 15 行,4 处重复。
- 替换:
  - 原子写 → `tempfile::NamedTempFile::new_in(dir)` + `persist(path)`(Windows 上即
    `MoveFileEx(MOVEFILE_REPLACE_EXISTING)`,语义相同);
  - 测试目录 → `tempfile::TempDir::new()`,顺带获得 drop 时自动清理。
- 风险:无。`tempfile` 3.27.0 已在树中。

### 1.3 `thiserror` — 手写的 `ShortcutError`

- 位置:`crates/app/src/shortcuts.rs:216-278`
- 现状:手写 `Display`(逐变体格式化)+ `impl std::error::Error` + `From<io::Error>`,
  共约 60 行;这是 app 内唯一一处手写 error 类型。
- 替换:`thiserror::Error` 派生宏,可省约 40 行。`thiserror` 2.0.20 已在树中。
- 风险:无。

### 1.4 `crc32fast` — 手写 CRC-32

- 位置:`crates/core/src/export.rs:116-126`(`crc32`,PNG `pHYs` 块校验用)
- 现状:逐位循环实现,每个字节 8 次迭代,无查表。
- 替换:`crc32fast::hash(data)`(1.5.2 已在树中),SIMD/查表加速,API 一行。
- 风险:无,输出位相同(IEEE 802.3 reflected)。

### 1.5 `rand` — 两处 SystemTime 随机种子 + LCG

- 位置:`crates/app/src/state.rs:452-459`(`time_seed`)、`crates/app/src/canvas.rs:949-953`
  (内联 SystemTime 种子)、`crates/core/src/tools/content_fill.rs:9-17`(手写 LCG `Rng`)。
- 替换:`rand` 已在树中;种子用 `rand::random::<u64>()`,content_fill 的 LCG 可换
  `rand_pcg`(或保守地保留 LCG——它的确定性被 patchmatch 输出隐式锁定)。
- 风险:低。注意 content_fill/Grain/AddNoise 的**确定性**是需求(测试断言同 seed 同输出),
  换 RNG 必须保留 seed 语义,只换实现来源。

### 1.6 `bytesize`(或 `humansize`)— 手写文件大小格式化

- 位置:`crates/app/src/state.rs:1919-1938`(`byte_count_label`:十进制单位、<10 保留 1 位小数)
- 替换:`bytesize::ByteSize` 或 `humansize::format_size`。注意两者默认是二进制单位(KiB),
  需配置十进制;若只想保留现有显示格式,提取共享 helper 亦可,crate 是可选。
- 风险:低(显示层,测试若有格式断言需同步)。

---

## 2. 第二优先级:中等收益,需要少量适配

### 2.1 `fontdb` — 两份手写系统字体枚举/查找(app 与 core 各一份,已互相漂移)

- 位置:
  - `crates/app/src/tools.rs:951-985`(`available_font_names`:6 个硬编码字体目录、
    ttf/otf/ttc 过滤、file_stem、排序去重、OnceLock 缓存)
  - `crates/core/src/tools/text.rs:17-46`(`FONT_DIRECTORIES` + `FONT_SUBSTITUTES` +
    `find_font_file` + `font_cache`,core 侧另有自己的字体目录表——**与 app 侧列表不一致**)
- 替换:`fontdb::Database::load_system_fonts()` + family 索引。`fontdb` 0.23.0 已随
  gpui-kit 在树中(resvg 同款),跨平台目录覆盖比 6 个硬编码路径完整得多。
- 适配:`FONT_SUBSTITUTES` 的 Helvetica→Arial/SegoeUI 代换逻辑可保留在上层,用
  `db.query(&Query { families: &[Name("Helvetica"), Name("Arial"), …], .. })` 表达,
  反而更干净。
- 风险:低-中。字体**名称集合**可能变化(例如 fontdb 返回真 family 名),菜单展示层需回归
  测试;core 的文字栅格化仍走 fontdue,只需 fontdb 给出文件路径后 `Font::from_bytes`。

### 2.2 `fontdue::layout`(已依赖)或 `cosmic-text`(已在树中)— 手写逐词换行与逐字符测量

- 位置:
  - `crates/app/src/inline_text.rs:173-210`(`wrapped_line_break`:按空格 `split_inclusive`
    逐词换行)、`:176-187`(`advance_of`:逐字符 advance + tracking,无字体按 0.6em 兜底)
  - `crates/core/src/tools/text.rs`(文本 box 测量与栅格化,同样是逐字符累计)
- 替换:`fontdue` 已是直接依赖,其 `layout` 模块自带 `LayoutSettings` + `WrapStyle`
  (词换行)+ `HorizontalAlign`,可直接吃 `fontdue::Font`;更进一步的 `cosmic-text` 0.19
  已在树中,带来 bidi、复杂文种 shaping、kerning/ligature——手写逐字符 advance 全部缺失这些。
- 风险:**中**。Swift 版(AppKit TextKit)排出的换行点与行宽会被测试/UI 对拍;换行算法
  变化会改变可见输出。建议先只替换 core/app 重复的"测量"部分,换行点保持手写,或者
  接受一次显式的"文本排版不再逐位对齐 Swift"决定。

### 2.3 `png`(已在树中)— 手写 PNG `pHYs` 块注入

- 位置:`crates/core/src/export.rs:88-113`(`inject_png_dpi`:手工拼 chunk + CRC)
- 替换:绕过 `image` 的高级 API,直接用 `png::Encoder::set_pixel_dims(PixelDimensions {
  xppu, yppu, unit: Unit::Meter })`(0.17.10 起支持;树内 0.17.16 满足),让编码器自己写
  `pHYs`,删掉整个手写 chunk 注入和 CRC。
- 风险:低。测试 `png_dpi_injects_a_phys_chunk`(export.rs:298-314)可直接验证。

### 2.4 `jpeg-encoder` — 手写 JFIF 密度补丁

- 位置:`crates/core/src/export.rs:164-182`(`patch_jfif_density`:事后改写字节流 APP0 段)
- 替换:`jpeg-encoder` 0.7.1(纯 Rust,零拷贝 API)原生支持 `PixelDensity` +
  `PixelDensityUnit::Inches`,编码时一步写对,替代 `image::JpegEncoder` + 字节补丁两步。
- 风险:低-中。JPEG 编码器换掉后,同 quality 的字节输出**必然不同**;`jpeg_round_trips…`
  测试断言的是解码后像素(>230 等),大概率仍过,但需跑全量测试确认。

### 2.5 `palette` — HSB↔RGB 转换与散布 6 处的 0-255 缩放

- 位置:
  - `crates/core/src/document.rs:90-160`(`PickerHSB::from_rgb` / `rgb`:手写 HSB 公式)
  - `crates/core/src/document.rs:57-88`(`hex()` / `from_hex`,可保留,很短)
  - f64→u8 颜色缩放重复 6 处:`crates/app/src/state.rs:1897-1904`、
    `crates/app/src/workspace.rs:263-270`、`crates/app/src/tools.rs:762-765`、
    `crates/app/src/tools.rs:1137-1139`、`crates/app/src/color_picker_ui.rs:285-287` 与
    `:713-718`、`crates/app/src/panels_ui.rs:1388-1389`
- 替换:`palette` 0.7.7 的 `Hsv`/`Srgb` 转换(`from_color`/`into_color`,f64 组件),
  以及统一的 `Srgb::into_format::<u8>()` 处理量化。6 处缩放至少应收敛成一个共享 helper,
  是否引入 `palette` 二选一。
- 风险:中。`PickerHSB` 的 hue 度数制、"拖过灰色保持 hue"语义被拾色器测试锁定;
  palette 的 Hsv 色相是归一化浮点,转换时要乘 360。换算公式本身是标准公式,回归测试
  能覆盖,但建议只把**新的**颜色代码迁到 palette,旧路径暂不动。

### 2.6 `same-file` — 路径等价比较

- 位置:`crates/app/src/project.rs:906-912`(`same_path`)、`crates/app/src/tabs.rs:296-308`
- 现状:`canonicalize` 后比较(依赖 fs 调用,对不存在路径返回 Err)。
- 替换:`same_file::is_same_file(a, b)`(1.0.6 已在树中),语义更准(检测硬链接/大小写)。
- 风险:低,边际收益。

---

## 3. 第三优先级:大改造,测试锚定行为,需逐项评估

### 3.1 `euclid` — 手写 CoreGraphics 几何类型(geometry.rs 全文 394 行)

- 位置:`crates/core/src/geometry.rs:15-43`(CGPoint)、`:59-74`(CGSize)、
  `:76-240`(CGRect 含 intersection/union/integral/transformed)、
  `:242-351`(CGAffineTransform 含 **CG 语义的 concat 顺序**)
- 替换:`euclid::point2/size2/rect/Transform2D`(0.22.14 已随 gpui-kit 在树中,
  开 `serde` feature 即有序列化)。
- 风险:**高,不推荐整体替换**:
  1. `CGAffineTransform::concat` 的复合顺序(`p' = M_self · M_other · p`)与 euclid 的
     `Transform2D::then/post_transform` 方向需要对齐,这正是 git 历史里修过的
     "CG concat gotcha"(见 memory 与 geometry.rs:304-318 注释),测试
     `affine_inverts_roundtrip` 等锚定了行为;
  2. 序列化格式:项目 manifest 是 Swift-exact JSON 键,CGRect 序列化为
     `{origin:{x,y},size:{w,h}}`,而 euclid `Rect`/`Box2D` 序列化为 `{origin,size}` /
     `{min,max}` 形状不同,会破坏 `.comp` 兼容;
  3. `CGRect::NULL`(无穷原哨兵)语义贯穿全库。
- 建议:保留 geometry.rs 作为"CG 兼容层";若想减少手写,只把 **app 层新增**几何代码
  引向 gpui 自带的几何类型(渲染边界),core 不动。

### 3.2 `libblur` / `image::imageops` — 手写高斯模糊与重采样(raster.rs 约 600 行)

- 位置:
  - `crates/core/src/raster.rs:797-836`(`gaussian_blur_rgba`:分离核、边界 clamp、同尺寸输出)
  - `:838-867`(`gaussian_blur_gray`)、`:884-897`(`gaussian_kernel`)
  - `:320-402`(`bilinear_sample` / `resample`:Nearest/平滑/High 三档 + 逐级减半链)
  - `:428-588`(Lanczos3 采样、查表化分离 resize——含一个与参考实现逐位对拍的测试)
- 候选:
  - `libblur` 0.24(2026-09 仍在发版):4 通道 + `EdgeMode::Clamp`,API 面向裸 u8 缓冲,
    是最接近的替换;性能(线性时间盒近似)远好于当前的 O(radius) 直接卷积——当前实现
    radius=4σ,大 sigma 时明显慢。
  - `image::imageops::resize`(`FilterType::Lanczos3`/`Triangle`/`Nearest`)可替换
    `lanczos_resize` 的独立用法;但注意 `image` 工作在**直通 alpha** 的 `RgbaImage` 上,
    本项目 Bitmap 是**预乘 alpha**,直接替换会在半透明边缘产生光晕。
- 风险:**高**。这些函数的边界语义(边缘 clamp、CI 的"透明外延"由 filters.rs 的
  `grow_to_fit` 补齐)、Lanczos 的相位((x+0.5)/scale-0.5 中心对齐)、以及逐级减半链
  (`DownsampleCache` 移植)都有逐位测试。建议:**只在性能成为热点时**替换
  `gaussian_blur_rgba/gray` 内核(用 libblur 或自写盒近似三 pass),API 与语义壳保留;
  `lanczos_resize` 有查表优化且测试完备,保留。

### 3.3 `imageproc::geometric_transformations::warp` — 手写仿射绘制

- 位置:`crates/core/src/raster.rs:662-791`(`draw_affine`:逆向映射采样 + High 预缩 +
  轴对齐 Lanczos 快路径 + source-over 合成)
- 风险:**高,不推荐**。它是渲染管线的核心:预乘 source-over、`exact` 快路径、High 质量分档
  都是 Swift/Core Graphics 对齐行为;imageproc `warp` 输出的是独立图像而非"合成到 dst",
  替换等于重写合成器。保留。

### 3.4 `imageproc::morphology` — 手写二值腐蚀/膨胀

- 位置:`crates/core/src/tools/object_selection.rs:200-240`(`eroded` / `dilated`,
  3×3 十字/方形结构元)
- 替换:`imageproc::morphology::erode/dilate`(支持 `Norm::Linf/L1`),适配需包一层
  `ImageBuffer<Luma<u8>>`。收益小(约 40 行),可换可不换;换则注意结构元形状要一致,
  否则 `edge_offset_erodes_and_dilates_by_whole_pixels` 测试会失败。

### 3.5 `imageproc` 其余可能点 — 魔棒扫描线泛洪

- 位置:`crates/core/src/tools/wand.rs:36-80+`(`wand_mask`:种子半径取参考色、逐通道
  容差、连续/非连续两模式、扫描线泛洪、返回匹配计数)
- `imageproc::region_labelling::flood_fill` 是"改写像素"式 API,不产出 mask、不含容差
  参考色语义;适配成本高于收益。保留,但**轮廓追踪**(`wand_trace`)部分若日后需要
  更高质量的轮廓,可参考 `contour_tracing`/`pathfinding` 类 crate。当前保留。

---

## 4. 评估后决定保留的手写实现(没有成熟 crate 或替换即破坏对齐)

| 模块 | 位置 | 为什么保留 |
|---|---|---|
| 24 种 PDF/Photoshop 混合模式 | `crates/core/src/blend.rs`(548 行) | 无成熟 crate 以"直通 alpha 浮点 + 逐像素函数"形式提供全部 24 模式;公式即标准,手写是合理载体 |
| 快照式撤销/重做(字节限额、嵌套事务、保存修订点) | `crates/core/src/history.rs`(472 行) | `undo` crate 是**逆命令**模型,与"整文档快照 + Arc 共享 + 字节限额裁剪"模型不匹配;移植成本>收益 |
| 内容识别填充(patchmatch)/ 修复画笔 | `crates/core/src/tools/content_fill.rs`(319 行)、`heal.rs`(663 行) | 生态里没有可用的 patchmatch crate;确定性输出被测试锁定 |
| 对象选择的边缘保持上采样 | `crates/core/src/tools/object_selection.rs`(544 行) | 对齐 `CIEdgePreserveUpsampleFilter` 的联合双边滤波,无对应 crate |
| PSD 读取器(PackBits、矢量蒙版、调整映射) | `crates/core/src/psd/`(约 2870 行) | `psd` crate 停更在 0.1.x;`zune-psd`/`rawpsd` 只覆盖基础栅格层,不支持矢量蒙版与调整信息——手写读取器仍是唯一完整方案 |
| 选区多边形布尔/栅格化 | `crates/core/src/selection.rs`(布尔已用 `geo`;`rasterize` 手写,~600 行) | 布尔部分已是最优解(geo);抗锯齿 coverage 栅格化无直接对应 crate(lyon 输出三角网而非 coverage mask) |
| 加性噪声的坐标键控哈希(Box–Muller) | `crates/core/src/filters.rs:233-286` | 坐标键控确定性噪声是 hash-noise,不是流式 RNG;`rand`/`noise` 都不直接对应 |
| 运动模糊 / 镜头畸变 | `crates/core/src/filters.rs:289-342, 418-471` | `CIMotionBlur` 高斯锥形核与径向畸变是特定对齐目标,imageproc 无运动模糊 |
| 相机原始调整(4235 行) | `crates/core/src/adjust/camera_raw.rs` | 对齐 Camera Raw 的曝光/曲线/鲜艳度等,数值语义专属 |
| 快捷键和弦标签/录制 | `crates/app/src/shortcuts.rs:626-662`、`sheets_ui.rs:1076-1093` | gpui-kit 无对应 API;平台拼写差异是业务本身 |

---

## 5. 非 crate 但值得顺手做的去重(扫描中发现)

1. **文件对话框样板 ×3**:`crates/app/src/welcome.rs:124-157`、`flows.rs:119-142`、
   `workspace.rs:584-615` 几乎逐字重复的 `cx.prompt_for_paths(...)` + spawn + 双层
   `unwrap_or_default()`——提一个公共 helper。
2. **容错加载 ×2 / 进程级单例 ×2**:`shortcuts.rs:489-573` 与 `settings.rs:165-279`
   的"版本检查+损坏回退+OnceLock 毒锁恢复"是同一模式的两份拷贝,可提取泛型工具。
3. **RGBA→BGRA 交换 ×2**:`state.rs:1907-1915` 与 `color_picker_ui.rs:585-588`。
4. **拖放样板 ×3**:`canvas.rs:90-108`、`welcome.rs:189-204`、`workspace.rs:402-415` 的
   `ExternalPaths` drag_over/on_drop 重复。
5. **按字符数估文本宽度**:`workspace.rs:277-287`("每字符 7px"注释自认是估算),
   gpui 有真实文本测量可替换。

---

## 6. 建议执行顺序

1. **零风险批次**(一个下午):`dirs`、`tempfile`(原子写 + 测试目录)、`thiserror`、
   `crc32fast`、`rand` 种子。全部已在依赖树,删代码为主。
2. **低风险批次**:`png::Encoder::set_pixel_dims` 替换 pHYs 注入;`jpeg-encoder` 替换
   JFIF 补丁;`fontdb` 统一两份字体枚举;`same-file`;`bytesize`。
3. **需要回归测试护航的批次**:`fontdue::layout`/`cosmic-text` 接管换行测量(先 core/app
   共享测量,再换行点);`palette` 接管 HSB 与颜色量化。
4. **按需批次(性能驱动)**:`libblur` 替换高斯模糊内核(API 壳不动)。
5. **明确不做**:geometry→euclid 整体替换、draw_affine→imageproc、PSD 读取器、
   混合模式、快照式 undo。

---

## 附:核实来源

- fastblur 为 RGB-only、多通道 WIP:[docs.rs/fastblur](https://docs.rs/fastblur)(因此 RGBA 场景改推 libblur)
- libblur 0.24.0、FastBlurChannels/EdgeMode API:[docs.rs/libblur](https://docs.rs/libblur/latest/libblur/)
- png crate `set_pixel_dims`/`PixelDimensions`(0.17.10+):[DeepWiki: image-rs/image-png](https://deepwiki.com/search/how-do-i-write-a-png-with-a-ph_285036a2-5d01-48b0-a300-002839f74389)
- jpeg-encoder 0.7.1 `PixelDensity`/`PixelDensityUnit`:[docs.rs/jpeg-encoder](https://docs.rs/jpeg-encoder/latest/jpeg_encoder/)
- fontdue layout 模块(WrapStyle/HorizontalAlign/LayoutSettings):[docs.rs/fontdue layout](https://docs.rs/fontdue/latest/fontdue/layout/index.html)
- fontdb `load_system_fonts`(resvg 同款):[crates.io/crates/fontdb](https://crates.io/crates/fontdb)、[docs.rs/fontdb Database](https://docs.rs/fontdb/latest/fontdb/struct.Database.html)
- palette 0.7.7 `Hsv`/`Srgb` 转换:[docs.rs/palette](https://docs.rs/palette/latest/palette/)
- PSD 生态现状(psd 停更 / zune-psd / rawpsd):[chinedufn/psd](https://github.com/chinedufn/psd)、[docs.rs/zune-psd](https://docs.rs/zune-psd)
- `euclid`/`cosmic-text`/`lyon`/`tempfile`/`thiserror`/`crc32fast`/`rand`/`same-file`/`dirs` 已在依赖树:本项目 `Cargo.lock`(gpui-kit 0.7.0 传递依赖)
