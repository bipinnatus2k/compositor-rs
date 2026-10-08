# ADR 0001：EditorSession 的实体模型——每 tab 一个 GPUI 实体

- 状态：Accepted（2026-10-08 架构裁定 Q3/Q5，33 号票落地）
- 范围：`crates/app`（`tabs.rs`、`state.rs` 与全部 UI/画布访问面）；`compositor-core` 零改动

## 背景

Swift 原版里，`EditorSession`（EditorSession.swift:101）是一个
`@Observable` 类，由 `ProjectWorkspace`（同为 `@Observable`，:19-20）按
tab 持有；SwiftUI 依属性级观察自动失效视图。Rust 移植此前把
`EditorSession` 作为普通结构体**值持有**在 `ProjectTab.session` 里，窗口
用一个自管的 `revision: u64` 计数器（`AppState::touch`）做全局失效，帧
缓存以 `(tab_id, revision)` 为键。

这带来两个问题：

1. **失效粒度只有窗口级。** 任何会话突变都使整个窗口重绘；非活动 tab
   的会话变化（后台导入、跨项目复制落点）只能等下一次任意 touch。
2. **失效路径是手写的。** 每个突变点都要记得调 `touch(cx)`，漏一处就
   是"改了不刷新"的静默 bug；GPUI 原生的 `Entity::update` + `notify` +
   `observe` 机制完全没有用上。

2026-10-08 的架构复审（Q3：会话状态如何失效；Q5：实体拆分到什么粒度）
给出了两个候选与一个既定方向，裁定如下。

## 裁定

**每个 `ProjectTab` 持有一个 `Entity<EditorSession>`（方案 (a)），失效走
GPUI 原生机制。**

1. `ProjectTab::new(name, cx)` 里用 `cx.new(|_| …)` 建 session 实体
   （ToolDefaults 的启动默认值应用随迁入构造闭包）。`ProjectTab` 本身
   仍是普通结构体，由 `ProjectWorkspace` 的 `Vec<ProjectTab>` 持有。
2. 访问面：
   - 读：`tab.session.read(cx).<field>`（`&self` 场景）或
     `session.read_with(cx, …)`（泛型上下文，tabs.rs 内部与测试）；
   - 写：`tab.session.update(cx, |session, _| { … })`，随后**保留**原有
     的 `touch(cx)`（revision 语义暂不变——见"迁移"）。
3. 观察者：`AppState` 对**活动 tab** 的 session 实体挂
   `cx.observe(&session, |_, _, cx| cx.notify())`（
   `AppState::sync_session_observer`），tab 切换/关闭/新建等改变活动
   tab 的流程后重新挂接，**非活动 tab 不被观察**（丢弃旧 Subscription
   即撤销观察）。
4. 对话框编辑态（`levels` / `hue_saturation` / `filter_edit` /
   `effects_editing` / `color_picker` 等 Option 字段）留在 session 实体
   内一起失效，不单独拆实体。

## 否决项

- **维持现状：单窗口实体 + 全局 revision 计数。** 失效粒度粗（见背
  景），且手写失效路径与 GPUI 机制并行生长，越晚迁越贵。测试基线
  （892 项全绿）与帧缓存键 `(tab_id, revision)` 已就位，迁移成本此刻
  最低，故否决。
- **完全属性级模拟 Swift `@Observable`。** 在 Rust 里给
  `EditorSession` 的每个字段造观察/订阅 plumbing，是对宏驱动的属性观察
  的手工复刻——形似神不似（四不像）：API 面爆炸、每加一个 core 字段
  都要同步两处、还与 core 不依赖 gpui 的宪法冲突。否决；等价收益由
  下述 (b) 迁移用**实体边界**而非**字段级订阅**取得。
- **`ProjectWorkspace` 做成实体（Swift 是 `@Observable` 类）。** Rust 侧
  保持普通结构体挂 `AppState`。原因：签页开关（建/关/切换 tab、quit
  流程）是低频交互，窗口级失效已足够；把它做成实体只会引入第二个观
  察边界而无失效收益。此为**有意偏差**，记录于
  `docs/rust-vs-swift-diff-report.md` 的偏差清单；若日后签页出现高频
  突变（如拖动重排 tab），再重开裁定。

## 后果

- 正向：会话有了稳定身份（`EntityId`），观察/失效走 GPUI 原生路径；
  `copy_layer` 等跨 tab 读写不再需要 `split_at_mut` 的借用杂技（两个
  session 实体各自可借）；tabs.rs 的 XCTest 移植测试改为
  `#[gpui_kit::test]` + `TestAppContext`，与实体构造对齐。
- 代价：约 590 处访问点从 `tab().session.<x>` 机械改形为
  `read(cx)`/`update(cx, …)`；`&self` 的读路径（`composite`、
  `transform_displayed`、菜单构建等）签名增加 `cx: &App`。行为不变——
  全量测试与门禁绿是验收标准。
- `touch(cx)` 的 revision 计数**暂时保留**：帧缓存与缩略图缓存仍以
  `(tab_id, revision)` 为键，通知作用域的细化（谁 bump revision、
  observer 触发什么）不在本 ADR 的范围。

## Future work：(a) → (b) 的既定迁移方向

**用户明确要求留档：本裁定是 (a)，方向是后续迁移到 (b)——完整镜像
Swift 的 `@Observable` 对象清单，用实体边界替代属性订阅。** 落地条目
（即 parked 的 37 号票）：

1. `DocumentHistory` → 独立 `Entity<DocumentHistory>`（Swift：
   DocumentHistory.swift:6-7 是独立 `@Observable` 类，session 持有引
   用）；undo/redo 的失效从 session 级细化到 history 级。
2. 对话框对象 → 独立实体，session 以字段持有（Swift 四个
   `@Observable` 类）：
   - `LevelsEdit`（Levels.swift:104-105），
   - `HueSaturationEdit`（HueSaturation.swift:367-368），
   - `FilterEdit`（Filters.swift:200-201），
   - `ColorPickerState`（ColorPalette.swift:179-180）。
   对话框滑杆拖动不再触发 session 级失效。
3. 通知作用域细化：全局 `touch(cx)` revision → 按 tab/按关注点作用域
   （缓存键 `(tab_id, revision)` 已就位，是细化基础）；非活动 tab 的观
   察裁剪（本 ADR 已做活动 tab 单观察，进一步按面板订阅细化留此）。
4. 完成后把本 ADR 状态改为 Superseded/Extended。

前置条件：33 号票落地并稳定一轮；性能依据先行——对话滑杆与 undo 通知
热点的 profile 再评估，避免为细化而细化。
