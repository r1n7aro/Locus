# Univer OSS 实测报告

日期：2026-09-16。结论：**单窗口 CSV 工作表方案可行，不能直接替换现有 CsvGrid。** 核心交互已在 Locus WebView2 中验证；正式迁移需要文本类型保护、仅修改视图的合并适配，以及浮动窗口的独立运行时与状态传递。

本次未更换正式编辑器，也未修改根 `package.json` / `bun.lock`。验证工程位于 `scripts/univer-oss-validation/`，包含独立依赖锁、宿主、CDP 驱动、窗口测试和依赖审计。界面采用现有满幅工作表布局，关闭 Univer 工具栏、公式栏及工作表底栏，未引入新的产品页面或装饰控件。

## 环境与许可

- Univer：npm `latest` 为 `0.25.1`，本次固定该版本；未采用 `1.0.0-rc.0`。
- Windows + 隔离 Locus `bun run locus:test:app -- --skip-onboarding`，WebView2 `Edg/153.0.4234.32`。
- 测量主窗口视口为 1400 × 900；网格高约 865px。功能与性能结果来自 Vite 生产构建的本地预览页，不是 jsdom。
- 隔离根目录：`<runtime-root>\locus-app-test-<run-id>`。正式 Locus 保持运行，测试未读写正式会话库。
- 采用逐个注册 OSS 插件的方式；没有安装 `@univerjs/presets` 通用聚合包。
- 实际依赖审计：137 个包，MIT 91、Apache-2.0 25、BSD-3-Clause 11、ISC 9、0BSD 1；没有 `@univerjs-pro/*`。`@univerjs/telemetry` 的 package.json 缺少 license 字段，包内 LICENSE 与源码头明确为 Apache-2.0，该包仅提供服务标识符。
- 上述授权适合继续按项目 GPLv3 组合分发，正式集成时仍需纳入仓库现有第三方许可清单与声明。

## 功能验证结果

主验证脚本 **24 项：23 项通过，1 项原生行为不符合 Locus 约定**。窗口专项另有 2 项：独立运行时通过，共享 DOM 搬移未通过。失败项保留在报告与脚本中，没有屏蔽。

| 场景 | 结果 | 证据与边界 |
| --- | --- | --- |
| 无冻结、冻结 1 列、冻结 3 列后横向滚动并拖选 | 通过 | 原生 CDP 鼠标事件；A2 到 L4 的选择覆盖中间不可见列 |
| Shift + 方向键跨冻结边界扩选 | 通过 | C2 扩展到 D2 |
| 跨冻结区复制数据 | 通过 | 库生成的 HTML 表格包含 3 行 × 12 列，包括不可见中间列 |
| 输入前导零、以 `=` 开头的字符串 | 适配后通过 | `000123` 与 `=1+1` 保持字符串；空白格必须有显式单元格文本格式 |
| 中文组合输入 | 通过 | Chromium CDP composition 提交“中文输入验证”，无重复字符；未测试 Windows 候选窗口 |
| 多行编辑 | 适配后通过 | Alt+Enter；需读取富文本 dataStream，并将段落 CR 转成 CSV 编辑器使用的换行 |
| 矩形粘贴 | 适配后通过 | 浏览器 ClipboardEvent 的 2×2 TSV 保留前导零；未测试 OS 剪贴板与 Excel HTML 格式粘贴 |
| 原生单格编辑撤销/重做 | 通过 | 数据回滚与重新应用正确 |
| 合并区域跨冻结边界并点击 | 通过 | B2:D3 可渲染与选中 |
| 原生合并后取消合并保留覆盖值 | **未通过** | 默认 merge 命令清空非左上角值，取消合并后仍为空 |
| 仅改变视图的 merge mutation | 通过 | 通过公开导出的 mutation 修改合并几何，保留覆盖值；编辑锚点后取消合并，其他值仍在 |
| 隐藏/显示合并区内的列 | 通过 | 冻结与合并配置保持有效 |
| 缩放、指定行高、面板改宽与临时隐藏 | 通过 | 125% 缩放、64px 行高；宽度 1400→700→1400，选择状态保留 |
| 销毁并重新创建实例 | 通过 | 快照恢复数据、冻结、合并、缩放和选择 |
| CSV 原文往返与单格保存 | 通过 | 复用 Locus 解析器；BOM、CRLF、引号、长整数、日期形文本、公式形文本和末尾空字段保持，单格修改只改目标字段 |

### 接入时必须处理的三个问题

**1. 文本格式必须作用于实际可编辑单元格。**

在工作表 `defaultStyle` 上设置 `@`，虽然显示层能读出文本格式，但 0.25.1 的输入/粘贴代码读取的是单元格自身样式。首次测试中，空白格输入 `000123` 被转换成数值 123，`=1+1` 被识别为公式，TSV `007` 被转换成数值 7。给有限编辑区域的空白格也设置显式 `s: "plain"` 后，同一组真实输入测试通过。生产适配必须覆盖新增行列及各种粘贴入口，不能只在首次加载时设置。

关闭公式栏不等于可以省略公式 UI 插件：0.25.1 的普通单元格编辑器由 `UniverSheetsFormulaUIPlugin` 提供。去掉该 OSS 插件时能够显示表格，但没有完整单元格键盘编辑器。

**2. 合并只能改变 Locus 的视图模型。**

使用原生 `range.merge()` 不符合现有 CSV 语义。验证中改用 `AddWorksheetMergeMutation` / `RemoveWorksheetMergeMutation`，只调整合并几何，成功保留覆盖值。这些是公开导出的低层 mutation，但不会代替 Locus 的业务撤销历史；正式集成应把右键菜单、快捷键和视图保存统一路由到 Locus 的命令，再更新网格投影。不能直接导出 Univer 全表覆盖 CSV。

**3. 浮动窗口需要独立运行时。**

采用与 Locus 一样的原生 `about:blank#locus-shared-workbench-workbench-*` 子窗口，将现有网格 DOM 搬过去并同步样式：点击选格、文字输入可以发生，但 Enter 未提交到单元格，原值仍为 `2:0`。已记录子窗口收到可信 F2 与 Enter 事件，并保留未提交编辑器截图，因此不是未发送按键。

随后让同一子窗口加载自己的 JavaScript 运行时，通过快照创建 Univer：编辑后的 `2:0standalone-window-edit` 正确提交，光标移到下一行，冻结状态保留。**可行的是“每个窗口创建自己的实例 + 数据/视图快照或增量消息”，不是直接搬移已有实例的 DOM。** 本次只验证了快照进入子窗口和编辑提交，没有实现工作台的完整双向状态同步、草稿冲突与原生窗口关闭协议。

## 性能与体积

后续已补充同环境、同 CSV 数据更新路径的三轮 [Tabulator / Univer 对照](./csv-grid-comparison.md)。下列数字保留为本轮单独验证记录，做引擎间选型比较时请使用对照报告。

以下为本机单次生产构建样本。数据是字符串，冻结 2 列；加载包含 Locus 解析、投影、Univer 生命周期 Rendered 及两次 requestAnimationFrame。滚动与修改也等待两帧，数值不是纯计算耗时或 FPS。不与旧方案不同环境的历史数字直接比较。

| 样本 | 解析/投影到可渲染 | 单格修改到两帧后 | 20 次跳转滚动中位数 / P95 |
| --- | ---: | ---: | ---: |
| 5000 行 × 20 列，10 万格 | 407 ms | 48 ms | 49 / 51 ms |
| 100 行 × 1000 列，10 万格 | 416 ms | 60 ms | 66 / 103 ms |

- 10 万格场景保持约 48 个 DOM 元素、2 个 Canvas，不随单元格数量线性增加 DOM。
- 连续三次重建 5000×20 样本并执行 GC，页面 JS 堆约 41.7、41.8、42.4 MB；这是短程观测，不能代替长时间泄漏测试，也不是整个 WebView2 进程内存。
- 本次最小验证宿主构建：主 JS 约 5.51 MB，gzip 约 1.58 MB；全部产物约 10.17 MB，gzip 合计约 2.67 MB，包含动态语言/断词数据。不是 Locus 正式包的净增量，正式集成需要按需加载和打包分析。
- 浏览器记录无未捕获 JavaScript 异常、无冻结/选区警告；唯一网络错误为验证宿主缺少 `favicon.ico` 的 404。

## 建议

可以继续做 Univer 的正式适配原型，但不要立即替换所有现有 CSV 页面。先固定独立窗口运行时方案，保留 Locus CSV/YAML 与文档会话作为唯一数据来源，统一编辑、合并与撤销的命令入口。完成结构编辑、列重排、排序/筛选的源坐标映射、跨窗口双向同步、冲突保存、Excel HTML 粘贴和真实 Windows IME 验收后，再决定上线迁移。

若必须保持现有共享 DOM 浮动窗口架构且不增加独立运行时，当前验证结果不足以支持替换。之前“优先验证 Univer OSS”的建议应收敛为这个有条件结论。

## 复现与证据

- [验证工程说明](../../scripts/univer-oss-validation/README.md)
- 本地原始记录：`scripts/univer-oss-validation/results/report.json`、`shared-window.json`、`retained-memory.json`、`dependencies-and-bundle.json`。
- 截图：同目录 `frozen-1.png`、`frozen-3.png`、`merged-frozen.png`、`shared-window.png`、`standalone-window.png`。
- `bun run build` 与独立 TypeScript 检查通过。`bun run verify` 当前按设计退出 1，原因是保留原生破坏性合并的兼容性断言；其他主验证项通过。
- 功能 API 与配置以安装的 0.25.1 类型和源码为准，避免混用官网当前 RC 文档。

官方资料：[Univer 安装与插件模式](https://docs.univer.ai/guides/sheets/getting-started/installation)、[冻结](https://docs.univer.ai/guides/sheets/features/core/freeze)、[范围与选区](https://docs.univer.ai/guides/sheets/features/core/range-selection)、[源码与许可证](https://github.com/dream-num/univer)。
