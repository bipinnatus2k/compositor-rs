# gpui-kit 参考文档（vendored 副本）

本目录是从 [GPUI Kit](https://github.com/longbridge/gpui-component)（官网
<https://gpui-kit.com>）仓库 `website/` 下复制的**逐字英文参考副本**，供本项目
的 agent 与开发者就地查阅，不必跨仓库。

| 子目录 | 来源 | 内容 |
| --- | --- | --- |
| `docs/` | `website/docs/` | 41 篇 GPUI 框架机制教程（entity / element / event / render / animation / window / keybinding / test …） |
| `base/` | `website/base/` | 49 篇 gpui-base 无样式基础层文档 |
| `component/` | `website/component/` | 79 篇样式组件用法文档（button / dialog / dock / data-table …） |

未复制 `shell/`（gpui-shell 是 JS 扩展运行时，本项目不使用）与 `zh-CN/`
（中文版；需要中文时查上游仓库 `website/zh-CN/`）。

## 约束

- **本目录是只读 vendor 副本，禁止直接编辑。** 更新方式：从上游仓库对应路径
  重新复制整目录覆盖，再用 `git diff` 审阅。
- 文内相对链接指向上游文档站结构，在本仓库内可能无法解析；
  `{{gpui_pre_version}}` 等模板占位符原样保留。
- 其中 `docs/design-guides.md` 与 `docs/coding-guides.md` 是规范性文件，
  入口与适用性裁定见 [`../../standards/`](../../standards/README.md)。

## 许可与署名

GPUI Kit 的文档散文与插图以
[CC BY 4.0](https://creativecommons.org/licenses/by/4.0/) 授权（其软件本身为
Apache-2.0，见上游 `LICENSE-DOCS.md`）。本目录为逐字复制、未作修改，特此署名：
文档来自 GPUI Kit 项目，<https://gpui-kit.com>（源码仓库
`longbridge/gpui-component`）。
