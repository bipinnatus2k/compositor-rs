# ADR 0003：废除 AppState——状态实体与视图实体双层镜像 Swift

- 状态：Accepted（2026-10-09 grill-with-docs 架构裁定；落地按票 38→39→40→41→42→43→37→44 推进）
- 范围：`crates/app`（全部）；`compositor-core` 仅票 38 的 `CanvasViewport` 增补
- 取代：ADR 0001「ProjectWorkspace 保持普通结构体」的有意偏差；执行 ADR 0002 预留的「AppState 移除路线」

## 背景

移植期生长出的 `AppState` 是 43 个字段的神对象：窗口级 viewport + revision
帧/缩略图缓存、13 变体 `CanvasDrag`、sheet/panels/panel_widgets、12 个
`Entity<InputState>`/`Entity<SliderState>` 控件、preview/flow 杂项全部挂在
其上；root view 就是 `Entity<AppState>`（`content_view.rs` 的
`impl Render`）；约 30 个文件含 `impl AppState` 块、全 crate 120 处引用。
`ui.rs` 的 `app_*` 六件套适配器（ADR 0002）是过渡缝。单 Render 树下任何
session 通知都全窗口重渲染，帧缓存键 `(tab_id, revision)` 是唯一的失效
卫生，另有 13 处内联 `revision += 1` bypass。

原版 Swift 的形态：`ProjectWorkspaceView` 观察 `@Observable ProjectWorkspace`，
内部以 `.id(workspace.current.id)` 对活动 tab **重建整个 `ContentView`**
（session 绑定 + 全部每-tab UI 状态随之重生，非活动 tab 自动失观察）；
`EditorSession` 上挂 `CanvasViewport`（切 tab 自动还原每 tab 视口）；
`DocumentHistory` 与四个对话框对象是纯数据 `@Observable` 类；
`CanvasView`（NSView）的拖拽态是 plain private var；菜单在 App 层
`.commands` 声明一次、经计算属性取 active session 直调；面板是 NSPanel
独立窗口。

parked 的 37 号票（history/对话框拆实体）卡在一个死结：单 Render 树下
物理拆实体不缩小失效面。本裁定改写该前提——视图树镜像后，失效粒度由
视图实体边界承载。

## 裁定

**目标总纲：全镜像。状态实体清单 = Swift `@Observable` 清单；视图实体
镜像 Swift 视图树；root 只组装子视图实体、不做装配渲染。**

1. **状态实体**：`Entity<ProjectWorkspace>` 纯状态实体（无 Render，镜像
   `@Observable`）；`ProjectTab`/`ProjectController` 保持普通类型（镜像
   Swift 非 observable 事实）。`EditorSession` 已是每 tab 实体
   （ADR 0001，33 号票）。**history/对话框对象不拆实体**——源码核实
   `DocumentHistory`/`LevelsEdit`/`HueSaturationEdit`/`FilterEdit`/
   `ColorPickerState` 均为纯数据类（`@Observable final class`，无
   View/NSView，AppKit 仅 NSSound×4/NSColor×2）；core 不依赖 gpui 使
   core 字段不可能成为 gpui 实体；实体粒度的 `@Observable` 镜像已由
   33 号票完成。失效细化改道「视图实体 + 缓存键作用域」（票 37）。
2. **视图实体双层**：root = `Entity<ProjectWorkspaceView>`（观察
   workspace 实体），持 `Option<Entity<ContentView>>`，切 tab 重建
   （镜像 `.id`：每-tab UI 草稿重建、非活动 tab 自动失观察）；
   `ContentView` 观察 session、组装子视图实体。命名跟随原版，文件落
   根级（既有文件组织裁定）。
3. **实体化判据**：读可变观察状态（session/workspace 实体字段）的
   Swift view struct → view entity；纯静态/纯装饰 → 保持 `RenderOnce`
   或函数。既有 RenderOnce 组件层（ADR 0002）不动摇——降为 view entity
   的渲染体/内部构件，props 设计与测试保留。
4. **字段归属逐字段按 Swift**：`viewport` → core session（`CanvasViewport`，
   Swift `EditorSession:170`，删 `fit_for`/`fit_tab` refit hack）；
   `CanvasDrag` → 画布视图实体私有（镜像 CanvasView plain vars，随 tab
   重建消亡）；控件实体（slider/input 族）→ 各视图实体；
   `SHORTCUTS_SHEET` → shortcuts 面板实体字段；`DRAIN_PUMP_RUNNING` →
   root。搬不动或有害的逐条记 diff report。
5. **动作路由**：注册点留在新 root（镜像 `.commands` 单点声明语义），
   handler 体 = 薄装配路由进实体方法；画布键盘监听随画布实体（镜像
   CanvasView 的 NSEvent 局部监听归属）；`keys.rs` action/bindings 层不动。
6. **全局与模态机制**：`PickerFieldState`/`CurvesCanvasLayout`/
   `PanelDragState`/`NEXT_COOKIE`/`GradientCache` 按 ADR 0002 保留，
   不重开（实现中自然消解处顺路收编、逐条记录）；模态 sheet 机制不动
   （Pending 留 controller、`SheetState` 留 root，偏差沿用）。
7. **模态锁定行为约束**：对话框/面板打开或文档忙碌 → 不可关闭该文档、
   不可切换签页；平移与缩放不受限。现行 `can_switch` 九个消费点
   （select/new_canvas/open/close/confirm_quit/close_window/receive/
   drain/copy_layer）已兑现该语义，重构后逐点保留。
8. **迁移顺序：先剥离后换壳**。38 viewport→core → 39 画布实体 →
   40 图层面板 → 41 面板宿主+内容 → 42 工具头/welcome/status →
   43 workspace 实体化+双层 root+AppState 除名 → 37 失效卫生 →
   44 文档收口。C/D/E 与 B 技术独立，但 `content_view.rs` 装配点按文件
   独占规约串行。每票 gates 全绿 + 测试**断言零改动**（构造代码机械
   适配）；票 37 profile 基线先行、复测数字说话。

## 否决项

- **字面拆 history/对话框实体**。core 不依赖 gpui 是宪法铁律，两个工程
  形态皆不可取：所有权倒置（~40 个对话框方法签名手术，且本身违数据
  模型 parity——Swift 就是 session 拥有这些对象）；影子实体（五对象双
  真相，每条 core 内部突变路径都要补同步点，漏一处即 UI 陈旧）。收益
  仅剩对话框拖动期间 chrome 不重渲染——几百个轻量元素的 diff，相比每
  帧 composite 重算小一到两个数量级，且票 37 仪表落地后可测；若数字证
  明热点，届时为单个最热对话框局部上影子实体（Future work，数据驱动，
  不预付）。
- **单层 root**。丢 `.id` 重建语义：每-tab UI 草稿需手工清、非活动 tab
  失观察需手工维护（现状 `fit_for` 模式的延续）。
- **先换壳后剥离**。第一票在子树未搬走时横跨全部文件（workspace 实体
  化 + root 改名 + 画布/面板/图层列表迁移临态），与「架构票独占文件
  清单」规约冲突、难回退；先剥离使每票为局部手术、AppState 逐票瘦身，
  43 只剩装配与流。
- **白名单收紧**（只实体化画布/图层/面板三大子树）。与全镜像矛盾；
  RenderOnce 子树随父全量重渲染，失效面接近现状。
- **sheet 机制按 tab 重排**。Swift 的对应是 ProjectController 经
  NSWindow 呈现的原生 NSSavePanel/OpenPanel，模态本就窗口级、不随 tab
  消亡；重排反而制造偏差。

## 偏差记录（沿用，不重开）

- NSPanel 独立窗口 → 窗口内浮层（`anchored` + `occlude`）。
- 自绘模态 sheet 替代原生 NSSavePanel/OpenPanel；Pending 留 controller、
  `SheetState` 留 root。
- 五个模块级全局（ADR 0002 偏差记录同款）。
- 属性级观察不模拟（ADR 0001 既定）：实体粒度是唯一的观察单元；本裁定
  的失效收益来自视图实体边界与缓存键作用域，不来自更细的观察。
- 票 39 字段归属审计产出的例外逐条记 diff report。

## 后果

- `AppState` 除名；`ui.rs` 六适配器与 `AppHandler` 退役；~120 处引用清零。
- 切 tab 行为变化（朝 parity）：视口按 tab 还原（refit hack 删除）；
  每-tab UI 草稿随 `ContentView` 重建；非活动 tab 自动失观察。
- core 公共 API 增补：`CanvasViewport` 类型 + `EditorSession.viewport`
  （票 38，commit 描述按 AGENTS §7 列清单）。
- 既有 RenderOnce 组件（ADR 0002 的 30 个 struct）不被推翻，成为 view
  entity 的渲染体；helper-seam 测试惯例继续适用。
- 179 个 app 测试断言零改动；构造代码机械适配。
- ADR 0001 的 Future work（(a)→(b) 实体清单）由本裁定取代，票 44 执行
  改写（状态记 Extended by 0003）。

## Future work

- 票 37 落地后若 profile 显示 chrome 重渲染为通知热点：为单个最热对话框
  评估 app 侧影子实体（局部手术，数据驱动，不预先支付双真相税）。
- 票 44：ADR 0001 Future work 改写、CONTEXT.md/README/diff report/
  AGENTS §5 收口。
