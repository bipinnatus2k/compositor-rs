# ADR 0004：描边实时执行的瓦片模型——live-stroke 性能对齐原版

- 状态：Accepted（2026-10-25 grill-with-docs 两轮规划裁定）；**未落地**——
  票 49–54（`.scratch/parity-review-tickets`，来源四），落地后回填台账。
- 范围：`compositor-core`（`tools/brush.rs`、`render.rs`、`raster.rs`、
  RasterEdit seam）+ `crates/app`（`state.rs` 帧缓存/revision、
  `canvas/editor_canvas.rs` 上屏）。
- 关联：执行 ADR 0003 Future work 的「帧缓存 `(tab_id, revision)` 进一步
  作用域化」项；术语裁定见 CONTEXT.md（瓦片 / 覆盖率 / 补丁 / 脏矩形 /
  settled-tail，2026-10-25 新增）。

## 背景（2026-10-25 性能排查结论）

用户报告画笔等绘制工具普遍延时卡顿。三路子代理排查 + 人工核验确认：描边
每个 mouse-move 在 UI 线程**同步执行全图级流水线**，且无事件合并、无后台
线程覆盖此路径：

1. `effective_coverage()` 每 move 新分配两个全网格缓冲并**从零重积分全部
   segments**（`brush.rs:702-736`、`render_continuous_tile` 自 `:1288` 起），
   成本随手描长度线性增长、整条描边累计平方级；`result()` 再加一次
   `base.clone()` 全网格深拷贝 + 全图逐像素循环（`brush.rs:739-751`）。
2. 每 move 全文档重合成（`state.rs:1122-1128` → `render.rs:249`）：每个
   可见图层无条件 `Bitmap` 深拷贝（`render.rs:448`）、
   `clear_effects_cache()` 每帧清空（`render.rs:274`）、蒙版命中缓存也整图
   memcpy（`render.rs:115`、`171`）、降采样只有档位算法无缓存全量重算。
3. `frame_image` 缓存键 `(tab_id, revision)`（`state.rs:1201-1213`），而每
   move `touch()` 都 bump revision → 每 move 整图 `data.clone()` + 逐像素
   BGRA 交换 + 新建 `RenderImage` 整图上传（`state.rs:1788-1796`，12MP 文档
   ≈48MB/帧）；且替换旧纹理**不调 `drop_image`**，D3D11 图集 tile 只增不减
   （gpui-pre 0.3.7 无自动回收；`window.rs:4988-5000`）。
4. 每 move 全图缩略图重采样（`set_preview_layer`，`state.rs:1058-1065`）。
   ——**票 49 量测修正（2026-10-25）**：该成本走 Shape 拖拽路径；画笔描边期间
   图层行缩略图已被票 37 的内容键冻结（每帧计数 0），票 50 的缩略图项相应
   缩减为 drop_image 一项。

原版对照（事实核查）：`BrushStroke` 256px 瓦片 + `dirtyTiles` 只碰 touched
（BrushStroke.swift:89-91、144、146）、`stamp`/`gridTip` 预烘焙、
`MetalBrushCoverage` 的 permanent 缓冲跨 move 持久增量积分、
`EditorCanvas` 按 `dirtyDocumentRect` 局部重绘（:506-517）、
`TiledLayerRenderer.drawStroke` 补丁悬浮于已提交像素（`TiledPieceCache` 只
缓存已提交 raster）、图层缩略图仅在 mouse-up 提交时更新
（EditorSession+Brush.swift:119、128-150）、`DownsampleCache` 身份键 LRU。

**关键事实**：Rust 连续引擎的数学逐项镜像 Metal kernel（Gauss-Legendre
节点/权重同字面值、hard/soft 分支、`tip_density`/`brush_coverage`/
`segment_distance_squared` 全对照），且 `culled_kernel_is_bit_identical_to_
the_reference`（`brush.rs:2403`）等测试用 `assert_bit_identical` 把重构
**约束到逐位复现**。因此瓦片化只能动存储与调度，不能动积分数学。

## 裁定（对应 grill 两轮 Q1–Q14）

1. **整链规划、六票分阶段**（Q1）：任何单点修复会被其余环节的全图级成本
   抵消；每票独立提交、独立可回退、各自可感知改善。
2. **执行模型**（Q2）：保留连续积分语义，存储重构为原版 GPU 模型的 CPU
   版——256px 瓦片、permanent 跨 move 持久、每 move 只积分新 settled 段 +
   当前尾段、`tailBackup` 按瓦备份、`dirtyTiles` 只重作 touched。否决换用
   原版 CPU 回退的 dab/stamp 算法（打破逐位锚定、改变描边外观，违 1:1）。
3. **上屏目标结构**（Q3）：已提交合成持久复用同一 `RenderImage`（同
   ImageId 图集命中零上传）+ 脏区补丁小纹理叠画；「持久合成缓冲 + 脏矩形
   增量合成」为第一阶段中间形态。gpui 事实：无子矩形纹理更新 API、无应用
   层局部重绘；同层多次 `paint_image` 叠画是常规用法。
4. **缩略图时机**（Q4）：改 mouse-up 提交时更新，描边期间面板冻结在笔触前
   状态——对齐 `commitPaintSnapshot` 语义。
5. **DownsampleCache 纳入本期**（Q5）：原 diff report 前置条件「待量测后
   立票」以票序满足——量测=票 49 先行、实现=票 54 殿后。
6. **范围**（Q6）：共享链（合成/纹理/缓存）+ Stroke 路径（画笔/橡皮/修复/
   仿制/模糊）；Warp（涂抹/液化）与 Shape 拖拽吃共享链收益，专属热点复测
   后另行记票。
7. **验收**（Q7）：门禁全绿、既有断言零改动（新测试只增不改）、逐位复现，
   补移植 Swift XCTest `largeBlankCanvasOnlyAllocatesTouchedTilesUntilCommit`
   （`brush_session_tests.rs:16-21` 头注记录的缺失项）；票 49 基线数字入本
   ADR 台账，目标（基线后允许一次性校准）：1200×900 + 100px 软笔刷 +
   125Hz 事件序列描边帧 ≤16ms、4000×3000 同场景 ≤50ms。
8. **记录载体**（Q8）：本 ADR + diff report「性能规划」节 + CONTEXT.md
   术语 + 票 49–54。
9. **revision 拆分**（Q10）：文档 revision（结构/已提交像素变化，缓存失效
   键）与 live epoch（描边每 move，只驱动重绘不失效文档级缓存）分立，镜像
   原版 `brushRevision`（EditorCanvas.swift:506-517 的 DisplayState 对比）。
10. **合成缓存切分**（Q11）：活动层 below/above **双段**持久缓冲，键挂文档
    revision——描边层上方图层必须盖在描边之上，单段缓存不成立；对齐
    `TiledPieceCache` 只缓存已提交 raster 的 piece 语义。
11. **补丁形状**（Q12）：单张覆盖触及瓦片的 bbox 补丁纹理一次叠画，非逐
    瓦片多补丁——内容同源、仅打包粒度不同，偏差记 diff report。
12. **DownsampleCache 键**（Q13）：`Arc<Bitmap>` 指针身份（对应
    `ObjectIdentifier`，新像素=新键=自然失效）；文档/渲染路径像素传递随之
    Arc 化，顺带消除 `render.rs:448` 的每层整图深拷贝。
13. **图集回收**（Q1-bis）：任何纹理替换点必须显式 `drop_image`。
14. **notify 粒度不动**（Q14）：见非目标。

## 票序（来源四；全串行——state.rs/editor_canvas.rs/brush.rs/render.rs 文件独占）

| 票 | 内容 | 阻塞于 |
|---|---|---|
| 49 | 插桩与基线（invalidation_counter 开 debug 读出 + 双场景基线） | — |
| 50 | 纹理 drop_image 回收 + 缩略图 mouse-up 化 | 49 |
| 51 | revision 拆分 + below/above 双段缓存 + 脏矩形重合成 + RasterEdit dirty API + 像素 Arc 化 | 49 |
| 52 | 描边补丁叠画上屏（消灭每 move 整图纹理重建上传） | 51 |
| 53 | 连续引擎瓦片化（逐位复现 + largeBlankCanvas 移植） | 51 |
| 54 | DownsampleCache 身份键 LRU | 49、51 |

## 否决项

- **换 dab/stamp CPU 回退算法**：改变描边像素外观、当场打破逐位锚定测试，
  违 1:1 纪律（见裁定 2）。
- **「优化版整图重建」**（如原地 BGRA 转换避免 clone）：保留整图上传与图集
  churn，不达目标；gpui 下唯一免上传手段是内容不变即复用同一 ImageId。
- **单段合成缓存**：上方图层每帧全画，带调整/效果层时重新引入卡顿，与
  piece 语义不符（见裁定 10）。
- **DownsampleCache 用 `(layer_id, revision)` 键**：失效粒度错位（描边/预览
  期间逐出无关层）、与原版身份语义偏差（见裁定 12）。
- **画布作用域 notify 机制**：gpui-pre 0.3.7 无局部重绘，视图树构建非热点，
  不预付机制（见非目标）。

## 非目标

- **整窗 notify 粒度 / 画布作用域失效**：gpui-pre 0.3.7 无应用层局部重绘
  （事实核查：renderer 每帧全 scene 绘制）。**gpui-kit 高版本可配 gpui-fast
  引擎、一定程度支持局部重绘——引擎升级时研究如何利用该特性**（用户裁定
  2026-10-25，见 Future work）。
- **Warp / Shape 专属优化**：共享链落地后复测，有剩余热点再立票。
- **effects 描边期间增量刷新**（原版 `LayerEffectsSurface` 只刷笔刷所到
  之处）：本期只修失效粒度（描边期间不清无关层缓存），增量刷新留票。
- 蚂蚁线相位动画等既有未立项项不因本期扩科。

## 后果

- 描边期间每 move 的 CPU 工作从 O(文档像素) 收敛到 O(脏区)，整图纹理上传
  从每帧一次收敛到提交时一次；所有工具经由共享链受益。
- core 公共 API 增补（票 51/53）：`RasterEdit::dirty_document_rect()`、
  `result_in(rect)`；像素类型在 seam 上以 `Arc<Bitmap>` 传递。按 AGENTS §7
  在提交描述列清单。
- 逐位约束兜底：`assert_bit_identical` 系列与 spacing/rotation 不变性测试
  零改动全绿是每票硬验收。
- diff report §2 表 Rendering 行与 DownsampleCache 条目随票 51–54 落地收口。

## 基线台账（票 49 回填，2026-10-25）

口径：`COMPOSITOR_STROKE_BASELINE=1 cargo test -p compositor stroke_baseline
-- --nocapture`（Windows 本机；workspace test profile 为 `optimized +
debuginfo`，非纯 debug）；每帧 = 生产 move 分支（append + invalidate）+ 事件
后自动 flush 的整窗渲染（视图树 + 画布合成 + 帧重建，无 GPU present）；
文档单全尺寸图层、Brush 100px/硬度 0.5、每 move 沿水平线步进；计数含建档
帧（thumbnail_regen=1 为初始行缩略图，非每帧成本）；描边 begin 静默
（`set_drag` 不 notify，首个 dab 搭第一次 move 的帧）。

| 场景 | 基线（2026-10-25） | 票 50 后 | 票 51 后 | 票 52 后 | 票 53 后 |
|---|---|---|---|---|---|
| 1200×900 / 100px 软笔刷 / 41 帧 | counts [40, 42, 41, 1, 0]；337ms 均值 / 440ms 峰值 | 208ms 均值（-38%）/ 452ms 峰值（首帧含建档噪声，均值为准） | | | |
| 4000×3000 / 同上 / 13 帧 | counts [12, 14, 13, 1, 0]；2714ms 均值 / 3029ms 峰值 | 1120ms 均值（-59%）/ 1709ms 峰值 | | | |

（counts 为 (root_notify, composite, frame_rebuild, thumbnail_regen,
mask)；数字取 invalidation_counter 计数 + 帧耗时 min/mean/max，口径由票 49
固定；目标值（≤16ms / ≤50ms）按此 optimized+debuginfo 口径维持，基线后不再
校准——两个数量级的差距先由票 50–53 收敛。）

## Future work

- **纹理回收的两个低频替换点**（票 50 复查披露）：`refresh_jpeg_preview`
  与图层面板缩略图缓存的纹理替换尚未接 `drop_image`——低频小额，随票 52
  的纹理生命周期重构一并收口。
- **gpui-fast 局部重绘研究**（用户裁定）：gpui-kit 升级到可配 gpui-fast
  引擎的版本后，评估画布作用域失效/局部重绘如何接入本模型（补丁叠画与
  局部重绘正交，可叠加）。
- effects 增量刷新、Warp/Shape 复测票（非目标节两项的后续）。
- 蚂蚁线相位动画（既有未立项项）。
