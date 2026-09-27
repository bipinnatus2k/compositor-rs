# 全工程代码实现情况审计（2026-09-28）

由 9 个并行子代理对全工程（core 约 4.06 万行 + app 约 1.61 万行 + 测试 4,224 行）做逐行审查。
**方法约束：不采信任何注释、doc comment、既有文档（含 `docs/agent-reports/` A–H 旧报告与记忆），一切结论以实际代码为准，每条发现给出实测 `文件:行号`。**

> **重要环境说明**：审查进行期间（00:49–01:35）工作树正被另一进程做 `session → workspace 多标签`迁移
> （`git status`：13 个 app 文件修改 + 新增 `crates/app/src/flows.rs`，均未提交；HEAD = `d945a6d`）。
> 迁移中途 app crate 曾出现 507 个编译错误，审计收尾时已恢复为 0 错误（core 编译干净，19 个 warning）。
> 因此 app 层两份报告的行号锚点不同：`07` 锚定审查结束时的最终快照（文件 md5 记录于报告开头），
> `08` 锚定 HEAD `d945a6d`（迁移 hunk 已逐个核对）。core 层六份报告不受影响（工作树未动 core）。

---

## 一、总体结论

1. **核心算法是真实实现，不是假实现。** 混合模式 24 种公式正确（`blend.rs`，报告称"25 模式"属文档漂移）；Levels/Curves/Levels Auto 数学完整；Spot Heal 与 Smudge/Liquify 与原 C 内核逐常数一致；笔刷是连续沉积引擎的严肃 CPU 移植；Content-Aware Fill 核心是 PatchMatch 类真实算法；PSD 读取（Header/Records/RLE/分组/向量蒙版）完整并有 34 个测试锁定。
2. **真正的 stub 只有少数几处，但其中两处已接进菜单**：主体移除 `subject_removal.rs:7-10`（`image.clone()` + TODO，却挂在 `menu.rs:182` "Remove Background…"）；Content-Aware Fill 菜单命令接的是 no-op `run()`（`content_fill.rs:250-253`，真算法 `run_fill` 无生产调用）。此外 `panels.rs` 整个 652 行模块无生产调用；PSD 写出为有意设计的不支持；Object Selection 核心完整实现但全工程零生产调用（Vision 能力缺口，文档已诚实声明）。
3. **最大主题是"接线断裂"而非"功能缺失"**：tabs.rs 约 1100 行多标签模型、project.rs 约 2400 行确认弹窗/保存面板模型在 HEAD 均无 UI 入口；wand 的 Object 模式 UI 可切换但 core 从不读取；Camera Raw 去边吸管/自动白平衡/readout 均为死代码。
4. **测试 690 个通过、0 失败、0 忽略，但属于"假绿"隐患**：3 个测试因嵌套事故从未被收集运行（`render.rs:1377-1441`），且若干断言是从当前实现行为抄回的"实现快照"。
5. **2026-09-24 旧报告 A–H 已大面积过时**：合计约 100 条论断被本次审计证伪（绝大多数是旧报告所称"缺失"此后已实现），详见表四。

## 二、高严重度发现汇总（跨报告去重后 21 条）

| # | 域 | 问题 | 位置 |
|---|----|------|------|
| 1 | session | 删除图层绕过活蒙版 bake 询问，静默 unlink | `crates/core/src/session.rs:766-768, 3264-3318` |
| 2 | session | `delete_selected_layers_with_baked` 多选时删错对象，且 bake 写入在 undo 步之外 | `session.rs:3791-3810` |
| 3 | session | 待定渐变可提交到错误图层（`GradientDraft.layer_id` 被忽略，写 `active_layer_id`） | `session.rs:3987-4027, 152-157` |
| 4 | session | undo/redo 缺 Swift `canUseHistory` 守卫，变换/像素搬移中可 undo，commit 写回陈旧 draft | `session.rs:593-637` |
| 5 | canvas | Image Size 重采样：就地非均匀蒙版被双重变换，内容飞出位图 | `crates/core/src/canvas.rs:359-384` |
| 6 | canvas | u64 预算下溢：debug panic / release 预算失效后可分配 3.6GB 位图（接近高） | `canvas.rs:190-196` |
| 7 | render | 1×1 实心蒙版 + 任意启用效果 = 合成 panic（UI 可达：新建图层蒙版即 1×1） | `crates/core/src/render.rs:695-701, 442-444` |
| 8 | render | 3 个测试嵌套在函数体内从未运行（含 100MP guard 唯一测试） | `render.rs:1376-1457` |
| 9 | adjust | Hue/Saturation Colorize + 负色相输出错误颜色（UI 可达，修复为一行 `rem_euclid`） | `crates/core/src/adjust/hue_saturation.rs:422, 624-645` |
| 10 | psd | `unit_field` 对截断的 vogk/vstk 段越界读 8 字节 → 导入线程 panic，应用崩溃 | `crates/core/src/psd/vector.rs:326-330` + `types.rs:279-281` |
| 11 | tools | Subject Removal 是 stub 且已接入菜单，静默无效 | `subject_removal.rs:7-10` → `filters.rs:369` → `menu.rs:182` |
| 12 | tools | Content-Aware Fill 菜单命令是 no-op，且 state 侧无条件制造空撤销步 | `content_fill.rs:250-253` + `app/state.rs:779-821` |
| 13 | tools | 选区剪裁按笔刷网格坐标采样（文档坐标语义），缩放图层上剪裁错位 | `brush.rs:722-741` 等 vs `selection.rs:554-567` |
| 14 | tools | 笔刷预览每帧全画布 O(像素×段数) 重渲，段数无界 | `brush.rs:624-658, 1179-1226` |
| 15 | tools | Wand Object 模式是无效开关（core 零调用方） | `object_selection.rs:66-106` vs `session.rs:1606-1673` |
| 16 | tools | Heal 失败时把预览暗色 wash 永久写入图层 | `brush.rs:575-614` + `app/canvas.rs:729` |
| 17 | app | 多标签迁移中途编译破坏（审计结束时已修复，留档） | 当时 `state.rs:503`（`present_pending` 未定义）、`menu.rs:16` |
| 18 | app | 未保存确认流半成品：4 个 resolve 函数零调用者，确认框事件无人回传 | `app/tabs.rs:524/364/550/392` |
| 19 | app | tabs.rs 约 1100 行与 UI 零接线（除 close_window），保存/打开绕过 ProjectController | `app/tabs.rs` 全文件，`sheets_ui.rs:1452` |
| 20 | app | 模态表单不设键盘防：约 40 个 handler 不检查 sheet/输入焦点，Ctrl-Z 可穿透 Levels 预览改文档 | `app/workspace.rs:392/417/644/915` 等 |
| 21 | app | blend-mode 快捷键不 `revision += 1`，帧缓存按 revision 键控 → 画布停留旧图 | `app/workspace.rs:833-840` |

值得注意的中严重度（择要）：`history.rs:165-168` 蒙版内存估算按 1 B/px 而实际存 RGBA（低估 4 倍）；`project/workspace.rs:85` ≥64 层蒙版链 bake 失败时把 asset 静默覆盖为空；`project/store.rs:517-520` save 事务无 fsync、崩溃窗口内原路径无包、`.staging-*`/`.backup-*` 残留无 GC；Camera Raw 的 Upright Guided 只用第一根线（±25 固定值）、Lens Profile 无镜头档案（固定近似）、暗角 Paint Overlay 样式名实不符；Add Noise seed 硬编码 7（`app/state.rs:798/1565`）；motion blur 边界能量丢失不归一（`filters.rs:449-451`）。

## 三、死代码 / 死 UI 清单（择要）

- **core**：`object_selection.rs` 全模块（自标 `allow(dead_code)`）；`subject_removal.rs`；`clone_stamp.rs` 整文件 19 行占位；`content_fill::run_fill`；pixel.rs 的 `blend_gray`/`invert_mask`/GuidedMatte；调整域的 Scope 示波器整套、`GuidedMatte`、色相波段编辑方法；render.rs `mask_clip_image_limited` 的 limit 参数生产从不传值；session.rs 4 个全工程零调用 pub 方法 + 约 44 个仅内部使用的过度 pub。
- **app**：`panels.rs` 整模块；Camera Raw 去边吸管（`state.rs:322/461/1589`）、`camera_raw_readout` 双端、`apply_camera_raw_auto_white_balance`（`state.rs:1621`）；`request_refit`/`fit_tab`（同尺寸 tab 切换不 refit 的根源）；Window > Reset Panel Layout 只改状态栏文字（`menu.rs:210` → `workspace.rs:756-758`）；Move 头 Auto Select 勾选框持久化但无代码读取（`tools.rs:587-591`）；color_picker_ui 3 个构造后永不渲染的滑杆；Save As 有快捷键无菜单项（`shortcuts.rs:96` vs `menu.rs:73-84`）。

## 四、与 2026-09-24 旧报告（docs/agent-reports/ A–H）的核对结果

| 旧报告 | 本次矛盾 | 状态 |
|--------|----------|------|
| A 文档/会话/历史 | 36 条失效、7 条仍成立 | 旧"缺失清单"绝大多数已实现；**多选整体变换仍缺失**（group 管线铺好但源头不可达，`session.rs:4833-4862`） |
| B 变换/画布/项目 | 7 条失效 | fill/CanvasResizer/ImageResizer/CropSnap 均已补齐，但重写引入了发现 #5 的蒙版 bug |
| C 调整 | 13 条实质矛盾，4 条仍成立 | LevelsAuto/HueSat 立方体/is_valid 守卫均已落地；Camera Raw UI 仍仅 14 滑块 |
| D 选区 | 11 项中 9 项过时 | rasterize 已改解析面积 + winding（`selection.rs:580-809`），旧注释 `selection.rs:64` 成了实现错误的最后证物 |
| E 工具 | 22 项中 15 项过时 | heal 3 处内核偏离**全部已修复**；ObjectSelection 从"缺失"变成"完整但零接线" |
| F 渲染/效果/滤镜 | 18 条矛盾（12 项差异中 10 项已修复） | 混合数学三项错误全修复；TiledLayerRenderer 瓦片机制仍缺失、Add Noise seed 硬编码仍成立 |
| G IO/PSD | 5 条矛盾 | 最重论断"用户无法导入 PSD"已失效（`sheets_ui.rs:1482-1514` 已接线）；Mac Roman 图层名、闭合路径收口均已修复 |
| H UI/应用 | 22 + 11 条矛盾 | "缺失"的透明度数字键/Space 手型/Auto-select/快捷键自定义系统/持久化全部已落地；Reset Panel Layout 空实现、无原生文件对话框仍成立 |

**结论：旧报告只能当作 2026-09-24 时点快照使用，任何结论引用前必须以本次审计或当前代码为准。**

## 五、测试与构建（2026-09-28 01:20 快照）

- 编译：workspace 0 错误（迁移中途曾 507 错误，已恢复）；core lib 19 warnings。
- 测试：core lib 586 + 集成 104 = **690 个，全部通过，0 忽略**。
- 但 3 个测试从未被收集：`render.rs:1379-1441`（嵌套事故），其中 `effects_guard_degrades_huge_results` 是 100MP 守卫的唯一测试。
- 质量隐患：`tool_flow_tests.rs:118-121` 用容差把已知 off-by-one 合法化；`adjustment_layer_tests.rs:253-257` 期望值从当前实现抄回（Swift 基准 128/32 vs 断言 192/60）；`selection_edit_tests.rs:9-11` 的"无 session 入口"理由已失效（`session.rs:2127-2228` 已实现 pixel move）；PSD 写出路径不存在、camera_raw 无集成测试。

## 六、报告索引

| 文件 | 范围 | 高 / 中 / 低 |
|------|------|--------------|
| [01-session.md](01-session.md) | session.rs（8,580 行状态机） | 4 / 8 / 11 |
| [02-document-model-history.md](02-document-model-history.md) | document/canvas/history/adjustment/color_picker/geometry | 1（+2 接近高） / — / — |
| [03-render-pixel-pipeline.md](03-render-pixel-pipeline.md) | render/blend/filters/effects/effects_session/raster/export/raw | 2 / 4 / 若干 |
| [04-adjustments.md](04-adjustments.md) | adjust/ 全部 + adjustment.rs 分发 | 1 / 6 / 14 |
| [05-psd-project-io.md](05-psd-project-io.md) | psd/ 全部 + project/ 全部 | 1 / 2（最重） / — |
| [06-tools-selection.md](06-tools-selection.md) | tools/ 全部 + selection.rs | 6 / — / — |
| [07-app-state-canvas.md](07-app-state-canvas.md) | app: state/canvas/tabs/tools/keys | 6 / 多条 / — |
| [08-app-ui-workspace.md](08-app-ui-workspace.md) | app: sheets*/workspace/project/shortcuts/settings/panels 等 | 3 / 多条 / — |
| [09-tests-build.md](09-tests-build.md) | 编译 + 测试运行 + 测试质量 | 2 / 3 / — |
