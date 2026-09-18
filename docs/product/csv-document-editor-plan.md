# CSV 文档编辑与公共文档基础设施计划

日期：2026-09-15

状态：CSV 编辑器、YAML 视图协议与知识目录接入已实施。根据实际试用反馈，界面改为满幅工作表，取消常驻工具栏，操作集中到右键菜单；WebView2 输入与文件保存验收通过。完整应用类型检查仍有一个本次范围外的错误，真实 Unity Editor 重新导入尚未实测。

## 本次验证结果

2026-09-15 当前共享工作区验证：

| 检查 | 结果 |
| --- | --- |
| CSV、公共基础设施、工作区文件、知识目录、工作台路由 Vitest | 12 个文件、223 项测试通过 |
| CSV Rust 单元测试 | 5 项通过：原文保存/知识扫描、视图协议、配套重命名/复制与 Unity GUID |
| WebView2 网格实际输入 | 15 项通过：直接键入、中文 IME、Enter/Tab、区域选择、粘贴、撤销重做、额外输入框边框检查 |
| WebView2 完整编辑器与原生文件读写 | 10 项通过：空文件、右键插入、保存后撤销、源码往返、独立 YAML 保存 |
| `bun run docs:validate` | 通过 |
| `bun run vite build` | 通过；现有大 chunk 提示仍存在 |
| `bun run typecheck` / 完整构建的类型检查前置步骤 | `src/services/sharedWorkbenchWindow.ts:54`：`number` 不能赋给 `Timeout`；该文件未由本任务修改 |
| `bun run typecheck:test` | 通过 |
| 本次修改文件的 `git diff --check` | 通过 |

实测使用 Windows 的隔离 Locus / WebView2 开发实例（Vite，未压缩，1100 × 700 网格），fixture 位于该实例的工作区和 `Assets/CsvTests`；保留正式 Locus 与用户的其他工作区。网格性能样本为 10 列字符串、CRLF：1 万单元格解析约 2 ms，解析加刷新约 327 ms；10 万单元格解析约 18 ms，解析加刷新约 678 ms。滚动到底部时分别渲染 58 / 80 行（1566 / 2160 个含行号的单元格），空白列也计入渲染。这是本机单次开发实例数据，不是硬件无关的性能保证。

独立 Vite 打包通过不等同于完整构建通过。Unity 范围的当前验证覆盖普通 `Assets` 文件读写、配套 `.meta` 保留/复制规则；未把它当作已完成真实 Editor 重导入验收。

## 范围与已确认决策

- 同时支持普通工作区文件、Locus 文档/知识目录和 Unity `Assets` 中的 CSV。
- `items.csv` 是唯一的单元格数据源；同目录的 `items.csv.view` 保存视图配置，内容采用 YAML。
- 不使用 `items.csv.meta` 保存 Locus 视图配置。Unity 的资源 `.meta` 文件保持由 Unity 管理；`items.csv.view` 被 Unity 导入后也可能拥有自己的 `.meta`。
- 首版是一份 CSV 对应一张数据表，集成在现有工作台标签页中，支持分栏与浮动窗口。
- 用户随后授权完整实施；公共基础设施、CSV UI、CSV/YAML 编解码、知识库格式扩展均已接入。
- 不改变现有会话数据库结构；若后续扩展涉及持久化 schema，另行提供明确迁移与旧数据导出验证。

## 现有基础与接入范围

| 现有实现 | 当前能力或限制 | CSV 接入要求 |
| --- | --- | --- |
| `src/components/ui/BaseMarkdownEditor.vue` | CodeMirror 6，源码/实时预览、选区与撤销历史 | 保留为源码适配器，表格使用独立网格适配器 |
| `src/components/workbench/WorkspaceFilePreview.vue` | 文件读取、保存、冲突选择、位置跳转、跨窗口文本草稿 | 复用公共文件会话，增加 CSV 路由、网格视图状态传递 |
| `src/composables/useFileChangeRevalidation.ts` | 共享工作区文件事件、激活/焦点探测与防抖 | 同时关注 CSV 和 `.csv.view`，分别跟踪数据与视图版本 |
| `src/components/knowledge/knowledgeEditorWorkspaceSession.ts` | 知识草稿、冲突、字段与 Markdown EditorState 缓存 | 复用公共身份与缓存；保留知识业务保存策略 |
| `src/components/knowledge/knowledgeCollaborativeEditing.ts` | 有界文本 diff、三方 rebase、带唯一上下文的编辑操作 | 通用算法下沉，知识正文的空白规范化留在适配器 |
| `src-tauri/src/commands/workspace_explorer.rs` | 内容哈希校验、原子文件替换；当前 UTF-8 文本编辑上限 1 MiB | CSV 单独确定容量与编码策略；保留工作区权限、generation/materialization 校验 |
| `src/components/workbench/DevelopmentWorkbench.vue` | 资产中的 Markdown 已分流到文档编辑器 | CSV 从资产入口进入同一文档编辑链路 |
| `src-tauri/src/knowledge_store.rs` | 路径、扫描、frontmatter、目录移动/删除假定 Markdown | 增加文档格式维度并覆盖完整生命周期，不能仅放开扩展名 |

知识库的分类与文件格式应分离：例如 design/reference 是分类，markdown/csv 是格式。CSV 不能写入 Markdown frontmatter；首版标题可由文件名提供。知识管理属性需要由知识索引/目录配置的明确扩展承载，不放入 CSV 数据或视图配置。只读来源继续遵守现有约束。

## 表格库评估

以下候选比较来自官方文档与仓库代码调研；最终采用 Tabulator 6.5.2，已完成 Locus WebView2 实测。没有宣称对所有候选库做过运行时对比。

| 库 | 能力与授权 | 判断 |
| --- | --- | --- |
| Tabulator | MIT；单元格编辑、区域选择、剪贴板、交互历史、Spreadsheet 模式 | 优先验证；按需组合网格能力，适合 CSV 数据编辑 |
| Jspreadsheet CE | MIT；电子表格交互、Vue 封装、工作表撤销重做 | 主要备选；对比行列操作、输入法和样式适配成本 |
| Handsontable | Vue 3 支持；商业用途需对应付费许可 | 如接受授权成本，可进入对比 |
| RevoGrid | MIT 核心、Vue 与虚拟滚动；History、智能填充等在 Pro 列表 | 大表候选，先核清免费版与 Pro 边界 |

最终组合：Tabulator 6.5.2 + Papa Parse 5.7.0 + YAML 2.9.1 + Locus 文档模型。采用普通 Grid 模式，通过工作表投影层与自定义编辑器提供电子表格交互。

- 网格库只负责显示、选择、键盘和编辑交互，不能成为持久化数据的唯一来源。
- Papa Parse 负责分隔符探测和剪贴板 TSV 解析/输出；Locus 词法模型保留 CSV 每个字段的原始写法及记录结束符，只重新编码修改过的字段。
- 库的原生导出、空行裁剪、自动类型转换、公式处理必须经适配层约束。
- 列增删、批量粘贴作为单次撤销操作等能力，需要单独验证；不假设库的交互历史覆盖所有结构操作。
- Tabulator 的 Spreadsheet 模式不支持部分改变行列布局的模块，也不提供单元格公式计算；首版不需要公式，但仍须比较普通 Grid 模式与 Spreadsheet 模式的适配成本。
- 已锁定依赖版本。首版不引入多工作表、合并单元格、公式引擎、图表或 XLSX 往返保存。

## 编辑与界面

- CSV 默认显示表格；“表格 / 源码”切换放入右键菜单，两种模式共享同一草稿和保存基线。
- 列标使用 A、B、C…，行号从 1 开始；首条 CSV 记录仍位于第 1 行。初始至少投影 26 列、100 行（大表按总容量裁剪），接近边缘时继续扩展。投影中的空白格不写入 CSV，直接输入才扩展真实数据。
- 单击选择后直接键入替换内容，支持中文 IME；双击或 F2 编辑原内容，Enter/Tab 提交并移动，Shift 反向移动，Alt+Enter 输入换行，Escape 取消。编辑控件填满单元格，内部无额外框线。
- 支持单元格编辑、方向键/Tab 导航、区域选择、Excel 多格复制粘贴、行列增删、撤销重做、查找替换。
- 支持列宽、冻结列、隐藏列、自动换行及“首行为表头”；兼容无表头、空表头和重复表头。
- 排序、筛选、隐藏列与显示列顺序属于视图操作。保存 CSV 时仍包含所有记录和源顺序；真正重排数据是单独的数据操作。
- 数据保存沿用 Ctrl+S、标签页 dirty、关闭确认和外部冲突选择。
- 按用户要求删除整行工具栏。插入/删除行列、清空、剪贴板、撤销重做、排序、查找、源码与视图设置使用 `BaseContextMenu`；低频面板复用 `BaseButton`、`BaseCheckbox`。保留满幅网格、连续 surface 和全局颜色、边框、字体 token。
- 不增加说明性顶部卡片、弱语义 badge 或常驻推荐信息；格式/编码等低频选项进入二级菜单。
- 普通表格、知识入口和 Unity 资产入口使用同一编辑器核心，避免三套状态与保存实现。

## CSV 数据与编码

- 以字符串二维数组及记录边界为数据模型；内部行列 ID 不写入 CSV。
- 不自动转换 `00123`、长整数、日期、布尔字符串、空白或以 `=` 开头的原文，不执行公式。
- 保留分隔符、UTF-8 BOM、换行、引号内换行、空行、末尾空字段、末尾换行及不等长记录。
- 仅查看或调整视图不能重写 CSV。网格展示补齐的空单元格不能自动变成磁盘中的新字段或新行。
- 常规 parse/unparse 可能规范化引号，不能宣称字节无损。若要求单元格编辑仅产生局部 Git diff，适配层需保留原始记录/字段区间，只重写受影响部分；结构编辑另行确定保真边界。
- 通用文本换行工具服务现有源码编辑器，它不是 CSV 无损编解码器；CSV 的引号字段内部换行须由 CSV 适配层保留。
- 解析失败时保留原文和草稿，显示错误位置并提供源码修复，不静默补齐或覆盖文件。
- 默认先处理 UTF-8/UTF-8 BOM。GBK、UTF-16 等需要明确的解码与回写策略，不自动替换非法字符后保存。
- 128 KiB 以上的文件加载/源码解析使用 Worker；虚拟滚动限制 DOM。单元格内键入由本地 textarea 处理，只在提交时更新词法文档和撤销快照。当前上限为 UTF-8 16 MiB、50 万矩形单元格、1 万列，超过上限保留源码入口；未支持 GBK/UTF-16 回写。

## YAML 视图文件

`items.csv.view` 的示例结构：

```yaml
schema: locus.csv-view.v1
headerRows: 1
rowHeight: 28
wrapText: false
frozenColumns: 1

columns:
  c1:
    sourceIndex: 0
    header: "id"
    width: 100
  c2:
    sourceIndex: 1
    header: "name"
    width: 220

columnOrder:
  - c1
  - c2
```

- 列配置用稳定 ID 到配置的映射；ID 只在创建列时生成，不能每次保存重建。示例 ID 仅用于说明。
- 显示顺序单独存储，调整顺序不搬动整块列配置。`sourceIndex` 是源文件绑定位置，不代表显示顺序。
- ID 仅存在于视图侧，无法单独解决外部改表后的身份匹配。结合位置、表头与重复表头出现次序/结构指纹重新匹配；无表头或匹配有歧义时，仅受影响列恢复默认布局，不能错绑到另一列。
- 保存列宽、隐藏状态、冻结、换行、显示顺序等持久视图配置；选择区、光标、滚动位置、临时编辑状态保存在本机工作台会话。
- 缺少 `.view` 时采用默认布局，仅在首次实际调整视图时创建；CSV 单独存在仍完整可用。
- 使用简单 YAML 映射/序列/标量，固定字段顺序、两空格缩进、LF 换行和字符串转义；拒绝重复键、非法字段值和不支持的 schema。
- 固定序列化顺序，不写保存时间等高频字段，不把第三方库的私有状态对象直接落盘。
- 损坏、有 Git 冲突标记或版本不支持时保留原文件，可暂用默认视图；未经用户处理不能自动以默认配置覆盖它。
- 首版只有 v1；后续格式升级需要明确、可重复执行的迁移，不能由旧客户端静默降级重写。

Git 默认对 JSON/YAML 都进行文本三方合并，不会按列 ID 做语义合并。YAML 块式写法减少结构标点，使 diff 更易审查；稳定布局和数据结构比格式本身更重要。双方修改同一列宽、同时插列或首次各自创建 `.view` 仍可能冲突。首版不依赖自定义 Git merge driver，不用自动拼接代替冲突处理。

## 保存、冲突与文件生命周期

- CSV 数据和 `.view` 分别记录版本、dirty 和保存结果。
- 列宽等独立调整防抖保存；涉及尚未保存的列增删时，相关视图结构延迟到 CSV 保存成功后写入。
- 两个文件分别校验内容版本并原子替换，但两次替换不是跨文件原子事务。CSV 成功而 `.view` 失败时，保留待重试视图并明确结果，不能回滚或覆盖已确认的新数据。
- 并发文件变更使用现有工作区监听与版本机制。干净草稿刷新；存在本地修改时保留草稿并呈现冲突选择。
- 保存回包只更新发起资源；保存期间继续输入必须保留，只有当前草稿全部保存后才能允许关闭。
- 代码复用不等于复用业务语义：通用文本 rebase 不能代替 CSV 行列语义合并或 YAML schema 校验。
- CSV 编辑器的移动/重命名/复制联动 `.view`，目标冲突预检，部分失败尝试回滚。移动同时携带现有 `.meta` / `.view.meta` 保留 GUID；复制仅复制数据和视图，由 Unity 为副本生成新 GUID。当前使用配套文件操作，不宣称已调用或验证 Unity AssetDatabase 的移动流程。
- 知识目录移动/删除联动 `.view`；知识树、正文索引和注入只包含 CSV 正文。普通文件树仍可把 `.csv.view` 当作文本文件查看，不将它识别为可执行 View。
- 外部单独移动 CSV 导致 `.view` 缺失时使用默认布局，不凭文件名猜测并挪动无关配置。

## 公共基础设施边界

本次抽取的入口与消费者：

| 公共模块 | 责任 | 已接入消费者 |
| --- | --- | --- |
| `src/composables/useDocumentFileSession.ts` | 格式无关的加载、草稿/基线、dirty、保存回包、过期请求隔离、错误状态；适配器注入编解码与 I/O | `WorkspaceFilePreview.vue`、`WorkspaceCsvEditor.vue` |
| `src/document/documentIdentity.ts` | checkout、generation、materialization epoch 与资源身份组成无歧义会话键 | 工作区文件、知识正文和目录会话 |
| `src/document/documentSessionCache.ts` | 通用 LRU、草稿/冲突钉住、软容量、更新 pin 状态不改变访问顺序 | 知识草稿缓存、Markdown EditorState 缓存、CSV 草稿/撤销/选区缓存 |
| `src/document/documentText.ts` | 保留原始字符串的有界 diff/hunk、三方冲突描述、唯一上下文编辑操作 | 知识协作编辑、CodeMirror 外部更新 transaction |
| `src/document/textDocumentFormat.ts` | 显式换行检测/规范化/回写，不裁剪末尾空白 | 现有工作区源码/Markdown 文件适配器 |

既有公共模块继续复用：`useFileChangeRevalidation`、`boundedTextDiff`、共享 workspace 事件、基础控件和主题 token。知识分类、分段保存、自动保存策略、frontmatter、目录权限，以及 CodeMirror selection/undo 的具体表示继续留在各自适配器中。

公共文件会话要求传入不可变草稿与快照；它不依赖 CodeMirror、CSV、YAML、Tauri IPC 或文件类型。`modelDraft` 只在加载/保存等同步边界更新，输入时只更新草稿。`save()` 返回 true 表示当前草稿已经保存，保存期间继续输入时返回 false 并保留 dirty，避免关闭时丢失后续输入。

## 实施阶段

### 阶段 0：公共能力抽取（本次）

- [x] 写入完整方案和选型依据，固定 CSV + YAML `.csv.view` 决策。
- [x] 抽取公共身份、会话缓存、无损字符串差异/合并与显式换行工具。
- [x] 抽取文件会话生命周期，接回现有工作区文件编辑器。
- [x] 知识协作与 CodeMirror 适配器使用同一公共文本差异算法，保留原有知识空白规则。
- [x] 完成边界测试、Markdown/知识/工作区文件回归、Vite 打包与文档校验。
- [ ] 全仓应用/测试类型检查与完整构建无错误：本次范围外的阻塞见“本次验证结果”。

### 阶段 1：网格选型验证

- [x] 完成候选调研，选择 Tabulator，实际验证中文 IME、区域粘贴、结构撤销和焦点；样式复用 Locus token。
- [x] 测试 1 万/10 万单元格档位和虚拟滚动，记录环境与数据；实际分栏可同时打开 CSV。
- [x] 锁定依赖版本、普通 Grid 模式与容量。完整构建的 KnowledgeView chunk 约 843 kB / gzip 215 kB；共享工作区无法据此把整个 chunk 增量归因于 CSV。

### 阶段 2：CSV 数据与 `.view` 协议

- [x] CSV codec、原始字段/记录保留、源数据与显示映射，解析失败转源码修复。
- [x] YAML schema 校验、确定性序列化、版本策略和列绑定。
- [x] 数据/视图独立保存状态、内容哈希校验，视图写入失败保留 dirty 并允许重试。
- [x] CSV 编辑器配套移动/复制与知识目录移动/删除，知识树排除 `.view`。

### 阶段 3：统一编辑入口

- [x] 表格/源码共用草稿，接入工作台、知识和资产文件入口。
- [x] 知识 CSV 的索引、创建、原文读取及目录操作；标题来自文件名，管理属性继承目录配置，不写 frontmatter。
- [x] 现有工作区文件快照以可选 `csv` 字段兼容扩展，包含 YAML 草稿和矩形选区/滚动；CSV 缓存保留未保存数据与撤销历史。
- [x] 复用现有控件及主题 token，实际验证键盘/焦点，移除工具栏和内部输入框边框。

### 阶段 4：端到端验收

- [x] 特殊 CSV 往返：引号、逗号、嵌入换行、BOM、空白、尾部空字段、不等长记录、无/重复表头。
- [x] 仅查看或修改视图时 CSV 字节不变；打开不存在的 `.view` 不创建文件，扩展数据本身也不强制创建视图文件。
- [x] 公共会话覆盖保存时继续输入/旧请求隔离；CSV 测试覆盖外部冲突、无效视图保护、视图失败后重试和草稿丢弃。
- [x] CSV 草稿缓存和快照接入工作台生命周期；实际验证分栏、源码往返及选区，自动测试校验过期磁盘基线拒绝导入。
- [ ] Unity 中 CSV 保存后正确重新导入；资产移动/重命名保留 GUID 和附属文件关系。
- [x] 实测 1 万/10 万单元格与 DOM 数量，设置 16 MiB / 50 万矩形单元格限制。

测试统一使用 `bun run test`，类型检查使用 `bun run typecheck`、`bun run typecheck:test`。WebView2 验证使用隔离实例 `bun run locus:test:app`；需要 Unity 集成时使用仓库 CLI driver，只操作本次测试资源。

## 调研来源

- [Tabulator 授权与能力](https://www.tabulator.info/alternatives/handsontable/)、[区域编辑](https://www.tabulator.info/docs/6.x/range/)、[Spreadsheet 模式限制](https://www.tabulator.info/docs/6.x/spreadsheet/)
- [Jspreadsheet CE](https://bossanova.uk/jspreadsheet/)、[撤销历史](https://bossanova.uk/jspreadsheet/docs/history)
- [Handsontable Vue 3 与授权](https://github.com/handsontable/handsontable/blob/develop/wrappers/vue3/README.md)
- [RevoGrid Vue](https://rv-grid.com/vue-data-grid)、[Pro 功能](https://rv-grid.com/pro/)
- [Papa Parse 文档](https://www.papaparse.com/docs)
- [Git 合并属性](https://git-scm.com/docs/gitattributes)、[YAML 1.2.2](https://yaml.org/spec/1.2.2/)
- [Unity 资源元数据](https://docs.unity3d.com/6000.0/Documentation/Manual/AssetMetadata.html)
