# ADR 0003：废除 AppState——状态实体与视图实体双层镜像 Swift

- 状态：Accepted（2026-10-09 grill-with-docs 架构裁定）；**落地完成
  （2026-10-10）**——票序 38→39→40→41→42→43→37 全部落地，`AppState`
  除名；本文由票 44 定稿回填。
- 范围：`crates/app`（全部）；`compositor-core` 仅票 38 的 `CanvasViewport` 增补
- 取代：ADR 0001「ProjectWorkspace 保持普通结构体」的有意偏差；执行 ADR 0002 预留的「AppState 移除路线」

## 落地台账（2026-10-10 回填；分支 refactor-app-architecture）

| 票 | 内容 | commit |
|---|---|---|
| 38 | `CanvasViewport` 进 core session（每 tab 视口还原） | 6385f59 + fff37cb（Swift 测试套件移植） |
| 39 | 画布视图实体 `Entity<EditorCanvasView>` | e35fab3 |
| 40 | 图层面板实体 `Entity<LayersPanelView>` | fb0c3cf |
| 41 | 七面板内容实体（SHORTCUTS_SHEET 消解） | f33b433 |
| 42 | 工具头族/welcome/status 实体（九实体） | ad869b8 |
| 43 | `Entity<ProjectWorkspace>` + `ProjectWorkspaceView` 改名除名 + `ContentView` 双层 `.id` 镜像 | 2d72632 |
| 37 | 失效卫生：缩略图内容键控 + bypass 统一 + 失效仪表 | 8966d69 |
| — | 41–43 双轴 review 补课（零硬违规、五项披露） | 93d8f9e |

全程 919 测试绿（core 627 + app 182 + 集成 110），既有断言零改动；
逐票 gates 与 boot smoke 全过。执行裁定细节的权威记录在
`.scratch/parity-review-tickets/issues/38-44`（票面审计表 + 落地记录）
与 `docs/rust-vs-swift-diff-report.md` 顶部各架构裁定节。

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
- **落地期新增（44 定稿收口）**：
  - **适配器「退役」= 改名终形**：`ui.rs` 六适配器以
    `WeakEntity<ProjectWorkspaceView>` 签名同形重写后，已是普通的根视图
    父回调（组件上报突变的合法形态），不再是过渡缝——裁定 6 的「退役」
    按此解释（review 披露 ⑦）。
  - **MenuBar 解析点**：保持吃活动 `Entity<EditorSession>`、根渲染期从
    workspace 实体解析——即 Swift `.commands` 计算属性
    （CompositorApp.swift:7）的同构（review 披露 ⑥；票面原文字过度
    指定，按实现修正）。
  - **sync_content 亚帧窗口**（review 披露 ⑧，低危留档）：切 tab 突变
    与下一帧渲染之间派发的事件仍路由到将死的旧 ContentView 子实体
    （化妆性）；`can_switch` 覆盖交互态但不覆盖此亚帧窗。
  - **面板重入规则（2026-10-11 崩溃修复沉淀，长期约束）**：面板实体的
    回调（`cx.listener`/`panel_on*`，运行在面板租约内）不得让根再读/更
    新同一面板实体——gpui 双租约即 panic。两条合法出路：面板把值经缝
    传过去（filter 的 settings+seed），或控件改经根句柄裸闭包分发
    （`app_on_click`，根上下文无面板租约，根侧读面板合法）。页脚
    OK/Cancel 已定为根分发（`panel_footer`），回归测试锁在
    `state.rs::panel_reentrancy_tests`。
  - **窗口级留根项**：模态 sheet 宿主、面板宿主 + panel_views、帧缓存、
    preview 族、pending_save/delete_ask_ids、~130 on_action 注册与流
    方法（Swift 对应物散布在 .commands/ContentView @State/对话框对象，
    Rust 集中于根视图——`can_switch` 保证行为等价）。

## 后果

- `AppState` 除名（2d72632，全 crate 213 处引用清零）；`ui.rs` 六适配器
  改名为根视图父回调（见偏差记录）；~120 处原引用清零。
- 切 tab 行为变化（朝 parity）：视口按 tab 还原（refit hack 删除，票
  38）；每-tab UI 草稿随 `ContentView` 重建；非活动 tab 自动失观察；
  滑杆/输入框 seeding 随重建重播、状态条文案随 tab 重置（review 追认，
  皆 Swift `.id` 真实语义）。
- core 公共 API 增补：`CanvasViewport` 类型 + `EditorSession.viewport` +
  `fit`/`zoom`/`zoom_keyboard`（票 38，commit 描述按 AGENTS §7 列清单）。
- 既有 RenderOnce 组件（ADR 0002 的 30 个 struct）不被推翻，成为 view
  entity 的渲染体；helper-seam 测试惯例继续适用。
- 919 个测试断言零改动；构造代码机械适配。
- 失效面收敛（票 37 数字）：滑杆拖动缩略图再生 15→0、蒙版缩略图每帧
  重算归零、revision 写入点收敛为 `touch`/`bump_revision` 两个。
- ADR 0001 的 Future work（(a)→(b) 实体清单）已取消并改写（状态记
  Extended by 0003，票 44 落）。

## Future work

- 票 37 落地后若 profile 显示对话框拖动期间的 chrome 重渲染为通知热点：
  为单个最热对话框评估 app 侧影子实体（局部手术，数据驱动，不预先支付
  双真相税）。失效仪表已就位（票 37），量测路径现成。
- 帧缓存 `(tab_id, revision)` 的进一步作用域化：合成是失效的必要成本，
  仅在 profile 证明可细分（如按 dirty rect）时再动。
  （→ 2026-10-25 已由 ADR 0004 立项落地路线：票 49–54。）
- 三面板 `session` 死字段（levels/hue_saturation/filter 的 house 惯例
  保持项）：下次触及对应面板票时顺手清理。
- sync_content 亚帧窗若在真实使用中显形（尚未观察到）：评估
  selected_id 门。
