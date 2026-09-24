> 子代理原始报告（域 A：文档模型 / EditorSession / 历史 / 图层操作），逐字保留，2026-09-24。

# 文档模型 / EditorSession / 历史 / 图层操作 域对比报告

## 总体结论

README 的“1:1 移植”说法在本域**只对数据模型和历史内核成立，对操作层不成立**。`document.rs` / `history.rs` / `adjustment.rs` 确实是忠实移植（常量、undo 事务嵌套、条目上限 100 / 256MB、undo 名列表基本一致），但 `session.rs`（1951 行）只覆盖了 Swift `EditorSession` 及其 6 个扩展（约 2900 行）中一小部分操作：**图层面板的多选/分组/拖放、活蒙版（剪贴蒙版）的全部 session 操作、蒙版复制/链接/独立变换提交、浮动选区（Cmd-T）、调整图层创建与编辑、导入管线、多标签 ProjectWorkspace、参考线拖拽、颜色选择器、不透明度快捷键全部缺失**。另外有若干处语义偏差（删除后活动层选择、蒙版变换提交路径、文件夹透明度、克隆复制范围、历史字节估算）会直接造成行为不一致。

---

## 一、【缺失】Swift 有、Rust 完全没有

### EditorSession.swift（主体状态机）
1. **多选状态机 `selectLayers` / `extendSelection`** — `Compositor/Compositor/Document/LayerGroups.swift:96` / `:107`；Rust `crates/core/src/session.rs:362`（`select_layer`）只能单选，`selected_layer_ids` 从未被赋成多元素。
2. **多选一步删除 `deleteSelectedLayers`（undo 名 "Delete Layers"）** — `EditorSession.swift:647`；Rust 无对应（`session.rs:402` `delete_layer` 只删单个）。
3. **面板列表重排 `reorderLayers(from:to:)`** — `EditorSession.swift:687`；Rust 无（只有 `move_active_layer`，session.rs:457，与 Swift `:703` 对应）。
4. **眼睛滑动批量显隐 `beginVisibilitySwipe` / `setVisibilityInSwipe` / `endVisibilitySwipe`** — `EditorSession.swift:673`/`:680`/`:685`；Rust 无。
5. **导入管线 `importImages` / `drainImports` / `insert` / `insertPhotoshop`（undo 名 "Import Images"/"Import Image"/"Import Photoshop File"）** — `EditorSession.swift:720`/`:737`/`:792`/`:855`；Rust core 完全无导入入口（PSD 只有读取器）。
6. **群组变换 `transformsAsGroup` / `groupTransformMembers` / `groupTransformBox` 及 begin/commit 的 group 分支（undo 名 "Transform Layers"）** — `EditorSession.swift:301`/`:303`/`:319`/`:379-384`/`:434-448`；Rust 只有 `TransformGroup` 类型（`transform.rs:210`），`begin_transform`（session.rs:1156）和 `commit_transform`（session.rs:1127）均无 group 路径。
7. **Option 拖拽复制变换 `beginDuplicateTransform` / `transformDuplicate` 及其 commit/cancel 清理** — `EditorSession.swift:396`/`:203`/`:422-424`/`:463-470`；Rust 无。
8. **`commitTransform` 的 floating / corners / mask 三个分支** — `EditorSession.swift:426-433`；Rust `commit_transform`（session.rs:1127-1150）只有普通图层路径。
9. **操作守卫 `canStartProjectOperation` / `canUseHistory` / `canEditLayers` 及 busy 状态机（`isProjectBusy`/`showsBusy`/waiters）** — `EditorSession.swift:150`/`:563`/`:600`/`:118-169`；Rust 的 `undo/redo/全部编辑操作`（session.rs:324-360 等）没有任何守卫。
10. **工具族笔尖停靠 `parkedBrushTips` / `tipFamily`（selectTool 换工具时换笔尖）** — `EditorSession.swift:223`/`:224`/`:336-344`；Rust 无。
11. **`cycleToolMode` 的 Shape/Blur/SpotHealing/CloneStamp 分支** — `EditorSession.swift:354-373`；app 层部分实现（`crates/app/src/canvas.rs:789-816`）只处理 Marquee/Lasso/Wand/Brush/Gradient，缺 `toggleShapeKind`、blur 三模循环、spotHealing 三模循环、cloneStamp `sampleAllLayers` 切换。
12. **ToolDefaults 持久化（autoSelect/transformControls/pixelGrid/grid/guides/rulers/snap 系列 + locksGuides 的 UserDefaults 读写）** — `ToolDefaults.swift:9-22`，`EditorSession.swift:198-279`；Rust `session.rs:252-260` 只硬编码了 Swift 的 fallback 值，无持久化机制。
13. **文本/形状草稿状态 `textDraft` / `shapeDraft` / `finishText`（selectLayer/selectTool 的守卫依赖）** — `EditorSession.swift:233-241`；Rust `select_layer`（session.rs:362）无此守卫。

### LayerGroups.swift（除两个工具函数外整文件缺失）
14. **`LayerHierarchy.entries/visibleLayers/validate`** — `LayerGroups.swift:3-47`；Rust 只有加载期校验 `crates/core/src/project/store.rs:157`（`hierarchy_rules` 测试）和 `document.rs:648/678` 的 `effective_visible_ids`/`descendant_ids`，无面板行模型。
15. **`LayerOpacity.effective` / `effectiveOpacity` / `CanvasDocument.effectiveOpacities`（文件夹透明度折叠）** — `LayerGroups.swift:53-64`/`:68`/`:83`；Rust 全仓库无 `effective_opacity`，合成时只用自身透明度（`crates/core/src/render.rs:233`）。
16. **`groupSelectedLayers`（"Group Layers"）、`addGroup`（"New Folder"）、`toggleGroupExpansion`、`canPlaceLayer`、`placeLayer`（"Move Layer"）、`moveActiveLayerOutOfGroup`** — `LayerGroups.swift:119`/`:177`/`:195`/`:203`/`:210`/`:231`；Rust core 无。app 有一个自制 `add_group`（`crates/app/src/layers_panel.rs:351`），行为不同（见行为差异 13）。

### LayerMerge.swift
17. **`mergePlan` / `canMergeLayers` / `mergeTitle`** — `LayerMerge.swift:7`/`:29`/`:30`；Rust `merge_layers`（session.rs:1288）内联了计划但缺下面“行为差异 8”所列守卫，且无菜单可用性查询。

### LayerMask.swift
18. **从选区建蒙版 `addMask(revealing:)`（undo 名 "Add Mask from Selection"，同一步内清选区）** — `LayerMask.swift:233-260`；Rust `add_layer_mask`（session.rs:1072）只能建纯色蒙版。
19. **蒙版目标选择 `selectLayerTarget`** — `LayerMask.swift:222-228`；Rust 无。
20. **跨层复制蒙版 `canCopyMask` / `copyMask`（"Copy Layer Mask"/"Replace Layer Mask"）** — `LayerMask.swift:290`/`:298`；Rust 无。
21. **蒙版链接开关 `toggleMaskLink`（"Unlink/Link Layer Mask"）** — `LayerMask.swift:312`；Rust 的 `LayerMask.is_linked` 字段存在（document.rs:249）但没有任何 session 操作改它。
22. **`displayedMaskPlacement`（变换进行中蒙版位置的显示推导）** — `LayerMask.swift:324-338`；Rust 无。
23. **`commitMaskTransform`（"Transform Layer Mask" / "Distort Layer Mask"）** — `LayerMask.swift:341-365`；Rust `commit_transform` 没有 mask 分支（见行为差异 7）。
24. **蒙版扭曲预览 `maskDistortPreview` + `MaskDistortPreviewCache`** — `LayerMask.swift:369-394`；Rust 无。
25. **`LayerMask.isValid` / `asset(from:)`（灰度蒙版格式校验 + 缩略图生成）** — `LayerMask.swift:23`/`:35`；Rust 蒙版直接是 RGBA `Bitmap`（document.rs:242），无格式校验。

### LiveLayerMask.swift（session 级全部缺失）
26. **`canLinkMask` / `linkMask`（"Create Clipping Mask"）、`removeLiveMask`（"Release Clipping Mask"）、`canToggleClippingMask` / `toggleClippingMask`、`adoptClipping`、`releaseDetachedClipping`** — `LiveLayerMask.swift:23`/`:31`/`:53`/`:205`/`:214`/`:42`/`:225`；Rust 只有加载校验 `validate_live_masks`（store.rs:194）和渲染期 `multiply_by_source_alpha`（render.rs:276），**没有任何创建/释放剪贴蒙版的操作**。
27. **删除时活蒙版处理 `deleteWithLiveMaskChoice`（bake 询问）与 `finishDeletingLayer(s)`** — `LiveLayerMask.swift:104`/`:131`/`:149`；Rust `delete_layer`（session.rs:402）直接删，不询问、不 bake，也不给依赖层重挂资产。
28. **`LiveMaskBaker.bake`** — `LiveLayerMask.swift:74-97`；Rust 无。

### LayerAppearance.swift
29. **`previewBlendMode`（悬停预览入口）** — `LayerAppearance.swift:78`；Rust 有 `blend_preview` 字段（session.rs:136）和 `displayed_blend_mode`（session.rs:584），但没有预览设置/清除方法。
30. **`canEditOpacity`** — `LayerAppearance.swift:86`；Rust `set_layer_opacity`（session.rs:639）无此守卫。
31. **`setSelectedLayersOpacity`（多选一步设透明度）** — `LayerAppearance.swift:108-119`；Rust 无。

### LayerAdjustment.swift + AdjustmentEditing.swift
32. **`addAdjustment`（"New X Adjustment"，Gradient Map 取前景/背景色、Grain/AddNoise 随机种子）与 `updateAdjustment`** — `LayerAdjustment.swift:184`/`:206`；Rust 无任何创建调整图层的操作（渲染端 `render.rs:129-131` 会应用，但造不出来）。
33. **AdjustmentEditing 全文件**：`beginAdjustmentEditing` / `editedAdjustment` / `previewAdjustmentEditing` / `finishAdjustmentEditing`（"Edit X Adjustment"）— `AdjustmentEditing.swift:6`/`:67`/`:101`/`:110`；Rust 无对应（`adjustmentEditingID`/`adjustmentOriginal` 状态不存在）。

### ProjectWorkspace.swift + EditorSession+Projects.swift（两文件在 Rust 中无对应物）
34. **多标签工作区 `ProjectWorkspace`（canSwitch/addTab/select/loadProject/close/confirmQuit/receive/receiveProviders/copyLayer 跨项目复制，"Copy Layers from Project"）** — `ProjectWorkspace.swift:20`/`:27`/`:39`/`:47`/`:77`/`:123`/`:142`/`:157`；Rust app（workspace.rs）是单文档布局，core/project 只是 `.comp` 存储格式。
35. **`projectSnapshot` / `installProject` / `clearProject` / `createNewProject`** — `EditorSession+Projects.swift:4`/`:20`/`:42`/`:55`；Rust 无 session 级安装/清空/重置历史入口（store.rs 只做包的读写）。

### ColorPalette.swift
36. **`setPaletteColor`、`canEditPalette`、颜色选择器全套（`ColorPickerTarget`/`ColorPickerState`/`openColorPicker`/`closeColorPicker`/`typeColor`/`openTextColorPicker`/`openEffectColorPicker`/`openGradientMapColorPicker`/`sampleIntoColorPicker`）** — `ColorPalette.swift:30`/`:25`/`:162`/`:180`/`:60`/`:79`/`:66`/`:70`/`:75`/`:106`/`:130`；Rust 只有 `palette_color`/`swap`/`reset`（session.rs:1540/1563/1576）与 `render.rs:439` `sample_composite_color`（对应 Swift `:137`）。`PickerHSB`/`quantized`/`hex`/`init(hex:)` 已移植（document.rs:90/49/57/67）。

### FloatingSelection.swift（整文件无对应）
37. **选区浮动变换 `canTransformSelection` / `transformCommand` / `beginSelectionTransform` / `floatingSelectionTransform` / `mergeFloatingTransform` / `cancelFloatingTransform` / `FloatingMerge.merge`（"Transform Selection"）** — `FloatingSelection.swift:15`/`:22`/`:27`/`:59`/`:68`/`:110`/`:121`；Rust 只有类型骨架（`transform.rs:223` 的 `TransformEdit.floating`、`selection.rs:606` 的 `FloatingTransform`），没有任何 session 代码构造或消费它。

### Guides.swift
38. **参考线拖拽全套：`GuideDrag` / `displayedGuides` / `hitGuide` / `beginGuideCreation` / `beginGuideMove` / `moveGuideDrag` / `finishGuideDrag`（"Delete Guide"/"Move Guide"）/ `cancelGuideDrag`** — `Guides.swift:58`/`:79`/`:91`/`:110`/`:118`/`:124`/`:132`/`:160`；Rust 无。
39. **对齐吸附 `alignmentSnapTargets` / `snappedGuidePosition` / `canEditGuides`** — `Guides.swift:182`/`:222`/`:72`；Rust 无（`add_guide`/`clear_guides` 已移植：session.rs:1424/1409，但无 `locks_guides` 守卫）。

### EditorSession+Brush.swift（session 状态机部分）
40. **`canPaint`（含有效可见性、蒙版启用、调整层排除等 6 项守卫）与 `makeRasterEdit` 的 100MP 预算** — `EditorSession+Brush.swift:5`/`:13-27`；Rust 无；app 的 `begin_stroke`（canvas.rs:427）是简化重实现，无像素预算。
41. **`commitPaintSnapshot` / `commitRasterEdit` 的会话级提交（含蒙版随图层扩张、提交前内容校验）** — `EditorSession+Brush.swift:128`/`:154`；Rust core 无。
42. **`usesOpacityKeys` / `typeOpacityDigit`（数字键设透明度，双键组合）** — `EditorSession+Brush.swift:191`/`:195`；Rust 无。
43. **`changeBrushHardness` / `changeBrushSize`（Shift-[ ]、括号键步进）** — `EditorSession+Brush.swift:212`/`:219`；Rust 无。
44. **`SelectionClipboard.swift` 的 `duplicateLayer(in:above:)` / `addPixelLayer` / `nextLayerName`（复制粘贴/图层经复制）** — `SelectionClipboard.swift:174`/`:187`/`:203`；Rust 无剪切复制粘贴的任何 session 操作。

---

## 二、【行为差异】两边都有但语义不同

1. **历史字节估算不同** — Swift `DocumentHistory.swift:103`（`retainedBytes`）按 `image.bytesPerRow * height`（灰度蒙版 1 字节/像素）计；Rust `history.rs:186` 一律 `width * height * 4` → 蒙版多的文档更早触发字节上限裁剪。
2. **快照相等语义不同** — Swift `ImageLayer.==`（`EditorSession.swift:4-7`）对 `asset.image` 用 `===` 身份比较；Rust `ImageLayer`/`ImportedImage` 是 derive 的值相等（document.rs:501、raster.rs:207，含像素数据）→ `history.rs:123` 的“文档未变则不记历史”判定与 Swift `DocumentHistory.swift:64` 在“内容相同的不同图像对象”场景下结论相反。
3. **删除活动层后的选中层** — Swift `finishDeletingLayer`（`LiveLayerMask.swift:142-145`）把活动层移到邻层 `layers[min(index, count-1)]`；Rust `delete_layer`（session.rs:414-417）直接置 `None`。（LayerTests.swift:27 专门测此行为。）
4. **`select_layer` 不提交变换、无守卫** — Swift `selectLayer`（`EditorSession.swift:325-331`）切层时 `commitTransform()` + `resolveGradient()`，且被 brushStroke/levels 守卫；Rust `select_layer`（session.rs:362-368）直接改字段。
5. **undo/redo 的恢复逻辑** — Swift `undo()` 先取消渐变、`restore()`（`EditorSession.swift:570-591`）取消裁剪、按 `keepMaskTarget` 恢复 `isMaskSelected`、换画布时适配视口；Rust `undo/redo`（session.rs:332-350）只换 document/active_layer_id，**`is_mask_selected` 变陈旧**，也不取消进行中的裁剪。
6. **canUndo 条件** — Swift `canUndo = canUseHistory && (history.canUndo || gradientEdit != nil)`（`EditorSession.swift:567`）；Rust `can_undo`（session.rs:324）只有 history 条件。
7. **蒙版单独变换提交走错路径** — Swift 对 `edit.mask` 提交 `commitMaskTransform`（`EditorSession.swift:432` → `LayerMask.swift:341`，只改 `mask.placement`，undo 名 "Transform Layer Mask"）；Rust `commit_transform`（session.rs:1127-1150）对同样的 edit 会**把 draft 写进 `layer.transform`** 且 undo 名恒为 "Transform Layer"。
8. **merge 守卫不全** — Swift `mergePlan`（`LayerMerge.swift:8`/`:14`/`:20`）要求 `canEditLayers`、多选时至少含一个非 group 层、群组合并时内部有像素层；Rust `merge_layers`（session.rs:1288-1340）三个守卫都没有；也没有 `LayerHierarchy.validate` 失败回退（LayerMerge.swift:65）。
9. **`duplicate_active_layer` 只复制单层** — Swift `duplicateActiveLayer`（`SelectionClipboard.swift:147-170`）连 descendants 一起复制、重映射 parent/maskSource ID、上限 10000 层、同步折叠状态；Rust（session.rs:491-513）浅拷贝一层，对文件夹复制会丢全部子层。
10. **渲染顺序与悬空父级** — Swift `drawLiveComposite` 按 `renderLayers`（层级树深度优先，`LayerGroups.swift:89-92`）绘制，且悬空 parentID 的层不进 `effectiveVisibleIDs`（LayerGroups.swift:11-26 按父链访问）；Rust `composite_onto`（render.rs:86-97）按数组顺序绘制，`effective_visible_ids`（document.rs:648-675）把悬空父级视为可见。
11. **`can_edit_appearance` / `can_edit_mask` 丢了 `canEditLayers` 合取项** — Swift `LayerAppearance.swift:83`、`LayerMask.swift:221` 均含 `canEditLayers`；Rust session.rs:579-582 / 1068-1070 只查选中数和活动层。
12. **`toggle_layer_mask` / `delete_layer_mask` 无 `canEditMask` 守卫** — Swift `LayerMask.swift:272`/`:280` 都先查 `canEditMask`；Rust session.rs:1092/1110 只查蒙版存在。
13. **app 层 `add_group` 与 Swift `addGroup` 语义不同** — Swift（`LayerGroups.swift:177-194`）命名 "Folder N"、建**空**组插在活动层上方、组的 parent 挂到活动层的 parent、undo 名 "New Folder"；Rust app（`layers_panel.rs:351-384`）命名 "Group N"、**把活动层移进新组**、组总是挂根、undo 名 "New Group"。
14. **`LayerAdjustment.is_valid` 缺 HSV 细分设置校验** — Swift `isValid`（`LayerAdjustment.swift:128-141`）会校验 `resolvedHSV.adjustments`/`bands`；Rust `is_valid`（adjustment.rs:280-303）完全没有 `hsv_settings` 的检查。
15. **画笔提交 undo 名** — Swift `commitPaintSnapshot`（`EditorSession+Brush.swift:141`）用 "Paint Mask"/"Erase"/"Blur"/"Clone Stamp"/"Spot Healing"/"Brush Stroke"；Rust app `commit_stroke`（canvas.rs:519-527）用 "Spot Healing"/"Erase"/"Paint"（"Brush Stroke"→"Paint"，且无 Clone Stamp/Blur 专名）。
16. **`NavigationTool` 的 Idle** — Swift `allCases` 含 `idle`（`EditorSession.swift:89-92`），label 三元链对 idle 落到 "Zoom (Z)"（`:97-98`）；Rust `NavigationTool::ALL`（document.rs:723-739）15 个不含 Idle，且给 Idle 单独定义了 label "No tool (A)"（document.rs:777）。
17. **`WandSettings.sampleSize` 表示不同** — Swift 是 `WandSampleSize` 枚举（`MagicWand.swift:14`）；Rust 用 `sample_radius: usize`（session.rs:59）。
18. **`beginTransform` 无 `persistent` 参数** — Swift `beginTransform(persistent:)`（`EditorSession.swift:375`）供 nudge/duplicate 传 `false`；Rust `begin_transform`（session.rs:1156）恒为 `persistent: true`，`TransformEdit.persistent` 无人读。

---

## 三、【简化】移植了但能力降级

1. **`LayerMask` 表示降级**：Swift 是单通道 DeviceGray `CGImage` + `isValid` 校验 + 96px 缩略图（`LayerMask.swift:6`/`:23`/`:35`）；Rust 是 RGBA `Bitmap`（document.rs:242-311），只移植了 `placement_moving_layer`/`background_of`（document.rs:279/294，逻辑一致）。
2. **`clipImage` 无缓存层级**：Swift `LayerMask.clipImage` + `MaskPlacementCache`（LRU 8 项 / 64MP 预算）+ `DownsampleCache`（`LayerMask.swift:110-166`）；Rust `mask_clip_image`（render.rs:27-57）每次重算，无缓存无 limit。
3. **`LayerMask.placed` / `drawSmooth` / `FolderMaskClip`** — `LayerMask.swift:80`/`:94`/`:172`；Rust 用 render.rs:148-174 的内联 folder-mask 循环近似，无独立可复用 API。
4. **`effective_visible_ids` 只做可见性**：Swift 版本同时承担渲染顺序（hierarchy entries）；Rust 拆开后丢了顺序语义（见行为差异 10）。
5. **`add_blank_layer` 插入点简化**：Swift 在选中文件夹时把新层插到文件夹内容最上方（`EditorSession.swift:614-628`）并 `collapsedGroupIDs.remove(parent)`；Rust（session.rs:371-399）只做“活动层上方”，且 `collapsed_group_ids`（session.rs:132）是**从未被读写的死字段**。
6. **`EditorSession` 大量 UI/异步状态未移植**：`viewport`、`cropError`、`effectsEditing`/`effectsPreviews`、`hueSaturationTask`、`filterEdit`、`pixelClipboard`、`colorPicker`、`renamingLayerID` 等（`EditorSession.swift:104-292` 一带）在 core 无对应，部分由 app 层临时状态替代。
7. **`selectedLayerIDs` 从 `Set<UUID>` 降为 `Vec<Uuid>`**（session.rs:130），无去重/无有效性过滤（Swift `selectLayers` 的 `valid` 交集逻辑，LayerGroups.swift:100-103）。

---

## 四、【多出】Rust 独有（简要）

- `session.rs:517-575` `add_shape_layer` / `next_shape_name`：把 Swift `ShapeTool.swift` 的 `finishShape` 会话部分提前收进 core（Swift 侧该逻辑在 ShapeTool.swift:161 附近）。
- `document.rs:777` 给 `Idle` 工具的专用 label（Swift 无）。
- `history.rs:212-232` `EntryClone` trait 与 `ArcPtr` 辅助（实现细节）；`session.rs:301/310` `document_size()`/`selection()` 便利方法。
- 全模型加 `serde::Serialize/Deserialize` 派生（Swift 用 JSONEncoder/Codable）。
- `history.rs:381` `transform_not_saved_blocks_nothing` 是一个**无有效断言的空测试**（只调了 `is_valid()` 不检查结果）。

---

## 五、测试缺口清单（Swift 用例 → Rust 对应）

（Rust 测试名均指 `#[cfg(test)]` 内联模块；`crates/core/tests` 为空。）

**HistoryTests.swift（7）**
- `everyLayerEditRoundTripsWithSelection`(:8) — **部分**（history.rs `undo_redo_roundtrip` + session.rs `undo_reverts_layer_add`；reorderLayers 段无对应）
- `navigationNoOpsAndSaveRevisionPreserveHistory`(:36) — **部分**（history.rs `mark_saved_and_reset`、session.rs `no_op_edit_preserves_history`；zoom/reorder no-op 组合无）
- `replacementCanvasAndNestedTransactionsUndoAsOne`(:63) — **部分**（history.rs `nested_transactions_fold_into_one`；换画布 undo 无）
- `historyBlockedDuringImportsAndDialogs`(:83) — **缺**
- `batchImportIsOneEntryAndFailuresDoNotAddHistory`(:103) — **缺**
- `queuedImportsHaveSeparateUndoEntries`(:125) — **缺**
- `historyBoundsEntriesAndUniqueRetainedPixels`(:139) — **部分**（history.rs `entry_limit_trims_oldest`；`retainedBytes==0` 断言无）

**LayerTests.swift（11）**
- `blankLayersAreTransparentAndInsertedAboveSelection`(:14) — **部分**（`blank_layers_number_sequentially`）
- `deletionPreservesCanvasAndChoosesNeighbor`(:27) — **缺**（对应行为未移植）
- `renameAndVisibilityKeepIdentity`(:43) — **部分**（`rename_and_visibility_are_undoable`，无 trim 断言）
- `reorderTranslatesVisibleOrderAndKeepsSelection`(:56) — **缺**
- `unavailableActionsDoNotChangeDocument`(:69) — **缺**
- `nativeSelectionDoesNotReloadRowsAndReorderKeepsIdentity`(:82) — **缺**
- `selectionAndRenameDoNotInvalidateCanvasButPixelChangesDo`(:118) — **缺**
- `compositingHonorsVisibilityOrderAndBlankLayers`(:141) — **部分**（render.rs `hidden_folder_hides_children`/`opacity_dims`）
- `newCanvasStartsWithOneSelectedEmptyLayer`(:182) — **有**（`new_canvas_starts_with_layer_1`）
- `deletingAMultiSelectionRemovesEveryLayerInOneStep`(:192) — **缺**
- `duplicatingALayerByDraggingPlacesTheCopyAsOneStep`(:223) — **缺**

**GroupTests.swift（4）**：`nestedGroupsMoveOutCollapseAndDeleteUndo`(:8)、`hiddenParentOverridesChildrenAndExportOrderFollowsGroups`(:38)、`groupsRoundTripAndSurviveImageAndCanvasResize`(:71)、`malformedParentLinksAndCyclesAreRejected`(:97) — **全部缺**（仅 store.rs `hierarchy_rules`、document.rs `effective_visibility_inherits`/`descendants_fold` 覆盖校验与可见性）

**GuideTests.swift（9）**
- `newGuidesUndoAndClear`(:18) — **有近似**（session.rs `guides_add_clear`）
- `lockPreventsCreatingAndMoving`(:35) — **缺**
- `projectRoundTripAndLegacyRejection`(:51) — **有**（store.rs `guides_round_trip`）
- `canvasAndImageSizeMoveGuides`(:78) — **有**（canvas.rs `resize_canvas_moves_layers_and_guides`）
- `flipCanvasMirrorsGuides`(:93) — **缺**（实现有、测试无）
- `snapTargetsFollowViewMenu`(:106) — **缺**
- `layoutGridLinesIncludeMajorsAndSubdivisions`(:133) — **有**（document.rs `layout_grid_lines`）
- `moveToolDragsAGuideAndRulerDropDeletesIt`(:140) — **缺**
- `rulerStepUsesNicePixelIntervals`(:170) — **缺**

**LayerMaskTests.swift（9）**：仅 `addDisableDeleteUndoAndTargetSelection`(:36) 有**部分**对应（session.rs `add_and_delete_mask`，无 target 选择段）；`coverageOpacityAndDisabledMasksRenderCorrectly`(:66)、`transformedMaskResizeAndCanvasChangesStayAligned`(:86)、`projectAndPNGPreserveCoverageAndDisabledState`(:110)、`masksRejectOlderSchemaAndUnsafePaths`(:132)、`folderMasksClipEveryLayerInsideAndMultiplyWithTheirOwnMasks`(:168)、`folderMaskCanBePaintedInvertedAndLoadedAsASelection`(:199)、`canvasShowsAFolderMaskWhileItIsBeingPainted`(:236)、`folderMasksSaveResizeAndNeedTheNewFormat`(:282) — **缺**（render.rs `folder_mask_clips_contents`、document.rs `layer_mask_placement_logic` 仅覆盖极小切片）

**LiveMaskTests.swift（5）**：`clippingColorPreservesSoftBaseAlphaWithoutBlackFringe`(:6)、`optionClickCreatesSharedStackAndDragOutReleases`(:59)、`hiddenBlackSourceSuppliesAlphaAndRasterMasksMultiply`(:82)、`cyclesUndoPersistenceBakeAndDelete`(:96)、`movingSourceChangesCoverageAndChainsMultiply`(:118) — **全部缺**（仅 render.rs `clipping_mask_uses_source_alpha`、store.rs `live_mask_rules`）

**LayerAppearanceTests.swift（5）**
- `hoverPreviewIsTemporaryAndNeverChangesSavedState`(:7) — **缺**
- `moveToolNumberKeysSetSelectedLayersOpacityAsOneUndo`(:30) — **缺**
- `blendModesAndOpacityMatchKnownPixels`(:73) — **有**（blend.rs 系列 + render.rs `multiply_blend_darkens`/`opacity_dims`）
- `opacityDragIsOneUndoAndKeepsSources`(:96) — **有近似**（`opacity_edit_folds_into_one_step`）
- `appearancePersistsThroughSaveResizeAndTransparentExport`(:112) — **部分**（project/convert.rs `round_trip_preserves_all_layer_styling`）

**AdjustmentLayerTests.swift（8）**：`globalAdjustmentAffectsBelowButNotAboveAndRemainsLive`(:25)、`clippedCurveChangesOnlyItsBaseAndCopyMergedMatchesExport`(:41)、`hueOpacityAndMaskPreserveOriginalPixels`(:57)、`adjustmentPersistsDuplicatesAndUndoRestoresSettings`(:73)、`curvesIdentityAndImageCommandPreserveAlpha`(:90)、`adjustmentBlendAndSoftMaskPreserveCoverage`(:101)、`invertAppliesWithoutAnEditor`(:122)、`legacyHSVStillDecodesAndRenders`(:262) — **全部缺**（Rust 只有 adjustment.rs 的 7 个模型测试 + render.rs `adjustment_layer_alters_pixels`）

**ProjectWorkspaceTests.swift（5）**：`newCanvasOpensAnEmptyTabWithoutAModal`(:7)、`layerDropProviderCopiesIntoANewProject`(:23)、`tabsKeepIndependentDocumentsAndUndo`(:38)、`crossProjectCopyRemapsIdentityAndHasIndependentUndo`(:58)、`dockCreatesTabsAndTargetedImportUsesExistingTab`(:78) — **全部缺**

**BlendShortcutTests.swift（1）**：`shiftPlusAndMinusStepTheActiveLayersBlendModeWhereverFocusIsExceptTextFields`(:7) — **有对应**（session.rs `blend_mode_cycles_through_menu_order`；键盘焦点路由部分不适用）

**SmartEditTests.swift（4）**：`fillContinuesRepeatingTexture`(:6)/`fillReconstructsBackgroundInsideSelectionAndUndoes`(:42)/`cancelAndNoSourceLeaveOriginalUntouched`(:60) — **部分**（tools/content_fill.rs 3 个测试只覆盖填充算法，session 级取消/undo 流程无）；`visionRequestRunsOnAnImage`(:72) — **缺**（依赖 Apple Vision）

---

### 域内无 Rust 对应的 Swift 文件（明确列出）
`FloatingSelection.swift`、`AdjustmentEditing.swift`、`ProjectWorkspace.swift`、`EditorSession+Projects.swift`、`ToolDefaults.swift` —— 这 5 个文件在 Rust 侧**完全无功能对应**（至多有类型骨架如 `FloatingTransform` 结构体）。`LayerGroups.swift`、`LiveLayerMask.swift`、`LayerMerge.swift`、`ColorPalette.swift`、`Guides.swift` 为**部分对应**。
