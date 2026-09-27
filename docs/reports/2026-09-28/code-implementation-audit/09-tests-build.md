# 09 · 构建与测试审计（dev-ui-fix @ d945a6d + 未提交改动）

> 审计日期：2026-09-28。范围：`cargo check / build / test` 实际运行结果 + `crates/core/tests/` 全部 10 个集成测试文件（4224 行）质量审查。
> 一切结论来自实际命令输出与读到的代码；未修改任何源码。

---

## ① 编译状态

### 1.1 `cargo check --workspace --all-targets` —— **失败**

| crate | 结果 |
|---|---|
| `compositor-core` | 编译通过，27 warnings（15 重复） |
| `compositor`（crates/app） | **编译失败：507 个 error**（check --all-targets 口径；`cargo build` 下 lib 为 71 个 error） |

错误全部集中在 app crate 自身，error code 分布：**E0609 ×505**（`no field 'session' on type '&mut AppState'` 468 个 / `&AppState` 37 个）、E0599 ×3（`present_pending`、`current_tab_mut` ×2）、另有 E0004 / E0063 / E0499 / E0502 / E0503 / E0596 若干。典型位置：`crates/app/src/workspace.rs:22`、`crates/app/src/canvas.rs:113/119/123/132`。

**根因（已核实）**：工作区存在**未提交改动** `crates/app/src/state.rs`（git diff +81/-25）：`AppState.session: EditorSession` 被替换为 `workspace: crate::tabs::ProjectWorkspace`，并新增 `tab()` 辅助（注释自述 "the rewrite of every former `self.session` call site"）——但 app crate 其余 500+ 处 `self.session` 调用点**尚未迁移**。HEAD（d945a6d）上 `session` 字段仍在，即破坏来自本次进行到一半的多标签重构，而非已提交代码。

**gpui 在 Windows 上的构建**：依赖链（含 gpui）编译正常——build 已推进到 app 自身源码的类型错误阶段，无任何 gpui 相关错误。一旦 state.rs 迁移完成，app crate 应可构建。

### 1.2 core 的 warning 分类（前 10 类）

| 数量 | 类型 | 代表位置 |
|---|---|---|
| 10 | variable does not need to be mutable | render.rs:1548 等 |
| 9 | unused import | compositor_core 多处 |
| 7 | unused variable | brush.rs:1737（`base`）等 |
| 3 | function is never used（死测试辅助） | document.rs:943 `palette_hex_roundtrip`、brush.rs:1232 `white_bitmap` |
| 3 | **cannot test inner items** | **render.rs:1379 / 1397 / 1426**（见 ④-1，严重） |
| 1 | **comparison is useless due to type limits** | **tests/brush_session_tests.rs:317**（见 ④-3） |
| 1 | unused doc comment | — |

### 1.3 `cargo build --workspace` —— **失败**

`error: could not compile 'compositor' (lib) due to 71 previous errors; 6 warnings emitted`。core 侧正常。

---

## ② 测试运行结果

`cargo test -p compositoor-core`（workspace 整体跑不了，app 编译失败；core 单独全绿）：

| 测试目标 | 数量 | 通过 | 失败 | 忽略 | 耗时 |
|---|---|---|---|---|---|
| lib 单元测试（`src/` 内 `#[cfg(test)]`） | 586 | 586 | 0 | 0 | 17.97s |
| tests/adjustment_layer_tests.rs | 9 | 9 | 0 | 0 | 0.01s |
| tests/brush_session_tests.rs | 15 | 15 | 0 | 0 | 0.18s |
| tests/clipboard_group_tests.rs | 15 | 15 | 0 | 0 | 0.02s |
| tests/downsample_tests.rs | 4 | 4 | 0 | 0 | 0.32s |
| tests/export_tests.rs | 5 | 5 | 0 | 0 | 0.01s |
| tests/glow_tests.rs | 16 | 16 | 0 | 0 | 0.06s |
| tests/selection_edit_tests.rs | 14 | 14 | 0 | 0 | 11.24s |
| tests/tool_flow_tests.rs | 14 | 14 | 0 | 0 | 0.15s |
| tests/transform_session_tests.rs | 9 | 9 | 0 | 0 | 0.00s |
| tests/type_tool_tests.rs | 3 | 3 | 0 | 0 | 0.10s |
| doc-tests | 0 | 0 | 0 | 0 | — |
| **合计** | **690** | **690** | **0** | **0** | — |

- **失败测试：无。被 `#[ignore]` 跳过：无**（`grep -rn "#\[ignore" src tests` 零命中，运行结果 0 ignored 一致）。
- **但注意**：源代码里存在 **3 个永远不会执行的测试**（render.rs 嵌套事故，见 ④-1）。`src/` 共 grep 到 589 个 `#[test]`，实际收集 586 个，差值恰为这 3 个。
- 最慢的集成文件是 selection_edit_tests（11.24s，含 4000×3000 大图用例 `invert_handles_large_images_and_uniform_masks_with_a_selection`）。

---

## ③ 单元测试分布（`grep -c '#\[test\]'`，分文件）

| 文件 | 数量 | 文件 | 数量 |
|---|---|---|---|
| session.rs | 130 | effects.rs | 8 |
| adjust/camera_raw.rs | 33 | tools/wand.rs | 7 |
| selection.rs | 27 | tools/object_selection.rs | 7 |
| tools/brush.rs | 26 | export.rs | 7 |
| render.rs | 25（**3 个死测试**） | effects_session.rs | 7 |
| project/store.rs | 22 | tools/text.rs | 6 |
| blend.rs | 20 | tools/mask_tracing.rs | 6 |
| raster.rs | 19 | tools/heal.rs | 6 |
| psd/mod.rs | 19 | tools/distort.rs | 6 |
| adjust/hue_saturation.rs | 18 | project/workspace.rs | 6 |
| color_picker.rs | 16 | tools/warp.rs | 5 |
| adjust/image_adjustments.rs | 15 | psd/reader.rs | 5 |
| canvas.rs | 14 | geometry.rs | 5 |
| adjust/pixel.rs | 14 | adjust/curves.rs | 5 |
| document.rs | 13 | project/manifest.rs | 4 |
| adjust/levels_auto.rs | 12 | tools/content_fill.rs | 3 |
| history.rs | 11 | raw.rs | 3 |
| transform.rs | 9 | psd/channel_coder.rs | 3 |
| adjustment.rs | 9 | project/convert.rs | 3 |
| adjust/levels.rs | 9 | project/mod.rs | 2 |
| psd/vector.rs | 8 | adjust/mod.rs | 2 |

**src 合计 589（其中 3 个从不执行）**；integration 合计 104。重心明显在 session.rs（130）与 adjust 系（camera_raw 33 + hue_saturation 18 + image_adjustments 15 + pixel 14 + levels_auto 12 + levels 9 + curves 5 = 106）。

---

## ④ 测试质量发现列表

### 高

- **[高] render.rs 三个测试被嵌套进另一测试函数体内，从未执行（假绿）** — `crates/core/src/render.rs:1377-1441`。`#[test] fn shape_rectangle_fills_with_color() {`（1377-1378）打开函数体后，1379-1380 又出现 `#[test] fn effects_cache_hits_for_identical_input()`，1397 `mask_clip_limit_caps_the_longest_side`、1426 `effects_guard_degrades_huge_results` 全部嵌在它体内，shape 测试本体在 1443 才恢复。三者为 **effects cache、`mask_clip_image_limited`、100 兆像素 effects 降级 guard 的唯一测试**，编译器已三次报 "cannot test inner items"；`cargo test --lib effects_cache_hits` → `running 0 tests`。典型的合并/粘贴事故，且当前"586 全绿"掩盖了它。

- **[高] app crate 507 个编译错误 = 该层零测试保护** — `crates/app/src/state.rs`（未提交 diff，`- pub session: EditorSession` → `+ pub workspace: ProjectWorkspace`，另加 `frame_image_tab` / `fit_tab` 缓存键改动）。`AppState.session → workspace.current_tab_mut()` 迁移只做了数据结构一半，500+ 调用点（workspace.rs:22、canvas.rs:113 等）未改。多标签帧缓存（`(tab_id, revision)` 缓存键、`fit_tab` 视口重适配）这类极易回归的逻辑，在编译通过前没有任何测试能覆盖——app crate 也没有任何测试目标。

### 中

- **[中] u8 恒真断言** — `crates/core/tests/brush_session_tests.rs:317`。`assert!(many <= 255);` 中 `many: u8`，比较恒真，编译器直接警告 "comparison is useless due to type limits"。该测试（`soft_stroke_builds_coverage_while_keeping_its_feathered_rim`）的有效断言只有 316 行的 `many > one + 60`；317 行是装饰。

- **[中] 用 ±1 容差迁就已知 off-by-one 缺陷** — `crates/core/tests/tool_flow_tests.rs:118-121`。注释自认 "Swift lands on exactly 50 × 20; the Rust crop rounds to 49 — a whole-pixel off-by-one in `warp_trimmed`'s bounds, recorded as a gap"，然后断言写成 `assert!((transform.size.width - 50.0).abs() <= 1.0, ...)`。缺陷被注释记录后用容差合法化，`warp_trimmed` 的边界取整 bug 将永远不会再被这个测试抓到。

- **[中] 期望值直接抄当前实现（与 Swift 基准背离的魔法数）** — `crates/core/tests/adjustment_layer_tests.rs:253-257`。`assert_eq!(p[7], 192); assert_eq!(p[11], 60);`，注释承认 Swift 语义应为 alpha 128/32，Rust 渲染器把调整层"叠"在底上得到 192/60，测试把 192/60 写死为期望。同类：`crates/core/tests/selection_edit_tests.rs:236-246`（把"清除无抗锯齿、对角线上每像素非 0 即 255"的二值裁剪断言为规格，注释承认 Swift 保留部分 alpha）；`crates/core/tests/transform_session_tests.rs:63-66`（断言 `undo_count == count + 2`，承认 Swift 折叠为一步）。这些断言透明度高（都有注释 + 模块头说明），属于"记录缺口"而非隐瞒，但数值本身是从实现行为抄回的——未来修正渲染器时这三处会同时"变红"，需要人肉判断是回归还是缺口的修复。

- **[中] 陈旧注释制造的覆盖空白：pixel-move API 其实已存在** — `crates/core/tests/selection_edit_tests.rs:9-11` 声称 "The pixel-move flow (`beginPixelMove`/`movePixels`/`finishPixelMove`) has no session entry yet, so the three Cmd-drag cases are not ported"。但 `session.rs:2127/2156/2194/2228` 已实现 `begin_pixel_move` / `drag_pixel_move` / `end_pixel_move` / `cancel_pixel_move`。理由已失效，三个 Cmd-drag 用例仍未补——属于"测试欠账被过期注释掩盖"。

### 低

- **[低] 死代码循环（伪操作）** — `crates/core/tests/brush_session_tests.rs:126-129`。循环取 `value` 后 `let _ = value;`，第一段遍历不产生任何断言（真正的 preview-vs-commit 比较在 143-149 行）。

- **[低] 被放弃的断言残骸（`let _ = ...` 家族）** — `crates/core/tests/selection_edit_tests.rs:120`（`let _ = (before, count);`，撤销计数"无记录"子断言被丢弃但夹具还留着）；`crates/core/tests/tool_flow_tests.rs:405`（`let _ = count;`）、`:492`（`let _ = layer;`）；`crates/core/tests/adjustment_layer_tests.rs:100`（`let _ = base_id;`）、`:462`（`let _ = NavigationTool::Move;` 仅为消化 import）；`crates/core/tests/transform_session_tests.rs:254`（`let _ = &mut session;`）。

- **[低] 存在性搜索断言（弱定位）** — `crates/core/tests/glow_tests.rs:224-238`（`found_glow`：整图任意一像素满足 alpha>0.5 && red>0.3 即通过）、`:274-292`（`inner_glow_preserved_in_export` 在一条 8 像素窄带上做同样的存在性搜索）。不固定辉光出现的位置/衰减形状，字体或布局变化时可能空过。可接受（字形依赖字体栅格化），但强度低于同文件其它像素坐标断言。

- **[低] 死测试辅助函数** — `crates/core/src/document.rs:943`（`palette_hex_roundtrip` never used）、`crates/core/src/tools/brush.rs:1232`（`white_bitmap` never used），均为 dead_code 警告在案的 cfg(test) 辅助。

### 正面评价（每个文件一句话）

10 个文件整体质量**高**：绝大多数用例断言真实像素值/坐标/撤销计数/层级结构（如 clipboard_group_tests.rs:119-128 的逐像素 + undo 验证、transform_session_tests.rs:207-213 的 30 次 preview 幂等性、export_tests.rs:200-212 的 JPEG 质量字节差），不是 smoke test；每个文件头部与用例注释都精确记录了与 Swift 原版的偏差（"recorded as a gap"），可追溯性好。唯一整文件偏弱的是 type_tool_tests.rs（113 行 / 3 用例，`invalid_draft_styles_are_rejected` 只测 `is_valid()` 纯函数，session 半数用例自述未移植）。

---

## ⑤ 覆盖空白

1. **PSD 写出：不是没测，是没有这条路径。** `grep -rn "psd" tests/` 零命中——10 个集成文件没有一个触碰 PSD。`psd/mod.rs` 的 19 个单元测试覆盖 reader + `PsdDocumentBuilder` 的导入 round-trip，但 `PsdDocumentBuilder` 只有 `assets` / `make_import` / `make_import_with_assets`（`crates/core/src/psd/document_builder.rs:28-46`），**没有任何把文档编码成 .psd 的导出 API**；session.rs 中 PSD 仅出现于导入（session.rs:3440-3443 `PsdReader::read_with_budget`）。PSD 导出是功能缺口 + 测试缺口双重缺失。
2. **camera_raw 及 adjust 子模块无集成测试。** camera_raw 有 33 个单元测试但 `compositor_core::adjust::camera_raw` 在 tests/ 零引用；levels/curves/pixel/levels_auto 同样只有单元级。adjust 系只有 `image_adjustments` / `hue_saturation` 被 adjustment_layer_tests 间接带到。
3. **画笔 session 流不存在。** `begin_brush` / `continue_brush` / `finish_brush` / `cancel_brush` 在 session.rs 中 grep 无命中（brush_session_tests.rs 模块头也自述未移植）；所有笔刷用例直驱 `RasterEdit`，撤销折叠、取消恢复文档、stroke 状态机这些 Swift 用例的核心行为无任何测试。
4. **effects 集成面窄。** 16 个 glow 集成用例之外，shadow/stroke 只有 `glow_tests.rs:611-672` 一个组合用例；`ColorOverlayEffect` 的唯一测试恰是 ④-1 中那条死测试（render.rs:1380）。
5. **app crate 零可运行测试**（编译不过），gpui UI 层、`tabs::ProjectWorkspace`、`project::TabId` 帧缓存均无保护。
6. **doc-tests 为 0**（8580 行的 session.rs 无任何文档示例测试）。

---

## ⑥ 总结论

1. **"690 全绿"是真的，但只对 core 成立，且打了三个折扣**：workspace 编译红灯（app 507 错误，来自未提交的半成品 state.rs 重构）；render.rs 1377-1441 有 3 个测试因嵌套事故从未执行（其中恰包括 effects cache 与 100MP guard 的唯一测试）；app 层零测试。
2. **core 测试本身质量高**：像素级/坐标级/撤销计数断言为主，Swift 基准逐用例对照，偏差全部有注释记录——这使"测试抄实现"的嫌疑点（adjustment_layer_tests.rs:256-257、tool_flow_tests.rs:120、selection_edit_tests.rs:238-246、transform_session_tests.rs:66）都是**显式的**，属于技术债而非造假；但它们意味着这些行为一旦"修复对齐 Swift"，测试需人工同步改期望。
3. **优先修复顺序建议**：先恢复 app crate 编译（完成 session→workspace 迁移或回退 state.rs），再把 render.rs 1377-1441 的嵌套三测试拆出为独立 `#[test]`，然后补 pixel-move（API 已在）与 PSD 导出决策（做或不做，删注释或补实现）。
