# Univer OSS 验证宿主

当前 CsvGrid 冻结范围回归也复用此宿主。生产构建及预览启动后，运行 `bun verify-frozen.ts <browserUrl> <targetId>` 和 `bun baseline-window.ts <browserUrl> <targetId>`，默认使用 14922 预览端口；第三个参数可指定开发宿主 `http://127.0.0.1:14921`。见[修复说明](../../docs/product/csv-frozen-range-compatibility.md)。

独立 Bun 项目，仅用于选型验证。固定 Univer OSS 0.25.1，以插件模式接入；不修改 Locus 正式编辑器、根依赖或 CSV/YAML 协议。

从本目录运行：

```powershell
bun install --frozen-lockfile
bun run dev
```

在仓库根目录启动本次专用实例：

```powershell
bun run locus:test:app -- --skip-onboarding
```

保存输出的 `LOCUS_RUNTIME_JSON`。按根 `AGENTS.md` 的 `/json/list` 发现流程确认本次隔离实例的调试端口，并先读取 `/json/version`，再读取 `/json/list`，记录主页面的 `id`。不要把正式 Locus 或其他 Agent 的目标传给下列命令。

回到本目录，使用实际发现的端口和目标 ID：

```powershell
bun cdp.ts http://127.0.0.1:<port> <target-id> navigate http://127.0.0.1:14921/
bun run verify http://127.0.0.1:<port> <target-id>
bun run verify:windows http://127.0.0.1:<port> <target-id>
bun run build
bun run audit
```

生产构建验证可运行 `bun ../../node_modules/vite/bin/vite.js preview --host 127.0.0.1 --port 14922 --strictPort`，将同一隔离页面导航到 `http://127.0.0.1:14922/`，再执行验证。

结果写入忽略版本控制的 `results/`，包括 JSON 和截图。`verify` 保留“原生合并/取消合并能否保留覆盖值”的断言，因此当前版本会报告一项失败并退出 1；紧接着的只修改视图的 mutation 验证应通过。不要将这一项删掉来宣称原生行为兼容。`verify:windows` 分别记录搬移共享 DOM 与独立窗口运行时的行为，报告中的 `passed` 才是判定依据。

验证中的中文输入使用 Chromium CDP composition，未覆盖操作系统候选窗口；粘贴采用浏览器 ClipboardEvent，不覆盖 Windows 系统剪贴板。复制验证覆盖库生成的 HTML 表格数据。未把示例中的单格 `commitCell` 当作完整的保存、冲突和撤销适配器。

测试专用窗口沿用 Locus 的 `about:blank#locus-shared-workbench-workbench-*` 原生入口，并在结束后关闭本次创建的窗口。退出验证后，应只停止本次创建的进程；保留日志与结果，目录清理遵守根 `AGENTS.md` 的隔离实例规则。

## 与当前 CsvGrid / Tabulator 对照

`baseline.html` 直接引用当前仓库的 `CsvGrid.vue`，没有复制或简化网格实现。生产构建包含两个独立入口，浏览器不会同时加载两套网格。

启动 14922 的生产预览和隔离 Locus 后运行：

```powershell
bun run compare http://127.0.0.1:<port> <target-id>
bun run compare:summary
bun run compare:window http://127.0.0.1:<port> <target-id>
```

也可在 `compare` 参数后追加行数、列数，只测一个形状；原始结果另存为 `comparison-<rows>x<columns>.json`。汇总脚本会合并这些文件，请勿重复放入同一组采样结果。

对照会导航指定的隔离页面，并固定 1400×900 视口。每个形状三轮，交替运行两种引擎；每轮使用新页面实例，记录建表、20 次滚动、5 次包含共同 CSV 数据更新的单格修改，以及 GC 后 JS 堆。基准不包含文件 I/O、完整编辑器外壳、冲突处理和 Locus 外层撤销历史。

多入口构建后 `audit` 的产物总数包含两个宿主；按引擎实际加载的 JS/CSS 体积见 `comparison-summary.json`，不能用全部产物相加代替单引擎体积。
