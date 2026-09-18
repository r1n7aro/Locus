# CSV 冻结列与范围选择修复

日期：2026-09-16。保留 Tabulator 6.5.2 与现有 CSV 数据、剪贴板、历史和视图模型。

## 原因与改动

Tabulator 的横向虚拟渲染窗口只索引非冻结列，但原生范围边框用该窗口裁剪完整列坐标。范围在冻结区或跨冻结区时，滚动后的边框可能消失或错位。原生自动滚动也只避让行号栏，没有避让整个冻结前缀。

- `csvRangeSelection.ts` 在当前 CsvGrid 实例构建前适配范围模块，保留原有选区模型、事件和公开 API。根据完整可见列宽计算坐标，将范围边框分别裁剪到冻结区和滚动区，使用当前窗口的绘制帧更新位置。
- 普通方向键和 Tabulator 的 Ctrl 跳转统一避让全部冻结列；移动到冻结单元格时不改变水平滚动位置。
- 隐藏列仍保留冻结前缀成员身份，避免 Tabulator 遇到隐藏的非冻结定义后把后续冻结列错误地当成右侧冻结列。
- 表格 DOM 移入共享浮动窗口后，鼠标松开和窗口失焦监听跟随当前所属文档，防止松手后仍继续扩选。
- 边框层显示在冻结单元格与合并层上方，复用现有 `--accent-color` 和选中样式；不新增界面文案或控件。

适配范围是 Locus 当前的平面、从左到右、连续左冻结列布局。不是全局关闭警告，也没有修改 `node_modules`；不支持的冻结布局仍保留上游诊断。适配依赖 Tabulator 6.5 内部范围接口，升级该库时需要重跑以下验证。

## 验证

相关 Vitest 共 **72 项通过**，其中新增 4 项回归覆盖冻结选区、键盘可见性、隐藏/重排/缩放以及浮动窗口鼠标松开。`bun run typecheck:test` 通过，实际 CsvGrid 的 Vite 生产验证入口构建通过。

隔离 Locus `<runtime-root>\locus-app-test-<run-id>`、WebView2 `153.0.4234.32` 中，用生产构建执行 **19 项检查**：

- 冻结 0、1、3 列时，水平滚动后的鼠标正反向拖选与复制内容。
- 边框几何与冻结层的实际绘制顺序，垂直滚动移出视口后的隐藏。
- Shift 扩选、Ctrl+Shift 跳转及目标列可见性。
- 跨冻结区粘贴、删除，前导零、长整数、公式形文本保持字符串。
- 隐藏首列、修改列宽与 150% 缩放后的冻结位置及选区对齐。
- 没有冻结/范围选择兼容警告或未捕获异常。

另外用真实共享浮动窗口验证 DOM 搬移、F2 编辑与 Enter 提交、跨冻结区拖选、松手后移动鼠标不再改变选区，结果通过。鼠标和键盘通过 CDP 原生输入派发；剪贴板数据通过页面 ClipboardEvent 注入，不读写系统剪贴板。

## 复现

按[验证工程说明](../../scripts/univer-oss-validation/README.md)启动隔离实例与生产预览服务，明确选择测试实例的 CDP 地址和页面 ID，然后在验证工程目录运行：

```powershell
bun verify-frozen.ts <browserUrl> <targetId>
bun baseline-window.ts <browserUrl> <targetId>
```

开发宿主可加第三个参数 `http://127.0.0.1:14921`；默认生产预览为 `http://127.0.0.1:14922`。

本地证据保存在验证工程的 `results/tabulator-frozen-selection.json`、`results/tabulator-frozen-selection.png` 与 `results/tabulator-shared-window.json`。结果目录由 `.gitignore` 排除。
