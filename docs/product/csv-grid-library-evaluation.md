# CSV 网格库重新评估

日期：2026-09-16。约束：Locus 的 `package.json` 声明 `GPL-3.0-or-later`，依赖必须允许按项目 GPL 许可组合分发，同时满足用户要求的免费分发。

GPL 允许商业使用和收费分发，因此选型不能用“我们是开源项目”代替上游许可检查，也不能将“非商业用途免费”视为 GPL 兼容。MIT 与 Apache-2.0 可以用于 GPLv3 项目，分发时保留依赖要求的许可证、版权及适用的 NOTICE。依赖无需与项目使用完全相同的许可证。

结论：保留 Locus CSV 无损文档模型、Papa Parse、YAML 视图文件与公共文件会话。现有 Tabulator 适配已超出简单 CSV 数据网格的范围，不宜继续扩展为工作表内核。优先验证 Univer OSS，Jspreadsheet CE 作为较轻的备选；验证完成前不替换现有实现。

本评估最初依据工作区源代码及官方资料。2026-09-16 已完成 Univer OSS 0.25.1 的隔离 Locus / WebView2 验证，见 [Univer OSS 实测报告](./univer-oss-validation.md)：单窗口交互可行，但默认合并语义与共享 DOM 浮动窗口不直接兼容；必须加入文本保护、视图合并和独立窗口运行时适配。

## 当前组合的边界

- `src/document/csv/csvDocument.ts` 保留字段原始写法、分隔符与记录结束符；Papa Parse 用于方言探测与剪贴板 TSV。这个分工应保留，不能改为网格整体导出后覆盖 CSV。
- `src/document/csv/csvView.ts` 将列宽、排序、隐藏、冻结、样式与合并信息保存在 YAML；视图不应改变原始数据。这一边界也应保留。
- `src/components/csv/CsvGrid.vue` 同时设置数据列 `frozen` 与 `selectableRange`。Tabulator 6.5.2 的 SelectRange 模块明确检查并警告该组合，仅冻结行号列不触发这个限制。
- `csvHorizontalRenderer.ts` 直接继承 Tabulator 内部虚拟渲染器；`csvMergeLayer.ts` 自行计算合并区域、冻结分界与可见区域裁剪；网格还接管键盘导航、剪贴板和缩放。后续每个新功能都需要验证这些机制的组合，成本高于单项功能接入。
- 原 `csv-document-editor-plan.md` 的首版边界排除了合并单元格。当前已经实现合并、样式和缩放，原来的库选型结论不能直接沿用到扩大的范围。

截图中的冻结警告来自实际能力边界，屏蔽 `console.warn` 不会修复选区定位。另一个 `workspace.pane_context_unavailable` 属于工作区恢复生命周期，与表格库无关。

## 候选对比

| 候选 | 授权及官方能力 | 对 Locus 的判断 |
| --- | --- | --- |
| Tabulator | MIT；数据网格、编辑、范围选择、冻结列，但当前版本对冻结数据列与范围选择组合发出警告 | 简单数据表仍适用；当前工作表需求下，继续接管内部选择与渲染机制的维护成本偏高 |
| AG Grid Community / Enterprise | Community 为 MIT，与 GPLv3 兼容；跨固定区范围选择有官方支持，但 Cell Selection 属于 Enterprise | Community 无法直接覆盖现有核心交互；不能将 Enterprise 的专有授权当作 GPL 兼容依赖 |
| Handsontable | 工作表交互、冻结与合并；当前免费许可限制为非商业用途或商业评估 | 当前非商业许可不适合作为 GPL 组合分发的依据；开源项目身份不会自动消除这些限制，也不能把旧 MIT 版本当作当前版本的授权 |
| Jspreadsheet CE | MIT；Vue 接入、范围选择、冻结和合并 | 较轻备选，但 CE 冻结列与筛选/页脚有限制，交互式冻结控制列在 Pro 中，不能假设 CE 等同于 Pro |
| Glide Data Grid | MIT；Canvas、React、编辑与选区；用于 Glide 自身 Data Editor | 数据网格候选，但 Vue 项目需引入 React 接入层；不能仅凭“merged cells”宣传认定满足任意二维合并 |
| Univer OSS | Apache-2.0，与 GPLv3 兼容；Canvas 渲染、工作表、选区、冻结、格式与可扩展插件，支持嵌入 Vue | 当前功能方向下优先验证；接入较重，需要限制为单工作表、必要插件及现有桌面工具界面 |

Univer 的 OSS 与 Pro 必须分开评估。协作、导入导出、打印、图表等能力有 Pro 边界；Locus 可继续通过自己的 CSV/YAML 模型读写本地文件，不依赖商业导入导出包。其界面层本身使用 React，Vue 接入不代表没有 React 运行时成本。

## 替换前的验收

验证应针对“组合是否工作”，不能再只勾选功能列表：

1. 冻结 1 列/多列后横向滚动，跨冻结边界拖选、Shift 扩选、复制、粘贴和键盘导航。
2. 合并区域跨冻结分界，隐藏或重排列后继续编辑；取消合并仍保留被覆盖的 CSV 源值。
3. 中文输入法、多行文本、连续编辑、Tab/Enter 导航、缩放与可变行高。
4. 10 万单元格及宽表的首屏、持续滚动、局部更新、内存与多标签切换，按同一 WebView2 环境比较。
5. 分栏、独立窗口、关闭后恢复光标与滚动位置；外部修改与冲突处理仍经过公共文件会话。
6. `001`、长整数、日期形字符串、以 `=` 开头的文本、BOM、CRLF、引号与末尾空字段无损保存；网格不得自动把 CSV 字符串变成数字或公式。
7. 撤销历史只保留一个权威来源；排序和筛选保持视图语义，不隐式重排源 CSV；网格回调与 Locus 文档更新不会循环触发。

只替换网格适配器，继续沿用当前主题 token、右键菜单、工作台标签及 CSV/YAML 协议。候选未通过组合验证前，不承诺迁移后即可消除所有交互问题。

## 官方资料

- [GNU 许可兼容性列表](https://www.gnu.org/licenses/license-list.en.html)
- [Apache-2.0 可以用于 GPLv3 项目](https://www.apache.org/licenses/GPL-compatibility)
- [GNU 对收费分发的说明](https://www.gnu.org/philosophy/selling.en.html)
- [Handsontable 当前非商业许可原文](https://handsontable.com/static/licenses/non-commercial/v4/handsontable-non-commercial-license.pdf)
- [Tabulator 6.5.2 SelectRange 源码](https://github.com/tabulator-tables/tabulator/blob/6.5.2/src/js/modules/SelectRange/SelectRange.js)
- [AG Grid Cell Selection 与固定区域](https://www.ag-grid.com/javascript-data-grid/cell-selection/)
- [AG Grid 开源与商业版本](https://github.com/ag-grid/ag-grid)
- [Handsontable 授权](https://handsontable.com/docs/15.2/javascript-data-grid/software-license/)
- [Handsontable 合并对源数据的影响](https://handsontable.com/docs/javascript-data-grid/merge-cells/)
- [Jspreadsheet CE 仓库与 MIT 授权](https://github.com/jspreadsheet/ce)
- [Jspreadsheet CE 冻结列与限制](https://bossanova.uk/jspreadsheet/docs/freeze-columns)
- [Jspreadsheet CE 合并](https://bossanova.uk/jspreadsheet/docs/merged-cells)
- [Glide Data Grid 仓库](https://github.com/glideapps/glide-data-grid)
- [Univer 架构、授权与 OSS/Pro 边界](https://github.com/dream-num/univer)
- [Univer 冻结](https://docs.univer.ai/guides/sheets/features/core/freeze)
- [Univer 范围与选区](https://docs.univer.ai/guides/sheets/features/core/range-selection)
