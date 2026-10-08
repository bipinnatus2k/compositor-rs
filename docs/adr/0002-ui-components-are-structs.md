# ADR 0002：UI 组件一律为 struct 组件——函数式组件清零与 AppState 解耦路线

- 状态：Accepted（2026-10-09 架构裁定，refactor-ui-components 分支 4 个 commit 落地）
- 范围：`crates/app`（`ui.rs`、`sheets_ui.rs`、`ui/*` 组件层与 `content_view.rs` 的装配点）；`compositor-core` 零改动

## 背景

移植过程中，app crate 的 UI 组件以**自由函数**生长：返回
`impl IntoElement` 的 `dropdown()`、`labeled_slider()`、
`render_menu_bar()` 等共 36 个，而 gpui-kit 的组件惯例（coding-guides
"Choose the right unit"）是轻量可复用组件做成 `RenderOnce` struct——
caller 供 props 与回调，值被消费即弃；`Entity<T>` 只留给有独立生命周期的
状态。全 crate 此前没有任何 `RenderOnce` 实现。

自由函数形态有三个实际问题：

1. **状态耦合藏在签名里。** 大量函数收 `&WeakEntity<AppState>` 或
   `&mut Context<AppState>`，组件逻辑与 app 壳互相缠绕；而 AppState
   本身是移植期的装配壳，**既定方向是最终移除**（原版Swift 并无
   AppState——每个 view 的常规 prop 就是 session 句柄）。
2. **去重欠账。** `labeled_slider` 两处同名不同参、`labeled_checkbox`
   与 `checkbox` 逐字符重复、`sampling_dropdown` 重复 `dropdown` 的装配。
3. **测试钉住的是形状而非组件。** helper-seam 惯例（cfg(test) shim /
   测试内镜像 / test-only reader）要求重构不动调用形状，函数形态让
   "哪里是组件、哪里是数据 helper"无从分辨。

## 裁定

**UI 组件一律为 struct 组件：无状态组件 = `RenderOnce`，不做 entity
（"无状态 entity"不是概念——entity 专指 `Entity<T>` + `impl Render`）；
只有承载 view 本地 `@State` 语义的组件才考虑 Entity。**

1. **props 按原版 Swift view 的属性表设计**（8/11 个 sheet 在原版就是
   独立 struct View，存储属性即 props）；原版无对应物的（MenuBar、
   通用控件层——原版是 `Picker`/`Toggle`/`.commands` 内联），按
   gpui-kit 组件习惯自拟形态。
2. **组件不命名 AppState。** 状态突变一律经回调上报（controlled value：
   `AppCallback<T>` / `ClickHandler` / `PressHandler` / `DragMoveHandler`
   / `DragUpHandler`，皆在 `App` 层）；`AppHandler`（`Fn(&mut AppState, …)`）
   作为过渡期菜单模型保留，随 AppState 一并退役。
3. **过渡适配器家族收敛在 `ui.rs`**：`app_dispatch`、`app_on<T>`、
   `app_on_click`、`app_on_press`、`app_on_drag_move`、`app_on_drag_up`
   ——把现有的 AppState 风格闭包包成组件回调，调用点（AppState 方法）
   即"薄装配函数"：读状态 → 构造组件 → 返回。AppState 移除时只有这层
   适配器重写，组件直接接走。
4. **纯装饰 helper 保留函数**：`text_label`、`div_row`、`unsaved_dot`；
   数据模型 `MenuEntry` 与 `build_menu`（PopupMenu 装配器，非组件）保留。
   被测试钉住的数据函数（`menu_entries`、`tool_title`、`panel_estimated_size`
   等）形状不动。

## 否决项

- **全部组件做成 `Entity<T>`。** coding-guides 明言"不要把每个视觉片段
  都变成 entity"：entity 边界有生命周期与协调成本；本轮 36 个组件里
  没有一个拥有需要跨帧独立身份的状态（原版的 `@State` 选中/拖拽在
  curves 一处，见下方偏差记录——其搬迁归状态模型重构）。否决"一律
  Entity"。
- **维持自由函数、只做去重。** 去重收益一次性，而函数形态让"AppState
  解耦"永远停在口头——每多一个函数组件，未来移除 AppState 的机械改形
  面就大一分。否决。
- **现在就把 `AppHandler`/`MenuEntry` 泛型化为 session 句柄。** 菜单
  handler 的写入逻辑横跨 tab/流程层（`state.tab().session`、undo 守卫、
  preview 刷新），提前泛型化等于把状态模型重构（parked 37 号票）的活
  提前做半套。否决；过渡适配器隔离了这一步。

## 偏差记录（对裁定表的执行偏差，均为工程裁定）

- **`labeled_slider_row` 消解为私有 helper** 而非 struct：其两个消费方
  （两版 `labeled_slider`）本轮都已组件化，留下的 pub struct 会是无调用
  方的死 API。
- **`CurvesCanvas` 保持 `RenderOnce`**（裁定表曾定 Entity 携带
  selected/dragging/bounds）：选中/拖拽状态的搬迁与 settings 草稿的
  归属纠缠，属状态模型重构；本轮只消除了真正的 render 纯度违规——
  布局期不再重入 app 实体，bounds 经模块私有全局 `CurvesCanvasLayout`
  桥接到指针 handler（与取色器 `PickerFieldState` 同款、已命名注档）。
- **`PickerFieldState`/`GradientCache` 保留模块私有 gpui 全局**（裁定表
  曾定收进 element state）：窗口级 handler 够不到 element state；全局
  已显式命名为布局→事件桥与共享位图服务。
- **面板拖拽 cookie 改为开板期分配**（`FloatingPanels::show`）而非
  per-render：活动拖拽由此在其自身触发的重渲染间存活，面板关闭按钮的
  element id 也随域稳定。

## 后果

- 调用点全面改形（约 25 个文件），但零测试改动——helper-seam 惯例
  兑现；行为逐位保持（渲染输出、id、尺寸不变）。
- 剩余的自由函数返回元素只剩画布侧 render 树（`AppState::render` 及其
  私有片段），它们是装配点，随 AppState 移除一并处置。
- 36 个组件中 3 个保留为函数、1 个消解为私有 helper、其余全部 struct 化；
  合并去重 3 组（`Dropdown`、`LabeledSlider`/`SheetRow`+`Slider`、
  `LabeledCheckbox`）。
