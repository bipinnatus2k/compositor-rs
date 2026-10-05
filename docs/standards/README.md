# 开发规范（standards）

本目录存放本项目的**规范性文档**：从上游 GPUI Kit 复制的 canonical 指南与
本项目改编的规则。AGENTS.md 是这些规范的总入口与适用性裁定；本 README 说明
副本来源、更新方式与逐条适用性。

## 文件清单

| 文件 | 来源 | 地位 |
| --- | --- | --- |
| [`design-guides.md`](design-guides.md) | gpui-component `website/docs/design-guides.md` 逐字副本 | 规范性设计指南：任务优先、视觉语言、布局、交互状态、覆盖层、动效、数据密集界面、文案、i18n、AI 生成界面准则、设计评审 checklist |
| [`coding-guides.md`](coding-guides.md) | gpui-component `website/docs/coding-guides.md` 逐字副本 | 规范性编码指南：crate 分层、状态所有权、`RenderOnce` vs `Entity<T>`、`ElementId`、渲染组合、事件/动作/焦点、异步、布局滚动、命名、测试策略、agent 编码规则、实现 checklist |
| [`COMPONENT_TEST_RULES.md`](COMPONENT_TEST_RULES.md) | 改编自 gpui-component `.agents/COMPONENT_TEST_RULES.md` | 本项目测试规则（已按 compositor 的移植验收模式改写） |

两份逐字副本受 [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/)
授权（GPUI Kit 文档许可，软件本身为 Apache-2.0）。特此署名：文档来自
GPUI Kit 项目，<https://gpui-kit.com>（源码仓库
`longbridge/gpui-component`），未作修改。

## 约束

- **禁改逐字副本**：`design-guides.md` / `coding-guides.md` 必须与上游
  `website/docs/` 对应文件逐字一致（本项目的裁剪通过下方适用性清单与
  AGENTS.md 完成，不通过删改正文完成）。
- **更新方式**：从上游仓库重新复制对应文件覆盖，`git diff` 应显示为零差异。
- 参考类文档（框架教程 / 基础层 / 组件用法）在
  [`../reference/gpui-kit/`](../reference/gpui-kit/)，同为只读 vendor。

## 适用性清单

上游指南为组件库视角编写。本项目是**应用**（图像编辑器），逐条裁定如下；
与正文冲突时以本清单为准。

### coding-guides.md

| 上游条款 | 裁定 |
| --- | --- |
| gpui-base / gpui-component / gpui-kit crate 分层与"不许改 gpui-base" | 分层**思想**适用，但映射到本项目边界：`compositor-core`（引擎，不依赖 gpui）↔ `compositor` app（gpui-kit UI）。上游 crate 本身是外部依赖 |
| 100% 向后兼容、公共 API 稳定性、semver/发布纪律 | **不适用**：本项目不是库，无下游消费者 |
| `RenderOnce` vs `Entity<T>` 选型、状态所有权 | 适用 |
| `ElementId` 稳定标识、渲染组合 | 适用 |
| 事件 / 动作 / 焦点、异步、布局与滚动、大列表 | 适用 |
| 主题与样式（语义 token） | 适用：颜色/间距/圆角一律走 gpui-kit 主题 token，禁裸 hex |
| 组件公共 API 设计（builder + reader、跨 seam 禁 `pub` 字段） | 改编适用：seam 指本项目的 core↔app 边界与 app 内可复用 UI 模块，不承担库级兼容义务 |
| 文件与命名约定 | 适用（另见 AGENTS.md 第 5 节） |
| 测试策略 | 适用，并与 [`COMPONENT_TEST_RULES.md`](COMPONENT_TEST_RULES.md) 合并执行 |

### design-guides.md

| 上游条款 | 裁定 |
| --- | --- |
| 设计论点 / 任务优先 / 视觉语言 / 布局 / 交互状态 / 覆盖层 / 动效 / 数据密集界面 / 文案 | **全部适用** |
| 无障碍与设计评审 checklist | 适用（键盘可达性按桌面惯例执行） |
| Internationalization（多语言维护） | **暂不适用**：本项目 UI 文案跟随原版 Compositor 的英文；其中文术语规则仅在撰写中文文档时参考 |
| Guidance for AI-generated interfaces | 适用 |
