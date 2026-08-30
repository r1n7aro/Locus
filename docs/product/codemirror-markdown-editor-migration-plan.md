# CodeMirror Markdown 编辑器完整迁移计划

状态：迁移已完成，进入持续性能回归

日期：2026-08-29

完成日期：2026-08-30

范围：知识文档打开链路 / Markdown 编辑内核 / Live Preview / 多文档状态缓存 / 自动保存与外部编辑合并 / Vditor 退出

## 实施与验收结果

- 知识文档的实时预览与源码模式已经统一到一个持久 CodeMirror 6 `EditorView`；点击、失焦、模式切换与文档切换均保留编辑器 DOM。
- 本地输入热路径传递 CodeMirror `Text` / `ChangeSet`，自动保存、手动保存、rebase 与会话切换边界才生成完整字符串。
- 外部全文同步与自动保存共用有界多 hunk diff；长文档采用线性扫描、稳定锚点和受限唯一上下文，避免高重复内容进入平方级路径。
- 文档正文、目录与历史读取已经拆分；正文缓存、目录缓存、in-flight 合并、epoch 失效、共享 workspace 事件监听和相邻文档预取已经落地。
- A → B → A 会话保留草稿、base、dirty、conflict、selection、undo 与 scroll；保存回包按不可变资源身份和 revision 回到发起会话。
- Live Preview 覆盖基础 Markdown、GFM table、KaTeX、workspace 图片、Knowledge/workspace/file/View/Unity 引用；危险输入、解析失败、选区相交与超大块稳定回退源码。
- 生产依赖、源码、Vite 配置与构建产物中已经清除旧编辑器及其静态资源复制链；生产 `dist` 总体积约 12.6 MB，CodeMirror 独立 chunk 为 588.06 kB / gzip 207.93 kB。
- 独立发布审查发现并关闭了多点外部变更破坏 undo、Preview 卸载丢草稿、目录旧保存回包劫持、checkout generation 复用旧会话、目录子树缓存失效、历史 enrichment 越过 epoch、引用缺少产品动作等边界。

2026-08-30 隔离 WebView2 验收样本：

| 指标 | 结果 |
| --- | ---: |
| 冷文档点击到正文 EditorView 挂载 | 67.4 ms |
| 冷文档点击到首帧 | 74.7 ms |
| 已挂载正文点击到焦点 | 0.6 ms |
| 普通段落聚焦高度变化 | 0 px |
| 30 次缓存文档往返 | p50 26.5 ms / p95 45.4 ms / max 54.2 ms |
| 往返期间正文 EditorView DOM 替换 | 0 次 |
| 往返期间正文 EditorView 数量降为 0 | 0 次 |
| 1,077,316 B / 12,019 行文档点击到正文 | 119.5 ms |
| 1 MB 文档首个输入事件 | 0.7 ms |
| 1 MB 文档可见 `.cm-line` | 11 个 |
| Preview 卸载重挂后的 undo / 外层 scroll | 正确恢复 / 0 px 误差 |
| Performance trace CLS | 0.00 |
| 20 次切换强制重排总量 | 34 ms；首轮为 639 ms |
| 控制台 error / warn | 0 |

仓库结构基准的普通输入 transaction：100 KB p95 1.95 ms，1 MB p95 2.49 ms；1 MB 外部 diff 7.10 ms，1 MB 高重复文本首尾多点 diff 2.94 ms；20 文档结构往返 p50 97.0 ms / p95 111.5 ms，EditorView DOM 全程稳定。验证入口为 `bun run locus:test:markdown-editor`；隔离截图与 trace 保存在 `artifacts/`。

自动化验证：369 个 Vitest 文件、2165 项测试通过，应用类型检查、测试类型检查、Vite 生产构建、Rust 快速读取/事件聚焦测试与 `cargo fmt --check` 通过。

## 最终决策

Locus 的知识文档编辑器完整迁移到 CodeMirror 6。

- 可编辑知识文档始终保留 CodeMirror 编辑面，点击正文只执行聚焦和选区更新。
- “实时预览”与“源码”共用同一个 EditorView，通过 Compartment 和 decorations 切换显示能力。
- 每个可见 Markdown 字段持有稳定 EditorView；每个文档字段持有独立 EditorState、选区、撤销历史和滚动位置。
- 文档切换使用 workspace-scoped 文档缓存与 EditorState LRU，缓存命中时直接显示，后台完成轻量校验。
- Agent、文件监听和保存回包产生的外部更新转换为 CodeMirror transaction，选区通过 ChangeSet 自动映射。
- 知识正文读取走快速路径，Git 提交信息延迟加载，列表与正文请求按 workspace 和资源身份去重。
- 最终版本删除 Vditor 包、静态资源复制、Vditor 专属样式、激活时 DOM 交换逻辑和对应测试。
- MarkdownRenderer 继续服务 Chat、工具结果和其他只读渲染场景；知识编辑面使用 CodeMirror Live Preview。

迁移期间允许存在仅供开发验证的引擎开关。发布切换完成后删除该开关，产品设置中只保留“实时预览 / 源码”视图选择。

## 当前基线与根因

### 已测编辑器基线

2026-08-29 的隔离 WebView2 样本测量得到：

| 场景 | 当前结果 | 直接原因 |
| --- | ---: | --- |
| 473 B 文档首次点击进入编辑 | 434–597 ms | 点击后才创建完整 Vditor 实例 |
| 同实例热激活 | 3.9–6.2 ms | 运行时和资源已经进入内存 |
| 冷启动增量堆内存 | 约 9.7 MB | Vditor 初始化多个编辑子系统与 Lute 运行时 |
| 预览消失到编辑器可用 | 约 433 ms | MarkdownRenderer DOM 先移除，Vditor 后异步 ready |
| Vditor 发布资源 | 约 23.7 MB | dist 包含编辑器、主题、图标、解析运行时和附属资源 |

这些数字用于确定问题形态。阶段 0 会在固定语料和固定机器上重新建立可重复的 p50 / p95 基线。

### 当前编辑路径

BaseMarkdownEditor.vue 在 rendered + deferRenderedEditor 下采用两套 DOM：

~~~text
未聚焦：MarkdownRenderer / SemanticCodeRenderer
              │ 点击
              ▼
移除预览 DOM → 创建 Vditor IR → after 回调 → 恢复焦点、滚动和插入点
              │ 失焦
              ▼
销毁 Vditor → 重新创建预览 DOM
~~~

闪烁来自编辑面整体替换、冷初始化和高度重新计算。markdownEditorActivation.ts 与 markdownEditorLayout.ts 当前承担几何快照、滚动恢复和 Vditor 内联样式覆盖，这些逻辑只能缩短视觉空档，无法消除双编辑面切换。

### 当前文档打开路径

打开延迟还包含知识数据链的等待：

- DevelopmentWorkbench.vue 通过 v-show 常驻每个知识标签的完整 KnowledgeView。
- 每个 KnowledgeView 创建自己的知识状态与监听，多个标签会重复加载同一 workspace 的列表、目录和状态。
- embedded 知识标签只传 selectedDocumentId；KnowledgeView 需要等待 documents 中出现对应摘要，随后才读取正文。
- useKnowledgeState.ts 首轮刷新并发加载 Design、Memory、Reference、Skill 等全部类型及目录。
- Skill 和 Reference 正文读取传 includeHistory: true；Rust 在正文响应路径同步执行 git log -1 --follow。
- 当前只有摘要 warmup，缺少 workspace-scoped 正文缓存、请求去重与相邻文档预取。

CodeMirror 替换可以消除编辑器冷启动，知识加载链需要同时收敛才能达到打开文档的低延迟目标。

### 当前文档会话风险

- KnowledgePreview 在文档 ID 或类型变化时强制重置三个字段草稿、dirty 集合、冲突和自动保存状态。
- 快速切换可以让未保存草稿失去所属会话。
- 旧文档保存完成后的 Promise 仍可能进入当前 KnowledgePreview 的 rebase 路径。
- updateSection、updateMeta 和 updateDocumentEdits 的排队闭包在实际执行时读取当前 selectedDocument type，切换选择后存在跨文档路由风险。
- contentKey 缺少 workspace、window 和 pane 身份；同路径文档在多 workspace、多分屏场景下可能共享错误状态。
- modelValue 外部更新目前依赖整段 setValue 和手工焦点恢复；长文档会产生完整字符串和完整 DOM 转换。

迁移先建立不可变资源身份、保存路由和会话缓存，再切换编辑内核。

## 产品体验契约

### 打开文档

1. 点击知识树节点后立即更新选中态和标题状态。
2. 正文缓存命中时在下一帧显示缓存内容，同时后台校验版本。
3. 正文缓存未命中时保持工作区几何稳定，快速正文响应到达后一次提交新文档。
4. Git 作者与提交时间独立补齐，属性行预留稳定位置。
5. 过期请求只写入自身缓存，不覆盖当前选择。

### 开始编辑

1. 实时预览模式下 CodeMirror 已经挂载。
2. 点击只改变焦点、选区和当前语法节点 decorations。
3. EditorView DOM 节点、滚动容器和正文根高度保持连续。
4. 输入法组合、拖放、粘贴、撤销和保存快捷键由同一 transaction 管线处理。

### 多文档切换

1. 每个文档的 summary、maintenanceRules、body 分别恢复选区、滚动位置和撤销历史。
2. 未保存 session 始终保留并固定在缓存中，保存完成或用户明确放弃后才允许淘汰。
3. 返回最近文档时复用 EditorState；EditorView 只为新资源执行 setState。
4. 同一文档在不同 pane 中保留独立选区和滚动状态，文本更新通过 transaction 同步。
5. 路径重命名把 session 原子 rekey 到新资源身份。

### 外部编辑与协同

1. Agent 修改、knowledge-changed、保存回包都携带 workspace、type、document ID、path、section 和 revision。
2. 非重叠变化转换为最小 ChangeSet，并标记 Transaction.remote 和 addToHistory(false)。
3. 本地选区、IME composition 和当前视口通过 changes mapping 保持稳定。
4. 重叠变化进入现有冲突 UI，对应文档 session 暂停自动保存。
5. 保存请求绑定发起时的目标和 local revision；回包只确认该 revision 之前的内容，继续输入保持 dirty。

### 视图模式

- 实时预览：单一 CodeMirror 编辑面，光标附近展示 Markdown 标记，其余可见区域使用排版和 widgets。
- 源码：同一 EditorView 保留完整 Markdown 标记、语法高亮、选区、历史和滚动。
- 只读：同一扩展体系启用 EditorState.readOnly 与 EditorView.editable(false)，链接和安全 widgets 保留可访问交互。
- 非 Markdown 引用文件：继续使用语义代码预览，或按语言能力进入 CodeMirror source 配置。

现有 localStorage key 与 rendered / native 持久值继续兼容，减少无关迁移：

| 持久值 | CodeMirror 行为 |
| --- | --- |
| rendered | Live Preview |
| native | Source |

编辑器内部使用 live / source 语义，storage adapter 负责映射。产品切换完成后仍可保留旧持久值读取，现有用户设置直接生效。

## 目标架构

~~~text
DevelopmentWorkbench / KnowledgeView
                │
                ├─ KnowledgeRepository（每 workspace 一份）
                │    ├─ 摘要与目录缓存
                │    ├─ 正文 LRU、in-flight 去重、版本校验
                │    ├─ knowledge-changed 事件路由
                │    └─ 快速正文 / 延迟 Git 元数据
                │
                └─ KnowledgeDocumentSession（每资源一份）
                     ├─ summary / rules / body buffers
                     ├─ base revision / local revision / dirty / conflicts
                     ├─ 自动保存目标与请求序号
                     └─ pane-scoped MarkdownEditorSession
                              │
                              ▼
                    BaseMarkdownEditor façade
                              │
                              ▼
                    Persistent EditorView
                     ├─ EditorState + history
                     ├─ Markdown / readOnly / mode Compartments
                     ├─ transaction router
                     ├─ Live Preview ViewPlugin
                     └─ selection / scroll snapshot
~~~

### 分层职责

| 层 | 职责 | 生命周期 |
| --- | --- | --- |
| KnowledgeRepository | 文档发现、读取、缓存、失效、事件与请求去重 | workspace |
| KnowledgeDocumentSession | 草稿、base、dirty、冲突、自动保存和 revision | document |
| MarkdownEditorSessionCache | EditorState、历史、选区、滚动和内存预算 | window + pane |
| BaseMarkdownEditor | Vue 生命周期、DOM host、props/事件兼容 | visible field |
| CodeMirror extensions | 输入、快捷键、主题、Markdown、预览和 widgets | EditorState |
| MarkdownRenderer | Chat 与只读 Markdown 场景 | renderer consumer |

### 资源身份

文档与编辑器状态使用结构化 key，路径只承担资源定位：

~~~ts
interface KnowledgeResourceKey {
  projectOrWorkspaceKey: string;
  checkoutId: string;
  workspaceGeneration: number;
  type: KnowledgeDocumentType;
  documentId: string;
  path: string;
}

interface MarkdownEditorSessionKey {
  windowId: string;
  paneId: string;
  resource: KnowledgeResourceKey;
  section: "summary" | "maintenanceRules" | "body";
}
~~~

projectOrWorkspaceKey 表达知识资源的稳定所有权，checkoutId + workspaceGeneration 表达本次读取和事件路由的运行时作用域。documentId 在该资源所有权内作为主要身份，type + normalized path 作为校验和无稳定 ID 时的回退。保存、重命名、删除、外部事件与缓存失效都传完整 key。

### 文档会话

~~~ts
interface KnowledgeDocumentSession {
  key: KnowledgeResourceKey;
  baseRevision: string;
  localRevision: number;
  acknowledgedLocalRevision: number;
  sections: Record<KnowledgeDocumentSection, MarkdownBuffer>;
  dirtySections: Set<KnowledgeDocumentSection>;
  conflicts: Record<KnowledgeDocumentSection, KnowledgeTextConflict[]>;
  pendingSave: SaveRequestIdentity | null;
}
~~~

- MarkdownBuffer 的实时权威文本使用 CodeMirror Text。
- dirty 判断比较 revision，输入热路径不执行整篇 toString。
- 自动保存定时器触发时才生成目标 section 的字符串快照和 edit operations。
- summary、rules、body 共享文档 session，保留独立 EditorState。
- 关闭隐藏 tab 时文档 session继续存在；脏 session保持 pinned。
- 保存成功按 request 的 targetKey 和 startedLocalRevision 确认。

### EditorState LRU

缓存分为两层：

- 文档正文 LRU：workspace-scoped，保存 KnowledgeDocument DTO、基础文件 metadata、版本与读取时间。
- EditorState LRU：window/pane-scoped，保存每个 section 的 EditorState、scroll snapshot、lastUsedAt 和估算权重。

初始策略：

- 每个 pane 保留最近 12 个干净 section state。
- 全窗口采用 32 MiB 软预算，以正文长度、历史深度和 widget 状态估算权重。
- dirty、saving、conflicted 和当前可见 state 固定保留。
- 超预算时依次丢弃干净 state 的撤销历史、复杂 widget cache、最后淘汰完整 state。
- 淘汰完整 state 前保存轻量 cursor/scroll；再次打开从正文缓存创建新 state。
- 限额通过基准测试校准，最终值写入单一常量和测试。

文档切换流程：

1. 提交当前 view.state 与 EditorView.scrollSnapshot。
2. 将当前 state 写回 session cache。
3. 取得目标 state；缓存缺失时从 MarkdownBuffer 创建。
4. 对目标 state 同步 mode、readOnly、placeholder、theme 等 compartments。
5. 调用同一个 EditorView.setState。
6. 使用 scroll snapshot 恢复视口，并在 requestMeasure 写阶段完成尺寸同步。

setState 只用于无派生关系的文档切换。当前文档的输入、外部变化、模式切换和属性变化全部使用 dispatch。

### Vue 数据契约迁移

现有 modelValue / update:modelValue 在迁移早期作为兼容层保留。最终知识编辑路径使用 session 驱动：

~~~ts
interface MarkdownEditorBinding {
  key: MarkdownEditorSessionKey;
  initialText: string;
  revision: string;
  readOnly: boolean;
  mode: "live" | "source";
}

interface MarkdownEditorChangeEvent {
  key: MarkdownEditorSessionKey;
  origin: "local" | "remote" | "restore" | "reconfigure";
  localRevision: number;
  changes: readonly MarkdownTextChange[];
}
~~~

- transaction 热路径发送增量变化和 revision。
- KnowledgeDocumentSession 在 autosave、导出、冲突计算时读取完整快照。
- 兼容 v-model 不再驱动当前 EditorView 回写，避免输入 → Vue string → props → setValue 循环。
- 目录配置编辑器与正文编辑器完成 session 接入后删除 syncWhileFocused。

## CodeMirror 依赖与构建

采用直接包依赖，避免引入包含多余默认能力的聚合配置：

- @codemirror/state
- @codemirror/view
- @codemirror/commands
- @codemirror/language
- @codemirror/search
- @codemirror/lang-markdown
- @codemirror/lang-json
- @codemirror/lang-python
- @lezer/markdown
- @lezer/highlight

实现要求：

- 扩展列表显式组装，控制 history、keymap、search、lineWrapping、placeholder 和 syntaxHighlighting。
- Markdown 核心随知识页面加载，知识页进入时完成预热；点击正文时不执行 dynamic import。
- fenced code language 按语言懒加载，首包不包含完整语言集合。
- 当前知识引用支持的 JSON / Python 使用 language compartment；JSON pretty print 变为明确格式化命令，原始文本始终是编辑权威值。
- Vite 为 CodeMirror 形成稳定 markdown-editor chunk，并记录 gzip / brotli 与解析耗时。
- bun.lock 只保留一组兼容的 @codemirror/state、view 和 @lezer/common 实例。
- dark/light 主题使用 Locus token，主题切换通过 CSS 和 theme compartment 生效。
- spellcheck、tab size、line wrapping、键盘映射和 readOnly 使用 CodeMirror API 配置。

## Live Preview 设计

### 核心原则

- Lezer syntax tree 作为语法位置来源。
- ViewPlugin 只遍历 EditorView.visibleRanges 及小幅 overscan。
- decorations 在 docChanged、viewportChanged、selectionSet 或 syntax tree 更新时增量重算。
- 当前选区相交的语法节点展示源码标记；其余节点应用排版、mark、replace 或 widget。
- 同一 EditorView 内完成源码显隐，编辑面和滚动容器保持原实例。
- WidgetType 实现 eq、updateDOM 和 destroy，稳定复用 DOM 与 Vue 子组件。
- block widgets 预留高度，异步图片、数学公式和 Unity 预览完成后使用 requestMeasure 协调布局。
- 大文档只构建可见区域 decorations；超大单行、超大表格和解析超时采用局部源码呈现。

### 当前语法块激活

用户点击已渲染节点时：

1. widget 或 decoration 根据 DOM 位置映射到文档 range。
2. transaction 设置 selection，并更新 active syntax ranges。
3. 目标块在同一帧展开 Markdown 标记。
4. CodeMirror 将 selection 映射到新布局并保持 scroll anchor。
5. 输入直接进入原 EditorView。

这个流程替代 captureMarkdownEditorActivation、预览 DOM 移除、Vditor after 和双 requestAnimationFrame 恢复。

### 功能矩阵

| 能力 | 实现方式 | 切换默认前要求 |
| --- | --- | --- |
| 标题 | line decoration + 标记显隐 | 必须 |
| 粗体、斜体、删除线 | mark decoration + delimiter reveal | 必须 |
| 行内代码 | mark decoration + delimiter reveal | 必须 |
| 有序/无序列表 | line decoration + marker widget | 必须 |
| 任务列表 | checkbox WidgetType + transaction toggle | 必须 |
| 引用、分隔线 | line/block decoration | 必须 |
| 链接 | 文本 mark + 安全点击；编辑时展开目标 | 必须 |
| fenced code | block decoration + Markdown nested language | 必须 |
| GFM 表格 | 非活动表格 widget；活动表格源码编辑 | 必须 |
| 数学公式 | KaTeX widget；活动公式源码编辑 | 必须 |
| 图片 | 解析后的 image widget、状态缓存和尺寸占位 | 必须 |
| 文件/知识/workspace 引用 | inline widget + 现有打开/拖拽动作 | 必须 |
| Unity Asset / Scene Object 引用 | inline 或 Vue widget adapter | 必须 |
| Unity property fence | 复用解析器与 Vue block widget | 必须 |
| View reference block | Vue block widget + 打开动作 | 必须 |
| 搜索命中 | CodeMirror search decoration + scrollIntoView | 必须 |
| 非 Markdown 代码引用 | source language 或 SemanticCodeRenderer | 必须 |

### 与 MarkdownRenderer 的共享

MarkdownRenderer 继续是全局只读渲染层。迁移会抽取以下纯逻辑供两侧复用：

- Markdown 链接与本地资源地址解析。
- 图片 source 解析与加载状态缓存。
- 数学公式解析与 KaTeX 安全渲染。
- 文件、知识、workspace、Unity 和 View 引用识别。
- Unity property fence 解析。
- 安全 URL、外部打开和拖拽 payload 构造。

Live Preview 直接消费结构化 token / model，避免把整篇 Markdown 先转 HTML 再反向定位源码。复杂 Vue widget 通过通用 VueWidgetHost 挂载，并在 WidgetType.destroy 中可靠卸载。

### 排版与布局

- 复用 --bg-color、--panel-bg、--border-color、--text-color、--text-secondary、--accent-color 等现有 token。
- 编辑器正文沿用 KnowledgePreview 当前 14px 字号、行高、正文宽度和边距节奏。
- summary 与 maintenanceRules 使用 compact auto-grow 配置和明确上限。
- 现有连续文档页与外层滚动容器先保持布局语义，body 的滚动所有权只在基准证明 viewport DOM 无法保持有界时调整。
- 1 MB auto-grow 正文必须实测 visibleRanges、DOM 行数和快速滚动；需要内层滚动时沿用 IDE pane 语法并保持正文宽度与页面结构。
- live 与 source 共用边框、背景、focus-visible 和滚动条样式。
- Markdown 文档内容保持轻量排版，widgets 的交互视觉沿用 MarkdownRenderer 现有文内引用语法。
- 模式切换、聚焦和失焦不触发编辑器根节点高度动画。

## 知识文档加载与缓存

### Workspace-scoped KnowledgeRepository

当前每个 KnowledgeView 内部拥有一套 useKnowledgeState。迁移后按 workspaceRef 复用仓库：

- 一个 workspace 只注册一次 knowledge-changed 与 plugins-changed 监听。
- documents、directories、overview、正文缓存和 in-flight 请求在同 workspace 内共享。
- KnowledgeView 保留本地 selection、sidebar 和 search 展示状态。
- 根知识页按需加载类型树；embedded 文档标签直接走目标文档快速路径。
- 多个 pane 读取同一文档复用同一个正文 Promise 和缓存条目。
- workspace generation 变化后整体失效，旧 generation 回包只进入丢弃路径。
- knowledge-changed payload 补齐 type、documentId、path、change kind 与 revision；本地保存回声可以精确确认目标缓存和 session。

### Embedded fast path

DevelopmentWorkbench 向 KnowledgeView 传递完整目标：

~~~ts
interface EmbeddedKnowledgeTarget {
  id: string;
  type: KnowledgeDocumentType;
  path: string;
  workspaceRef: WorkspaceRef;
}
~~~

embedded 文档打开顺序：

1. 使用完整 target 查询正文缓存。
2. 缓存命中时创建/恢复 KnowledgeDocumentSession。
3. 缓存缺失时直接 knowledge_read full，includeHistory=false。
4. 文档显示后按需加载所属类型摘要与祖先目录。
5. 根知识树数据在后台共享加载，正文显示不等待五类列表。

### 后端响应拆分

正文快速响应包含：

- KnowledgeDocument 内容。
- 文件大小、行数、字符数、token 估算、modifiedAt 等本地可得 metadata。
- 可用于前端校验的 content revision。

Git enrichment 独立调用并返回：

- lastCommitAuthor
- lastCommitAt
- 对应 workspace generation、type、path 和 content revision

Rust 的 git log --follow 离开 knowledge_read 正文关键路径。历史请求按 repo + normalized path 去重并短期缓存；选择变化后允许任务完成并写缓存，UI 只接收身份匹配的结果。

### 正文缓存策略

- key：workspace generation + type + documentId/path。
- version：后端 content revision，modifiedAt 作为兼容校验。
- 策略：cache-first + background revalidate。
- knowledge-changed 精确失效目标文档、所属目录摘要和相关 metadata。
- 当前 dirty session 收到外部变化时进入 rebase，缓存更新不会覆盖本地 buffer。
- 保存成功写入 authoritative document 与 revision，并确认对应 local revision。
- 删除立即移除缓存并关闭对应 session。
- 重命名原子迁移 cache key、session key、in-flight bookkeeping 和打开标签资源。
- 空闲时只预取当前节点相邻 1–2 个文档，受 workspace I/O 与缓存预算限制。
- 当前 selectionSeq、旧面板保持可见和正文成功后原子提交的语义继续保留；缓存只缩短等待，不引入中间空面板。

## 保存、撤销与外部更新

### 保存目标

所有编辑 API 在入队前捕获不可变目标：

~~~ts
interface KnowledgeMutationTarget {
  workspaceRef: WorkspaceRef;
  type: KnowledgeDocumentType;
  id: string;
  path: string;
  documentRevision: string;
  localRevision: number;
}
~~~

updateSection、updateMeta、updateDocumentEdits 和 rename 都使用 target.type，排队闭包不读取当前 selectedDocument 或 activeType。

自动保存期间编辑器继续可输入。saveLoading 只更新保存状态与高风险元数据控件，正文和目录 Markdown 字段保持原 EditorView 与当前 focus。

### 回包路由

- 每个 save 请求拥有 requestId。
- 回包先写 KnowledgeRepository 中 target key 的 authoritative revision。
- 发起 session 仍存在时，根据 startedLocalRevision 执行 acknowledge 或 rebase。
- 当前 UI 选择只决定是否重绘属性，不决定回包归属。
- 同文档较旧回包无法覆盖更新 revision。
- 路径重命名由后端结果触发单次 rekey。

### CodeMirror transaction 来源

| 来源 | annotation | 历史策略 |
| --- | --- | --- |
| 键盘、IME、粘贴、拖放 | local userEvent | 进入当前文档历史 |
| Agent / knowledge-changed | Transaction.remote(true) | addToHistory(false) |
| 保存 authoritative normalize | remote + saveAck | addToHistory(false) |
| 文档恢复 | restore | 使用缓存 state，不创建历史项 |
| live/source 切换 | Compartment reconfigure | 文本历史保持原样 |
| 冲突选择“保留本地/采用远端” | explicit command | 独立可撤销历史组 |

同一文档多 pane 时，把 ChangeSet 转发给其他 EditorView，并附 remote annotation。每个 pane 保留自己的 selection，正文保持一致。

## 分阶段实施

### 阶段 0：冻结契约与建立基线

工作项：

- 建立 1 KB、100 KB、1 MB、复杂表格/图片/数学/Unity 引用四组固定语料。
- 增加开发性能标记：tree click、cache hit、content response、editor first paint、focus ready、first input。
- 记录冷启动、缓存切换、20 文档往返、长文档输入、模式切换和外部 patch 的 p50 / p95。
- 为 BaseMarkdownEditor 当前 props、事件、保存快捷键、粘贴规则、换行归一化、readOnly 和三 section 布局补齐契约测试。
- 为快速切换期间的草稿保留、保存回包归属和 target type 捕获增加失败测试。
- 添加仅开发可用的 editor engine switch，默认继续使用当前引擎。

退出条件：

- 基准脚本或 DevTools 操作步骤可重复。
- 根因场景拥有自动化失败用例。
- 现有协同编辑测试纳入基线。

### 阶段 1：建立知识仓库与文档会话

工作项：

- 将 useKnowledgeState 的共享数据、事件和请求提升为 workspace-scoped KnowledgeRepository。
- KnowledgeView 使用局部 selection view model，多个标签共享 repository。
- DevelopmentWorkbench 传完整 EmbeddedKnowledgeTarget。
- embedded 文档直接读正文，类型树与其他目录后台加载。
- 新增正文 LRU、in-flight 去重、workspace generation 失效和相邻文档预取。
- 将 Git history 从正文读取拆到异步 enrichment。
- 引入 KnowledgeDocumentSession，草稿、dirty、conflict、autosave 绑定资源 key。
- 所有 mutation 在排队前捕获完整 target，保存回包路由到发起 session。
- knowledge-changed 事件增加精确目标与 revision，本地保存回声只刷新对应资源。

退出条件：

- embedded 文档正文不等待五类 documents/directories。
- 同 workspace 打开多个知识标签只执行一组共享列表请求和事件监听。
- 快速 A → B → A 切换保留 A 草稿。
- A 的保存回包无法修改 B。
- Skill / Reference 正文响应关键路径不执行 git log。
- 目录自动保存期间 EditorView identity、focus 和输入保持连续。
- 一次本地保存回声不会按知识标签数量放大全类型刷新。

### 阶段 2：CodeMirror 源码编辑内核

工作项：

- 安装直接 CodeMirror / Lezer 依赖并配置独立 chunk。
- 新增 CodeMirror state factory、extensions、theme、transaction router 和 session cache。
- BaseMarkdownEditor 保留外部名称，内部可选择 CodeMirror 实现。
- 实现 source 模式、history、搜索、line wrapping、placeholder、readOnly、Mod-S、纯文本优先粘贴和换行归一化。
- 接入 summary、maintenanceRules、body 与目录配置字段。
- 当前文档 props 更新转换为最小 ChangeSet。
- document switch 保存并恢复 EditorState、selection、history 和 scroll snapshot。
- 输入热路径切换为增量事件，自动保存时生成完整文本。
- hidden 知识标签暂停 editor view plugin 与异步 widget；最近可见 view 采用有限 warm pool，其余只保留 EditorState。

退出条件：

- source 模式覆盖当前 native textarea 行为。
- 点击已挂载编辑器在同一帧获得输入能力。
- A → B → A 的撤销栈、选区和滚动互相隔离。
- 100 KB 与 1 MB 输入达到性能门槛。
- IME composition、粘贴、撤销和保存快捷键通过自动化与实机验证。

### 阶段 3：Live Preview 基础能力

工作项：

- 基于 Lezer tree 与 visibleRanges 建立 selection-aware decorations。
- 完成标题、强调、删除线、行内代码、列表、任务列表、引用、分隔线、链接和 fenced code。
- live/source 通过 Compartment reconfigure 切换。
- 点击 decoration 或 widget 直接设置 CodeMirror selection 并展开当前语法块。
- 搜索命中迁入 CodeMirror decorations，移除 KnowledgePreview 中编辑面与 preview-rendered-search 的切换。
- readOnly 配置复用同一 live extension。

退出条件：

- 实时预览点击前后 EditorView DOM identity 保持一致。
- 聚焦、失焦、模式切换没有空白帧。
- 基础 Markdown 语料与 MarkdownRenderer 视觉语义一致。
- 搜索命中可滚动定位并可立即继续编辑。

### 阶段 4：复杂块与 Locus 语义对齐

工作项：

- 完成 GFM table、KaTeX、image、file/knowledge/workspace 引用。
- 完成 Unity Asset、Scene Object、Unity property fence 和 View reference widgets。
- 抽取 MarkdownRenderer 与 Live Preview 共用的解析、资源解析、安全和交互模型。
- 实现 VueWidgetHost 生命周期、异步缓存、尺寸占位和错误 fallback。
- 为复杂表格、超长单行、超大 fenced block 提供局部 source fallback。
- 完成 dark/light、窄 pane、缩放、复制选择、键盘导航和屏幕阅读器语义。

退出条件：

- 当前知识预览能显示的 Markdown 与 Locus 扩展语义均有 CodeMirror 路径。
- 异步 widget 加载不改变当前 selection，不产生整页跳动。
- widget 销毁后没有 Vue 实例、监听器和对象 URL 泄漏。

### 阶段 5：默认切换与稳定性验证

工作项：

- 开发引擎开关默认改为 CodeMirror，运行完整测试与固定语料基准。
- 使用 bun run locus:test:app -- --skip-onboarding 启动隔离实例。
- 在 WebView2 DevTools 验证长任务、heap snapshot、DOM 节点、layout shift 和 20 文档切换。
- 验证多 workspace、分屏、隐藏 tab、只读 Reference、Skill、Design、Memory 和目录配置。
- 验证 Agent 连续编辑、本地继续输入、冲突处理、保存失败、重命名与删除。
- CodeMirror 路径稳定后删除开发回退入口。

退出条件：

- 所有功能与性能门槛通过。
- 连续使用和切换测试没有数据丢失、跨文档保存、选区跳转和内存持续增长。
- 发布路径只使用 CodeMirror。

### 阶段 6：删除 Vditor

删除项：

- package.json 与 bun.lock 中的 vditor。
- vite.config.ts 中 vendor/vditor 静态复制。
- vite-plugin-static-copy；该插件当前只服务 Vditor，确认无新增消费者后同时删除。
- BaseMarkdownEditor.vue 中 Vditor import、图标 marker、cdn、IR DOM 查询、setValue、clearStack 和 destroy 逻辑。
- 全部 .vditor、.vditor-ir、.vditor-reset、.vditor-content 与 toolbar 专属 CSS。
- markdownEditorActivation.ts 及其测试。
- markdownEditorLayout.ts 中 Vditor DOM 覆盖；保留的通用布局逻辑迁入 CodeMirror theme/layout。
- deferRenderedEditor、syncWhileFocused 和 activationMinHeight 兼容 props。
- Vditor 专属 source/layout/activation 测试断言。
- dist/vendor/vditor 构建产物。

diff-match-patch 已被 knowledgeCollaborativeEditing.ts 直接使用，继续作为协同 rebase 的显式依赖。

完成检查：

~~~powershell
rg -n "vditor|Vditor" src vite.config.ts package.json bun.lock
bun run test
bun run typecheck:test
bun run typecheck
bun run build
~~~

第一条命令在列出的生产与依赖文件中返回零结果。

## 文件级改造地图

| 当前文件 | 计划动作 |
| --- | --- |
| src/components/ui/BaseMarkdownEditor.vue | 保留组件入口，替换为 CodeMirror host 与 binding |
| src/components/ui/markdownEditorActivation.ts | Live Preview 稳定后删除 |
| src/components/ui/markdownEditorLayout.ts | 删除 Vditor DOM override，迁移通用尺寸策略 |
| src/components/ui/markdownEditorFormatting.ts | 保留换行/粘贴纯函数，增加 ChangeSet 转换 |
| src/components/ui/markdownEditorViewMode.ts | rendered/native 兼容迁移到 live/source |
| src/components/ui/markdown-editor/* | 新增 state、extensions、session cache、live preview、widgets、theme |
| src/components/knowledge/KnowledgePreview.vue | 接入 document session，移除双预览分支与 per-document 草稿重置 |
| src/components/knowledge/KnowledgeDirectoryPreview.vue | 接入 CodeMirror session 与统一模式 |
| src/components/KnowledgeView.vue | 接入共享 repository、完整 embedded target 与局部 selection |
| src/components/workbench/DevelopmentWorkbench.vue | 传 target/window/pane/visible 身份，限制隐藏编辑 view 生命周期 |
| src/composables/useKnowledgeState.ts | 拆分 repository 与 view state，修正 mutation target 和回包路由 |
| src/services/knowledge.ts | 增加快速正文与 history enrichment 调用 |
| src-tauri/src/commands/knowledge.rs | 暴露 metadata enrichment 或等价分段接口 |
| src-tauri/src/knowledge_store.rs | 将 git log 移出正文关键路径，返回 content revision |
| src/components/MarkdownRenderer.vue | 继续服务只读场景，抽取可复用解析/交互模型 |
| package.json / bun.lock | 加 CodeMirror，最终删 Vditor 与仅供其使用的复制插件 |
| vite.config.ts | 删除 Vditor vendor copy，增加稳定 CodeMirror chunk 策略 |

建议新增模块：

~~~text
src/components/ui/markdown-editor/
├─ codeMirrorMarkdownExtensions.ts
├─ markdownEditorBinding.ts
├─ markdownEditorSessionCache.ts
├─ markdownEditorTransactions.ts
├─ markdownLivePreview.ts
├─ markdownLivePreviewTheme.ts
└─ widgets/
   ├─ MarkdownCheckboxWidget.ts
   ├─ MarkdownImageWidget.ts
   ├─ MarkdownMathWidget.ts
   ├─ MarkdownReferenceWidget.ts
   └─ VueWidgetHost.ts

src/composables/knowledge/
├─ knowledgeRepository.ts
├─ knowledgeDocumentCache.ts
├─ knowledgeDocumentSession.ts
└─ knowledgeMutationRouting.ts
~~~

实际拆分应保持模块单一职责，并避免把 CodeMirror 类型扩散到知识 IPC 和持久化类型。

## 测试计划

### 单元测试

- MarkdownEditorSessionKey 在 workspace、window、pane、document 和 section 之间隔离。
- LRU 淘汰顺序、软预算、dirty pin、rename rekey 和 delete 清理。
- document switch 保存与恢复 selection、history、scroll。
- external string diff 转 ChangeSet，selection mapping 正确。
- Transaction.remote、addToHistory(false) 和本地 undo 边界正确。
- rendered/native storage adapter 到 live/source 行为映射。
- Live Preview decorations 只覆盖 visibleRanges。
- 当前 selection 相交节点展示 Markdown delimiter。
- task checkbox、link、table、math、image 和引用 widget 生成正确 transaction。
- request dedupe、cache invalidation、workspace generation 与 stale response。
- save target 捕获、旧回包路由和 continued typing rebase。

### 组件测试

- focus 前后 .cm-editor 节点 identity 相同。
- live/source 前后 .cm-editor 节点 identity 相同。
- summary、rules、body 拥有独立 session。
- readOnly 不产生文档 change，链接与允许的 widgets 可操作。
- KnowledgePreview 切 A → B → A 保留未保存草稿。
- A save resolve 期间切到 B，B 内容保持不变。
- knowledge-changed 在聚焦输入时应用非重叠 patch 并保持 selection。
- 搜索高亮保留编辑面并滚动到匹配。
- hidden tab 暂停 view，恢复后 state 完整。

### 集成与手工矩阵

| 维度 | 样本 |
| --- | --- |
| 类型 | Design / Memory / Reference / Skill / 目录配置 |
| 权限 | 可编辑 / AI maintained / 只读 / 外部来源 |
| 模式 | live / source |
| 主题 | light / dark |
| 输入 | 英文 / 中文 IME / 日文 IME / emoji / 合成字符 |
| 内容规模 | 1 KB / 100 KB / 1 MB / 超长单行 |
| 语法 | 标题、列表、表格、代码、数学、图片、Locus refs、Unity widgets |
| 切换 | 树内切换 / workbench tab / pane / workspace |
| 外部变化 | Agent edit / filesystem event / save ack / rename / delete |

测试命令统一使用：

~~~powershell
bun run test
bun run typecheck:test
bun run typecheck
bun run build
~~~

桌面实测使用：

~~~powershell
bun run locus:test:app -- --skip-onboarding
~~~

## 性能门槛

在固定 Windows / WebView2 环境、release-like 前端构建和固定语料上采集至少 30 次，报告 p50 / p95：

| 指标 | 验收门槛 |
| --- | ---: |
| 已显示 live 文档点击到可输入 | p95 ≤ 16 ms |
| cached 文档选择到编辑器完成首帧 | p95 ≤ 50 ms |
| uncached 正文响应到编辑器完成首帧 | p95 ≤ 32 ms |
| live/source 模式切换 | p95 ≤ 16 ms |
| 100 KB 文档普通输入 transaction | p95 ≤ 16 ms |
| 1 MB 文档普通输入 transaction | p95 ≤ 32 ms |
| 外部 1 KB 非重叠 patch 应用 | p95 ≤ 16 ms |
| 标准语料编辑器引起的主线程长任务 | 0 个超过 50 ms |
| 聚焦/失焦编辑器根高度变化 | 0 px |
| 20 文档往返后的脏草稿丢失 | 0 |
| 20 文档往返后的跨文档 undo/save | 0 |

内存门槛：

- 冷打开一个小型知识文档的编辑器增量 heap 明显低于当前约 9.7 MB 基线。
- 20 文档往返后 heap 在 LRU 淘汰和 GC 后回到预算允许范围。
- hidden tab 数量增加时，EditorView DOM、MutationObserver、ResizeObserver 和 Vue widget 数量保持有界。
- 删除或淘汰 session 后，对应 EditorState、widget、对象 URL 和事件回调可被回收。

性能门槛发生环境波动时保留绝对门槛，同时报告相对当前主分支的改善比例。

## 风险与控制

| 风险 | 控制 |
| --- | --- |
| Live Preview 源码位置与 widget DOM 映射复杂 | Lezer range 为唯一来源；功能按语法族分阶段进入 |
| table / math / Unity block 改变高度 | 稳定 WidgetType key、尺寸占位、requestMeasure、活动块 source fallback |
| IME composition 被外部 patch 打断 | composition 期间排队远端变化，compositionend 后映射并提交 |
| 大文档每次输入生成完整字符串 | session 以 CodeMirror Text 为权威，autosave 时才 materialize |
| 多标签常驻增加内存 | workspace repository 共享、有限 warm view、EditorState weighted LRU |
| 脏 session 被淘汰 | dirty/saving/conflicted 状态固定保留 |
| 保存回包写入当前文档 | immutable target + requestId + local revision 路由 |
| workspace 切换复用旧缓存 | workspace generation 进入 key，旧 generation 回包丢弃 |
| CodeMirror 包出现重复 core 实例 | 直接依赖、bun.lock 检查、构建期依赖树断言 |
| 与 MarkdownRenderer 样式分叉 | 共用 token、解析模型和固定视觉语料截图 |
| 复杂扩展阻塞首次打开 | 核心同步预热，语言与重型 widgets 按可见块懒加载 |

## 完成定义

满足以下全部条件后，迁移完成：

- 知识文档 live 与 source 全部使用 CodeMirror。
- 点击正文、失焦、模式切换和文档切换期间没有预览/编辑 DOM 交换和空白帧。
- 每个文档 section 的草稿、选区、滚动与撤销历史独立恢复。
- 快速切换、Agent 外部编辑和保存回包通过文档身份与 revision 安全路由。
- embedded 文档正文打开不等待全类型树，也不等待 Git history。
- 正文缓存、请求去重、精确失效和 workspace generation 隔离生效。
- 当前知识 Markdown 与 Locus 扩展语义达到功能矩阵要求。
- 性能、内存、IME、readOnly、multi-pane 和 multi-workspace 验收通过。
- 生产依赖、源码、Vite 配置与构建产物中不存在 Vditor。
- 全量 Vitest、测试类型检查、应用类型检查和构建通过。

## 官方技术依据

- CodeMirror Reference Manual：https://codemirror.net/docs/ref/
- CodeMirror System Guide：https://codemirror.net/docs/guide/
- Decorations 示例：https://codemirror.net/examples/decoration/
- Configuration / Compartment 示例：https://codemirror.net/examples/config/
- Split View 状态同步示例：https://codemirror.net/examples/split/
- Collaborative Editing 示例：https://codemirror.net/examples/collab/

CodeMirror 官方文档确认 EditorState 为不可变值、EditorView 支持 setState 与 transaction 更新、Compartment 支持局部重配置、编辑器只绘制 viewport 周边内容、decorations 可以基于 visibleRanges 增量计算。这些能力直接对应 Locus 的持久编辑面、多文档状态恢复、Live Preview 和外部 patch 映射需求。
