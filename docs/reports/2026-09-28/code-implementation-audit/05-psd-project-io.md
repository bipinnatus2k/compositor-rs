# 文件 IO 子系统实现审计：PSD 读取 / 项目存储（2026-09-28）

审计范围：`crates/core/src/psd/`（mod.rs、reader.rs、types.rs、channel_coder.rs、document_builder.rs、vector.rs、fixture.rs）与 `crates/core/src/project/`（mod.rs、store.rs、manifest.rs、convert.rs、workspace.rs），合计约 7000 行。所有 12 个文件均完整通读；关键论断另行与 Swift 原始源（`Compositor/Compositor/IO/PSD/PSDReader.swift`）和 `crates/app` 接线代码交叉核对，并用 `cargo check -p compositor-core` 验证可编译。旧报告 `docs/agent-reports/G-io-psd.md`（2026-09-24）仅作交叉核对对象，不作为事实来源。行号均为今日工作区实读行号。

## 一、总览表

### PSD 各段（reader 实际覆盖）

| 段 / 功能 | 实际状态 |
|---|---|
| File Header（8BPS/version/尺寸/depth/mode/预算） | 完整实现（reader.rs:38-64；画布 1..=30000、≤100MP、8-bit RGB） |
| Color Mode Data | 完整实现（reader.rs:66-67，读长度跳过；RGB 模式下恒空，够用） |
| Image Resources | 部分：只解析 `8BIM` 资源且只取 id 1005 分辨率（reader.rs:71-97）；ICC 配置、缩略图、网格等全部跳过（与 Swift 逐行一致的设计裁剪，非缺陷） |
| Layer Records（bounds/通道表/blend/opacity/clipping/flags/extra） | 完整实现（reader.rs:138-227：luni、iOpa、lsct/lsdk、任意附加段全量收进 `extra` HashMap） |
| 全局蒙版信息 / 全局 tagging | 跳过（reader.rs:127-129 注释自认，与 Swift 等价——之后无读取） |
| 通道解码（PackBits/RLE、raw） | 完整实现（channel_coder.rs:15-126）；只解 `{-1,0,1,2,-2}`，spot 等多余通道按长度跳过（reader.rs:230,249-263）；ZIP/16-bit 压缩报 `UnsupportedCompression`——与 Swift 相同的限制 |
| Group / 分隔符（lsct 3/1/2 栈式配对） | 完整实现（reader.rs:301-371，未闭合组报 Truncated） |
| 层蒙版 | 基本完整但有缺陷：矩形原点（mask_left/top）读取后仅用于尺寸，偏移蒙版被按图层左上角放置（见发现 11） |
| Text（TySh/tySh/txt2） | 部分：不解析文本内容，仅标记 kind 并栅格化像素 + 转换报告（reader.rs:384、document_builder.rs:56-58）——设计如此，与 Swift 一致 |
| Smart object（SoLd/SoLE） | 部分：标记 + 栅格化 + 报告（reader.rs:390-391、document_builder.rs:59-61） |
| 图层效果（lfx2/lrFX/lmfx） | 部分：标记 + 丢弃 + 报告（reader.rs:393-394、document_builder.rs:62-64）；fill/opacity 乘积规则与 Swift 一致（PSDReader.swift:263-266 ↔ reader.rs:323-329） |
| 调整层 | 部分：仅 levl/curv/hue␣/hue2 三类解析（reader.rs:525-536）；ADJUSTMENT_KEYS 共 16 个 key（reader.rs:402-405），其余 12 类只标记 kind 并报 "not supported and was skipped"（document_builder.rs:90-101） |
| ICC 色彩管理 | 无（不读 ICC；与 Swift 一致不读） |
| Vector（vmsk/vsms/vogk/SoCo/vstk） | 基本完整但有缺陷：live/raster 双路径 + 自研解析积分光栅化（非 CG 位级一致，vector.rs:730-749 已声明）；存在一处越界 panic（发现 1） |
| PSD 写出 | stub 级（有意为之）：唯一的字节级写出器是测试 fixture（fixture.rs:1-4 注释 "Compositor does not write PSD"）；应用无 PSD 导出路径，与 Swift 一致。README 若称 "PSD reader/writer" 仅对应此测试 fixture |

### Project store 各功能块

| 功能块 | 实际状态 |
|---|---|
| manifest 校验（版本门控 1..=9、层级 64 深度、live-mask 链 ≤256、guides、命名/路径穿越、像素预算） | 完整实现（store.rs:38-247），规则密度高且有 20+ 测试锁定 |
| load（包读取 + 资产解码） | 完整实现：符号链接/目录/超限防护（store.rs:268-284）、APNG 拒绝（store.rs:302-304）、16-bit 拒绝（307-312）、像素预算在解码前检查（313） |
| save（事务性替换） | 基本完整但有缺陷：validate→全量编码→staging 目录→双 rename→失败回滚（store.rs:455-559）；无 fsync、进程被杀窗口内原路径短暂无包且不自动恢复、`.staging-*`/`.backup-*` 残留无垃圾回收（发现 4） |
| 垃圾回收（旧资产清理） | 完整实现：整包替换即回收（测试 store.rs:1170-1190 锁定旧资产消失） |
| convert 双向（ProjectData ↔ CanvasDocument） | 完整实现（convert.rs:30-196，加载时重校验） |
| workspace 跨项目拷贝 | 基本完整但有缺陷：≥64 长 clipping 链的层拷贝时像素被静默清空（发现 2） |

## 二、逐条发现

### 高严重度

- **[高] `unit_field` 对恶意/损坏 PSD 越界 panic** — `crates/core/src/psd/vector.rs:326-330` + `crates/core/src/psd/types.rs:279-281`。`unit_field` 先 `find_offset(data, b"UntF", key_at)`（只保证 `unit_at+4 ≤ len`），随后无条件 `f64_at(data, unit_at + 8)`，展开为 `u64_at` 的 `data[unit_at+8 .. unit_at+16].try_into()`——切片越界在 `try_into` 之前直接 panic。同文件的 `bool_field`（vector.rs:320 `type_at + 5 > data.len()`）、`int32_field`（vector.rs:335 `type_at + 8 > data.len()`）、`double_field`（vector.rs:344 `type_at + 12 > data.len()`）都做了前置边界检查，唯独 `unit_field` 没有（需要 `unit_at + 16 ≤ len`）。可达性：`layer.extra` 的值原样来自文件字节（reader.rs:223）；`live()` → `origination()` → `unit_field("Left"/"Top "/...)`（vector.rs:215-218、238）由 reader.rs:342 对**所有非组层**调用；`raster()` → `unit_field("strokeStyleLineWidth")`（vector.rs:119）由 reader.rs:350 调用。构造一个 `vogk`/`vstk` 附加段、让 `"UntF"` 出现在 payload 末尾不足 16 字节处即可让导入线程 panic（gpui 桌面应用即整体崩溃）。这是本子系统唯一的 panic 级输入路径；其余全部字节读取（PsdCursor、levels/curves/hue、unicode_name、macos_roman 表、fill_polygons 索引、decode RLE 边界）经逐点核查均有防护。

### 中严重度

- **[中] 跨项目复制 ≥64 长 clipping 链的层会静默丢失像素** — `crates/core/src/project/workspace.rs:73-87`（85 行）+ `crates/core/src/session.rs:3704-3705`。manifest 校验允许 clipping 链最长 256（store.rs:194-224），但 `bake_live_mask` 在链深 ≥64 时返回 `None`（session.rs:3704 `if depth >= 64 { return None; }`）；workspace.rs:85 `layer.asset = source.bake_live_mask(layer.id);` 把返回值直接覆盖到 asset 上且无回退——链长 64..=255 的被复制像素层，原本的像素会被替换成"无 asset"，mask_source_id 同时清空，结果是一个空白层。两个常量（64 与 256）没有对齐。
- **[中] `fill_polygons` 为整个目标位图分配 coverage 缓冲** — `crates/core/src/psd/vector.rs:786`。`vec![0.0f32; target.width * target.height]` 分配全图 4 字节/像素的覆盖缓冲，而实际只有 `[x0..x1) × [y0..y1)` 窗口被写入（786-884）。在 100MP 形状预算内（vector.rs:19、193），一个全幅 live shape 导入会先分配 400MB 位图、再分配 400MB f32 coverage，峰值约 0.8-1.2GB。缓冲只需窗口行 × 窗口列。
- **[中] save 的崩溃恢复声明与实现不符；无 fsync；残留无回收** — `crates/core/src/project/store.rs:517-520`（注释）vs `512-553`（代码）。注释声称 "a crash between the two renames leaves the old (or new, complete) package intact, **never a missing destination**"：实际在 `fs::rename(dir, backup)`（525-527）与 `fs::rename(staging, dir)`（529）之间进程被杀，原路径上就是没有包——数据完整但躺在 `name.backup-<uuid>` 里，应用侧 `load` 会直接报 Invalid，需要用户手工改名恢复；注释的 "never a missing destination" 不成立。此外 staging 里的文件写入（513-515）与两次 rename 均无 fsync，断电可能使新包内容未落盘；崩溃残留的 `.staging-*`（512）与 `.backup-*`（521）目录没有任何清理或自动恢复逻辑，会永久累积。store.rs:8-9 模块注释倒是对自己承认是 "best-effort remove-and-rename"——两处注释口径不一致，517-520 的强声明是错的那个。
- **[中] save 路径的全内存编码 + 三重像素克隆** — `crates/core/src/project/store.rs:455-493`（全部 PNG 先编码进 `BTreeMap<String, Vec<u8>>`）、`store.rs:363`（`encode_image_png` 为反预乘再 `bitmap.data.clone()` 一整份）、`crates/core/src/project/convert.rs:134-138`（`project_from_document` 把每个图层位图完整克隆进 `images`）。对一个 100MP 项目，save 一次瞬时持有：位图映射一份（400MB）+ 编码前克隆一份（400MB）+ 全部 PNG 字节。全部编码完成后才进入 staging 写盘，没有流式写出。
- **[中] 非 group 层的 `"pass"` blend key 被静默压成 Normal 且不进转换报告** — `crates/core/src/psd/document_builder.rs:84`（`... && record.blend_key != "pass"` 把 "pass" 排除出报告）+ `document_builder.rs:104`（`blend_mode_from_psd("pass")` 为 None → `unwrap_or(Normal)`）。正常 PSD 不会给普通层写 pass-through，但畸形文件或 fixture 产出的 `PsdRecord` 可以（assemble 只对组改写 "pass"，reader.rs:316-320；fixture.rs:81 按记录原样写出非组的 blend_key）。结果是 blend 丢失且用户得不到任何提示。低概率、继承自 Swift 语义的边缘，但"静默无报告"与该模块其它所有转换都有报告的风格相悖。

### 低严重度

- **[低] 死代码：`PsdChannelCoder::image` / `PsdChannelCoder::mask_image`** — `crates/core/src/psd/channel_coder.rs:55-60、62-68`。全工程（crates/ 下 grep + cargo check 警告核查）零调用点；`crates/core/src/psd/mod.rs:6` 的 `#![allow(dead_code)]` 把它们连同其它未用项一起屏蔽，不产生任何编译警告。这两个函数是对 Swift `image`/`maskImage` 的保真移植（Swift 在 decodeChannels 使用），Rust 侧 reader.rs:267、287 改用 `GrayMask::from_raw`/`rgba_bitmap` 直连，于是成了孤儿。
- **[低] `MAX_SHAPE_PIXELS` 双常量重复** — `crates/core/src/psd/vector.rs:19`（i64 100_000_000）与 `crates/core/src/document.rs:478`（usize 100_000_000）是两个独立定义的同值常量；`mod.rs:20` 的 `pub use` 在 psd 模块之外无使用者。
- **[低] `write_package` 的 if/else 两分支代码完全相同** — `crates/core/src/project/store.rs:524-528`。`if existing.is_dir() { fs::rename(dir,&backup) } else { fs::rename(dir,&backup) }`，死条件（无害，但说明此处是匆忙写就）。
- **[低] fixture 的 `encoded[..height * 2]` 切片无防护** — `crates/core/src/psd/fixture.rs:345-349`。若 `encode_plane` 因 `plane.len() < width*height` 返回空 `(0, Vec::new())`（channel_coder.rs:160-162），此处会 panic。当前调用链 `planes()`/`resample` 恒产出精确尺寸平面，实际不可达；fixture 又是测试专用，故仅记为脆弱点。
- **[低] PSD 层蒙版矩形原点被丢弃（继承自 Swift 的保真限制）** — `crates/core/src/psd/reader.rs:169-172` 读取 mask_left/top/bottom/right，但后续只使用宽高（reader.rs:236-237、255-256）；蒙版平面恒按图层自身 bounds 左上角应用（document_builder.rs:190-198 中 `placement: None`）。Photoshop 的蒙版矩形可与图层矩形错位，此类文件的导入位置会错。已核对 Swift（PSDReader.swift:139-142）同样只用于尺寸——这是两侧一致的共同缺陷，非 Rust 独有。
- **[低] `ManifestShape.start/end` 永不写出、读入即弃** — `crates/core/src/project/manifest.rs:356-361`（字段声明，注释自称 "kept for fidelity"）、`manifest.rs:372-374`（写出恒 `None`）、`manifest.rs:388-391`（读入把 line 归零到 (0,0)-(1,1)）。Swift 写出的 line 端点在 Rust 读回时静默丢弃。
- **[低] hue2 按 legacy `hue␣` 的二进制布局解析** — `crates/core/src/psd/reader.rs:532、627-664`。已核对 Swift 逐字节一致（PSDReader.swift:369、424-441），属忠实移植；但按 Adobe 规范 `hue2` 是 descriptor 布局而非 legacy 定长结构，对真实 Photoshop 文件的 hue2 调整层，colorize 位（reader.rs:631 `data[2] != 0`）与 7×6 字节 range 读数（643-651）可能取到错误数值。两侧同源的可疑实现，本仓库内无法进一步证实，标注存疑。
- **[低] `load` 的 `check_file` 与 `fs::read` 之间存在 TOCTOU 窗口** — `crates/core/src/project/store.rs:394-395`（manifest）及 426-435（资产）。检查与读取非原子；威胁模型（本地用户自己的包）下风险极低。

### 注释/文档与代码相符性抽查（无漂移项）

以下注释经与代码逐条对照**属实**，列出以正视听：`reader.rs:127-129`（丢弃游标等价于 Swift 的 layerSectionEnd 跳转——Swift PSDReader.swift:86 确认）；`reader.rs:184-185`（macOSRoman 解码，代码 reader.rs:472-501 完整实现了 128 项表，与注释一致）；`reader.rs:261-262`（对应 Swift defer 推进）；`document_builder.rs:191-192`（蒙版失败分支不可达、消息未移植）；`vector.rs:730-749`（解析积分器为声明式替代 CG，代码 750-903 与描述一致）；`fixture.rs:1-4`（fixture 非应用写出路径）；`mod.rs:6` 的 `#![allow(dead_code)]` 是唯一掩盖死代码的注释（见低严重度第 1 条）。

## 三、与旧报告 G（docs/agent-reports/G-io-psd.md，2026-09-24）的矛盾点

共 5 条实质矛盾（G 的行号引用本身也普遍过时 20-200 行，不逐一计）：

1. **【G 最重论断已失效】"app 完全没有调用 PsdReader，用户实际无法导入 PSD"（G 第 7、34、157 行）** — PSD 导入已完整接线：`crates/app/src/sheets_ui.rs:1482-1514`（`.psd` 分发、`std::fs::read` 后调 `import_photoshop_data`）；`crates/core/src/session.rs:3439-3490`（`read_with_budget` + `PsdDocumentBuilder::assets`/`make_import_with_assets`、无文档时新建画布/有文档时包 wrapping group、undo 名 "Import Photoshop File"、返回转换报告条数）。G 优先级建议第 1 条已完成。README 的 PSD 导入声明如今成立。
2. **【已修复】图层名 Pascal 串 `from_utf8_lossy` → U+FFFD（G 第 30 行差异 1）** — 现 reader.rs:472-501 是完整 Mac OS Roman 128 项映射表，reader.rs:185 实际调用；reader.rs:766-785 有专门测试锁定（含 0x80→Ä、0x8E→é、全 256 字节无 U+FFFD 断言）。
3. **【已修复】闭合路径收口角无 joint、按开放折线 butt 对接（G 第 52 行）** — 现 vector.rs:162-174 对闭合子路径显式传 `closed`，`stroke_outline`（vector.rs:624-684）在收口顶点计算 miter/bevel joint；`crates/core/src/psd/mod.rs:995-1018` 与 vector.rs:1021-1073 有测试锁定收口 miter 点与 miter-limit bevel。
4. **【描述过时】"8 样本垂直 box AA 的非零环绕填充"（G 第 51 行）** — 现实现是解析 signed-area 扫描线积分器（vector.rs:750-903；vector.rs:730-749 注释声明为对 CG 的有意替代，并引用 rust-vs-swift-diff-report §9.2-3），半覆盖像素输出精确 0.5 而非 16 档网格估计（mod.rs:1076-1153 测试锁定）。
5. **【已失效】"JPEG 导出整体缺失"（G 第 79、136-138 行）** — `compositor_core::export::jpeg_data_with_dpi` 已存在并被 app 使用（crates/app/src/sheets_ui.rs:1409-1425，携带文档分辨率作为 DPI；export 模块本身不在本次审计范围，仅推翻 G 论断）。附带：G 第 70 行 "导入尺寸预算缺失" 亦已部分失效——导出侧有 `check_export_size`（sheets_ui.rs:1403-1408），导入侧 PSD 有完整预算，普通图片导入侧仍未核（不在本次范围）。

G 中经核仍准确的论断：hue2/curv/levl 解析逐字节对齐（本次与 Swift 源核对属实）；`matches(url:)`/`read(from url:)` 文件入口仍无独立对应（Rust 仍 bytes-only，app 层用 `std::fs::read` 等价覆盖，G:31-33 的"缺失"表述已名存实亡）；composite 重绘→resample 简化（fixture.rs:333-340）属实；蒙版失败分支不可达（document_builder.rs:190-198）属实。

## 四、总结论

- **PSD 读取**是全部 12 个文件里质量最高的部分：段落覆盖、偏移/pad 计算、RLE 边界、预算门控、错误语义（全部走 `Result<PsdError>`，无 unwrap 于用户数据——唯一例外是发现 1 的 `unit_field`）都与 Swift 逐行对齐且有 34 个测试锁定；两处历史缺陷（MacRoman 层名、收口角 joint）在旧报告后被修复。**仅 1 个高严重度问题**（vector.rs:326 的越界 panic，一行边界检查即可修复）。
- **PSD 写出**：应用不写 PSD 是与 Swift 一致的设计事实；唯一写出器 fixture 是测试支持代码，实现正确但带两处脆弱切片（fixture.rs:347）。
- **项目存储**是"真实实现而非简化"：validate 的版本门控矩阵、层级/链校验、路径逃逸防护、整包替换式垃圾回收、staging+回滚事务都有测试背书。短板集中在**崩溃语义**：注释声称的 "never a missing destination" 不成立、无 fsync、`.staging-*`/`.backup-*` 残留无回收——普通失败不会损坏旧包（测试锁定），但进程被杀/断电场景的恢复完全依赖用户手工干预。
- **死代码总量很小**：2 个零调用的 pub 方法 + 1 个重复常量 + 1 个永不写出的 JSON 字段对，且被模块级 `#![allow(dead_code)]`（psd/mod.rs:6）有意识地掩盖。
- 性能面最大的坑是 `fill_polygons` 的全图 coverage 分配（vector.rs:786）与 save 的全内存三重克隆（store.rs:455-493 + convert.rs:134-138），均在 100MP 预算封顶内，属于"大文档时难看"而非失控。
