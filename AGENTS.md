# AGENTS.md

本文件是本仓库对编码 agent 的共享规范，是**唯一权威入口**：ZCode、Claude
Code（经 `CLAUDE.md` 引入）、Codex 等一律以本文件为准。规范分三层执行：
本文件（L1）、本地 skills（L2）、提交前命令门禁（L3）。

## 1. 项目宪法：1:1 移植纪律

本项目是 macOS 图像编辑器 [Compositor](./Compositor/)（Swift/SwiftUI/Metal +
C 像素内核）向纯 Rust 的 1:1 复刻。原版源码就在 `Compositor/` 目录，随时可
对照；README 的对照表给出每个 Swift 源文件对应的 Rust 模块。

- **公式、算法、数据模型与原版逐位对齐。** 移植 CG / CoreImage / C 内核公式
  时不得"顺手改进"：不重排运算顺序、不换近似算法、不改浮点表达式的结构与
  舍入路径，即使语义等价。
- 与原版的**任何有意偏差**必须在代码注释中写明原因，并在 `docs/` 留有记录。
- **测试是从原版 XCTest 移植的，是验收标准**（规则见
  [`docs/standards/COMPONENT_TEST_RULES.md`](docs/standards/COMPONENT_TEST_RULES.md)）。
  移植测试保持与原用例可对照的命名；不许改断言值或放宽容差来让测试通过。
- 行为拿不准时**以原版源码为准**，不凭直觉、不凭对 macOS 软件的使用经验猜。
- 命名跟随原版（`EditorSession`、`CanvasDocument`、`ImageLayer`…），
  术语裁定见 [`CONTEXT.md`](CONTEXT.md)。

## 2. Canonical 指南：改 UI 前必读

在改动 UI、交互、界面文案、布局、样式、组件用法或应用架构之前，阅读并遵循
本仓库的 canonical 指南（`docs/standards/`）：

- [Design Guides](docs/standards/design-guides.md) — 设计规范
- [Coding Guides](docs/standards/coding-guides.md) — 编码规范

**These guides are requirements, not optional inspiration.** 不要照搬 web
惯例、不要从单个现有界面反推设计体系、不要因为底层功能存在就加控件。完成
后对照两份指南与 Design review checklist 自查，未过自查不算完成。

- 做 UI 任务时同时加载本地 skill：
  [`skills/gpui-kit/SKILL.md`](skills/gpui-kit/SKILL.md)（组件目录 + GPUI
  机制参考 + Coding Guides）与
  [`skills/gpui-kit-design-guides/SKILL.md`](skills/gpui-kit-design-guides/SKILL.md)
  （何时读哪节 + 底线清单）。
- 参考手册（框架教程 / 基础层 / 组件用法）在 `docs/reference/gpui-kit/`，
  只读 vendor。
- 指南正文为组件库视角；**哪些条款对本项目生效以第 8 节适用性清单为准**。

## 3. 常用命令（提交前必跑）

```bash
cargo fmt --check            # 硬门禁：必须零差异
typos                        # 硬门禁：必须零报告（误报进 _typos.toml 并注释原因）
cargo clippy --all --all-targets
                             # 退出码必须为 0；不得新增警告（--all-targets 才能看到
                             # 测试代码里的使用，漏掉它曾删错"未使用"的 import）
cargo test                   # 引擎全量；UI 改动另跑 cargo test -p compositor
```

- 这是本地门禁（L3）。本项目暂无 CI，**不因"没有 CI"而跳过**。
- clippy 存量警告（~190 个，集中于移植内核的源码形状惯用法与测试代码里的
  未使用 import，见 `_typos.toml` 同款的"保留源码形状"裁定）是已知债务；
  **改动到哪个文件就把该文件清零**，全量清零后把命令收紧为
  `cargo clippy --all --all-targets -- --deny warnings`。
- 渲染路径 / 像素内核的性能敏感改动，先 profile 再声称无回归
  （macOS 可用 `MTL_HUD_ENABLED=1 cargo run`、`samply record cargo run`）。
- 新增依赖前检查是否已有等价物（历史上已按
  [`docs/reports/crate-replacement-report.md`](docs/reports/crate-replacement-report.md)
  清点过一轮）。

## 4. Non-negotiables（UI 底线）

源自 gpui-kit design-guides skill 的六条底线，是下限不是替代品：

1. **桌面优先于 web 惯例。** 键盘可达、窗口 chrome、菜单、密集数据视图、
   可调区域、持久导航。
2. **`Button` vs `Link`。** 应用内命令一律 `Button`（需要低调时 `ghost` /
   `outline`）；`Link` 只用于外部 URL 和邮箱。
3. **token 优先于字面值。** 禁裸 hex / `rgb(...)`；用 `cx.theme()` 语义
   token 与 rem 助手。指南中的间距数字是当前默认刻度，不是要照抄的字面量。
4. **状态必须可见。** hover、focus、按下、选中、禁用、加载、校验、危险
   状态各有明确一致的表现。
5. **覆盖层。** Escape 关闭最顶层表面并把焦点还给触发者。
6. **文案写"对象 + 动词"。** `Delete "Roadmap"?` 配 `Delete` 按钮，而不是
   `Are you sure?` 配 `OK`。

## 5. 命名与类型规约

- **禁 `Kind` 作为类型名后缀。** 它不说明枚举到底分类什么；用变体实际
  所是的名字。确无诚实名字可用时保留（如上游 `NodeKind`），并尽量
  `pub(crate)`。外部 crate 的既有名不重写。
- **拼全 `Context`**：`ComboboxTriggerContext`，不缩写 `…Ctx`。`cx` 保留给
  GPUI 的 `App` / `Context<T>` / `AsyncApp`；同时收到两个上下文的回调以
  `cx` 接 GPUI 的那个，另一个按其所持内容命名（`trigger`、`state`）。
- **跨 seam 公共类型禁 `pub` 字段**：状态快照、能力集、渲染上下文、选项集
  一律 builder 构造 + 方法读取。全布尔类型 setter 用字段名、reader 用
  `is_<形容词>` / `has_<名词>`；含非布尔字段的类型 setter 加 `with_` 前缀、
  reader 保留字段名。字段即定义、不会增长的小值类型（`Point`、`Rect`、
  `Edges` 一类）豁免。本项目的 seam 指 `compositor-core` ↔ app 边界与
  app 内可复用 UI 模块的边界。
- **core 不依赖 gpui**：引擎（文档模型、渲染、工具、格式）全部位于
  `compositor-core`；gpui / gpui-kit 只出现在 app crate。
- 与原版 Swift 命名保持对照关系，术语以 [`CONTEXT.md`](CONTEXT.md) 为准。

## 6. 测试规则

见 [`docs/standards/COMPONENT_TEST_RULES.md`](docs/standards/COMPONENT_TEST_RULES.md)。
要点：移植的 XCTest 是验收标准；简单优先；构造覆盖测试；复杂逻辑测试；
合并相关断言；反模式清单；core 纯逻辑用 `#[test]`，app UI 行为用
`#[gpui::test]`。移植测试优先于新造测试。

## 7. PR / 提交规范

- **一个 PR / 一组提交只做一件事**，diff 最小化。
- 提交标题匹配仓库既有风格（本仓库使用小写 conventional 风格：
  `fix(core): …`、`feat(app): …`、`docs: …`、`chore: …`）。
- AI 生成的代码须重构到项目风格后再提交，并在提交说明中标注 AI 参与。
- 涉及 `compositor-core` 公共 API 变化时，在 PR 描述中列出新增 / 变更项及
  一句话用途。

## 8. 规范文件体系与适用性清单

### 文件体系

```
AGENTS.md                       ← 本文件，唯一权威入口（L1）
CLAUDE.md                       ← @AGENTS.md 指针，勿在此写规则
CONTEXT.md                      ← 领域术语表
docs/standards/                 ← 规范性文档（canonical 副本 + 改编稿）
docs/reference/gpui-kit/        ← 参考文档（框架教程/基础层/组件，只读 vendor）
skills/gpui-kit/                ← vendored skill（L2，组件目录 + 机制参考）
skills/gpui-kit-design-guides/  ← vendored skill（L2，设计指南 + 底线）
```

- `docs/standards/` 与 `docs/reference/gpui-kit/` 中的上游副本**禁改**；
  上游更新后从 `D:\Project\rust\gpui-component`（或上游仓库）重拷对应文件，
  `git diff` 应为零差异。适用性裁剪只发生在本文件与
  [`docs/standards/README.md`](docs/standards/README.md)。
- 逐字副本受 CC BY 4.0 授权（署名见 `docs/standards/README.md` 与
  `docs/reference/gpui-kit/README.md`）。

### 适用性清单（与上游正文冲突时以本清单为准）

| 上游条款 | 裁定 |
| --- | --- |
| gpui-base / component / kit 三层 crate 分层、"不许改 gpui-base" | 思想适用，边界映射为 `compositor-core`（不依赖 gpui）↔ app（gpui-kit UI）；上游 crate 是外部依赖 |
| 100% 向后兼容、公共 API 稳定性、semver/发布纪律 | **不适用**（应用，无下游消费者） |
| `RenderOnce` vs `Entity<T>`、状态所有权、`ElementId`、事件/动作/焦点、异步、布局滚动、大列表 | 适用 |
| 主题语义 token | 适用：禁裸 hex / `rgb()` |
| 组件公共 API 设计（builder + reader、禁 `pub` 字段） | 改编适用：seam = core↔app 与 app 内可复用 UI 模块 |
| i18n（en/zh-CN/zh-HK 维护、rust-i18n） | **暂不适用**：UI 文案跟随原版英文；中文术语规则仅用于中文文档 |
| 文档双语同步、website 站点构建 | 不适用：副本单语 vendor，更新走重拷 |
| 无障碍 | 适用：键盘可达性按桌面惯例执行 |
| 上游 PR 标题风格（"匹配既有 PR 标题"） | 适用，具体风格见第 7 节 |
| 性能（改渲染代码须 profile） | 适用：见第 3 节 |
