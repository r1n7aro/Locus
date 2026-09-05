# 开发工作台与多工作区资源树重构计划

状态：方案已收敛，首个验收节点已确定

日期：2026-08-28

首个验收节点：主窗口内拖动创建分屏，并可手动调整分隔条。

范围：Locus 前端信息架构 / ProjectContext 共享资源 / 多工作区资源树 / 虚拟目录 / 一级视图页 / 编辑器标签与分屏 / 独立窗口 / worktree 执行作用域

## 最终产品决策

Locus 主窗口只保留一层应用级导航：

```text
Locus  开发  视图  插件  Agent  设置
```

现有第二层工作区栏整体删除。当前第二层中的会话、知识、协作和资产迁入“开发”页左侧资源树；“视图”提升为独立一级页面；现有一级“项目”入口由“开发”取代。

同一逻辑工作区由 `ProjectContext` 表达。它下面的多个 Git worktree 共享会话、知识、协作和逻辑资产目录。每个 `WorkspaceRuntime` 继续维护具体 worktree 的执行环境、Unity service、Knowledge index、AssetDB 和文件版本。

多工作区模式继续沿用当前 Workspace Tree 投影：每个 ProjectContext 只有一棵共享虚拟树，【知识】与【协作】在同一 ProjectContext 下各出现一次，不为主工作树和其他 worktree 复制节点。【协作】可以展开具体 worktree，用于选择分支、工作目录和 checkout 级操作；【知识】始终使用 ProjectContext 级资源身份。窗口化只改变右侧 EditorGroup，不改变左侧 Workspace Tree 的层级、布局预设和拖动语义。

“开发”是 ProjectContext 资源工作台。“视图”是跨工作区 View 总入口。“插件”“Agent”“设置”继续承载进程级、跨工作区通用能力。checkout 级 Agent / Plugin overlay、Unity、扫描和项目状态由对应 worktree 节点及其详情承载。

后续出现的会话、知识文档和资产标签属于右侧编辑器组。编辑器标签可以分屏和拖出窗口，和已删除的应用级第二层栏具有不同的状态模型与生命周期。

## 背景与现状

当前前端仍以单个当前工作区组织页面：

- `src/App.vue` 使用 `project / plugins / agent / settings` 一级导航，并在 `project-tab-bar` 中渲染 `chat / knowledge / collab / asset / views` 第二层分页。
- `src/stores/ui.ts` 把 `chat / collab / knowledge / asset / views` 和进程级页面压在同一个 `activeTab` 联合类型中。
- `ChatView.vue` 内嵌 `SessionPanel.vue`；Knowledge、View 等页面又各自维护一套左侧树或列表。
- 标题栏的工作区选择器继续表达唯一当前工作区，和已经引入的 pane / checkout 上下文重复。
- `KnowledgeExplorer.vue` 的拖动会调用知识移动接口并改变物理文件位置，不能直接作为新的用户视图拖动语义。
- `SessionPanel.vue` 中的 View 目录已经具有逻辑 `displayPath`、虚拟文件夹和拖动能力，但它仍依附于会话侧栏；新架构将这部分迁入一级“视图”页面。

后端已经具备前端重构所需的关键基础：

- `ProjectRegistry -> ProjectContext -> WorkspaceRuntime` 能区分同一逻辑工作区的多个 checkout，`ProjectContext` 已经包含 project-owned session catalog 的基础实现。
- `WorkspaceRuntime`、`WorkspaceRef`、`AgentExecutionContext` 和 checkout 级 service instance 已能表达独立执行作用域。
- `WindowContextRegistry` 已按 `(windowId, paneId)` 保存 focused checkout、generation、active session 和 runtime lease。
- `workspaceContextStore` 已能维护项目 / checkout 目录、pane context、作用域事件投影和焦点切换的 intent epoch。
- `WorkspaceEventEnvelope` 已包含 checkout、runtime generation、service instance 和 service generation，可用于多工作区 reducer。
- 会话窗口、工作区页面窗口和 View host 已提供独立窗口、拖动与两阶段交接的可复用经验。

这次迁移的核心任务是让前端完全使用这些显式作用域，并增加一个与物理目录解耦的用户资源视图。

## 目标

- 删除所有 worktree 级应用分页和标题栏 checkout 投影。
- 将“开发”提升为一级页面，形成左侧资源树与右侧编辑区的稳定工作台。
- 将“视图”提升为一级页面，聚合不同工作区的 View。
- 支持单工作区模式和多工作区模式，并允许同一窗口同时观察多个 ProjectContext 与 checkout。
- 每个 ProjectContext 统一呈现共享的会话、知识、协作和资产，以及可供资源绑定的 worktree 列表。
- 同一会话、知识文档、协作对象和逻辑资产可以在同项目的不同 worktree 中复用。
- 允许用户创建虚拟文件夹、排序和拖动资源；所有操作只修改 Locus 用户视图。
- 点击 ProjectContext、worktree、分区、文件夹或资源节点时，右侧显示对应内容。
- 初始闭环继续复用现有 Chat、Knowledge、Collab、Asset 和 View 页面主体。
- 后续在同一资源模型上增加编辑器标签、分屏、跨窗口拖动和恢复。
- 项目资源更新按 projectId 路由，执行、文件读取和服务事件按 checkoutId 隔离。
- 两个 worktree 可以同时运行会话、Unity、知识索引和 View，并在前端独立显示状态。

## 非目标

- 初始页面重构不同时完成通用窗口管理器。
- 虚拟目录不替代磁盘目录、Git worktree、知识类型或会话父子关系。
- 初期不允许通过普通拖动把资源迁移到另一个 ProjectContext。
- 初期不在资源树中展开全部 Unity Assets；资产分区先打开现有 Asset 页面。
- 初期不重做 Chat、Knowledge、Collab、Asset 和 View 的正文视觉设计。
- 初期不引入资源别名或同一资源的多处快捷方式。
- 进程级 Agent 和 Plugin 页面不承担 checkout 焦点切换。
- 共享资源不共享 runtime、文件句柄、索引实例、预览缓存或 Unity 连接。

## 首个验收节点：主窗口内拖动分屏

第一个对用户开放验收的完整闭环是主窗口内的拖动分屏。EditorInput、Workbench store、EditorGroup 和 pane context 等基础能力作为该闭环的内部实现步骤，不单独作为更早的用户验收节点。

验收路径：

1. 用户从左侧资源树拖动会话、知识文档或其他可打开资源到主编辑区边缘。
2. 编辑区显示明确的左、右、上、下分屏落点；释放后创建新的 EditorGroup。
3. 原编辑器和目标资源同时可见，每个 EditorGroup 使用独立稳定的 `paneId`。
4. 用户拖动两个 EditorGroup 之间的分隔条，实时调整宽度或高度；释放后保存比例。
5. EditorGroup 内只有一个分页时隐藏 tab strip，正文顶部与当前界面一致；同组打开第二个分页后才显示标签栏。
6. 显示标签栏后，用户可以把已打开的标签拖到另一个 EditorGroup，或拖到组边缘继续拆分。
7. 两个 pane 可以分别显示不同会话、会话与知识文档，或属于不同 checkout 的资源；消息、流式状态、模型上下文和 workspace scope 保持独立。
8. 关闭并重新打开应用后恢复主窗口的 split tree、比例、pane ID、标签和活动标签；失效资源显示可关闭的 unavailable editor。

首个验收节点的范围包括：

- 主窗口内水平与垂直分屏。
- 从资源树拖入分屏，以及标签在组间移动和继续拆分。
- 可拖动分隔条、最小 pane 尺寸、窄窗口约束和键盘可访问的 separator 语义。
- 每个 EditorGroup 按分页数量显示标签栏：一个分页保持当前无 tab 的界面，两个及以上分页显示 tab strip。
- Session、Knowledge document 和本地文件三类 Editor Adapter；至少验证双会话、会话与知识文档两种组合。
- `(windowId, paneId)` workspace focus、active session、VisiblePane lease 和 scoped event 隔离。
- 主窗口布局的版本化持久化与恢复。

原生独立窗口、跨 WebView 拖放、拖出主窗口和拖回主窗口属于后续通用 WorkbenchWindow 验收节点。

## 术语与身份

- `ProjectContext`：同一逻辑 Git 项目。主工作区和多个 worktree 共享 `projectId`、资源目录和用户布局。
- `ProjectResourceHub`：ProjectContext 内共享的 session、knowledge、collaboration 和 logical asset catalog。
- `Checkout`：一个具体工作目录，使用稳定 `checkoutId` 标识。
- `WorkspaceRuntime`：checkout 当前进程内的运行实例，具有会变化的 `generation`。
- `WorkspaceRef`：调用 checkout 级命令时使用的 `{ checkoutId, expectedGeneration? }`。
- `ProjectRef`：调用 project 级共享资源和布局命令时使用的 `{ projectId, expectedRevision? }`。
- `ExplorerLayout`：用户对某个 ProjectContext 共享资源树的虚拟布局，和物理路径无关。
- `WorkbenchResourceRef`：右侧编辑器打开的稳定资源身份。
- `CheckoutBinding`：某个 editor 或 run 当前读取、执行的具体 worktree。
- `EditorGroup`：一个可独立聚焦 checkout 的编辑器分组，对应一个 `paneId`。
- `WorkbenchWindow`：包含一个或多个 EditorGroup 的 Locus 窗口，对应一个 `windowId`。

`projectId + resource ID` 是共享资源的持久身份，`checkoutId` 是执行和 materialization 身份。runtime generation、service generation、文件路径和标题只作为运行时校验或展示信息，不作为树节点主键。

## 目标信息架构

### 一级导航

```text
┌──────────────────────────────────────────────────────────────────┐
│ Locus   开发   插件   Agent   设置                     窗口控制 │
├──────────────────────────────────────────────────────────────────┤
│                         当前一级页面                             │
└──────────────────────────────────────────────────────────────────┘
```

- “开发”替代当前“项目”。
- 删除 `project-tab-bar`、`ProjectPageTab`、`projectTabs` 和对应上下文菜单。
- 删除标题栏 checkout 下拉框。打开、聚焦、固定和移除工作区的入口迁入开发页资源树工具栏及 checkout 节点菜单。
- `app.tab.project` 与 `app.tab.dev` 收敛为 `app.tab.development`，中文为“开发”，英文为“Development”。
- 应用级 `activeTab` 最终只包含 `development / plugins / agent / settings`。
- 历史持久值 `project / chat / collab / knowledge / asset / views` 在读取时统一迁移为 `development`。

### 开发页布局

```text
┌───────────────────┬──────────────────────────────────────────────┐
│ 单工作区  多工作区 │ 编辑器组                                     │
│                   │ ┌──────────────────────────────────────────┐ │
│ 工作区资源树       │ │ 初期：单个活动资源                        │ │
│                   │ │ 后续：资源标签、分屏、拖出窗口            │ │
│                   │ └──────────────────────────────────────────┘ │
└───────────────────┴──────────────────────────────────────────────┘
```

资源树沿用桌面 IDE 的窄工具栏、细边框、低对比 hover 和现有设计 token。模式切换使用 `BaseSegmented` 或同等级轻量控件。打开工作区、折叠全部和更多操作使用现有图标按钮语法。

### 单工作区模式

单工作区模式隐藏 ProjectContext 外层，只投影当前 ProjectContext 的共享 Workspace Tree：

```text
middle-unity-test
├─ 会话
│  ├─ 新建会话
│  ├─ Gameplay
│  │  ├─ 修复移动状态机
│  │  └─ 调查动画卡顿
│  └─ 构建 Addressables
├─ 知识
├─ 协作
├─ 资产
└─ 视图
```

- 默认使用该模式，保持现有单工作区用户的认知成本。
- 【知识】与【协作】继续是 ProjectContext 级公共入口，在同一项目内保持唯一。
- 当前 checkout 只作为右侧 EditorGroup 的执行绑定，不改变共享树节点的身份和位置。
- 切换具体 worktree 通过【协作】下的 checkout 节点或“切换工作区”命令完成。
- 模式切换只改变资源树投影，不关闭 runtime，也不终止后台任务。

### 多工作区模式

多工作区模式为每个 ProjectContext 增加项目根节点，根节点内部继续显示同一棵共享 Workspace Tree：

```text
middle-unity-test                         ProjectContext
├─ 新建会话
├─ Gameplay                               虚拟文件夹
│  ├─ 修复移动状态机                      Session
│  └─ 战斗规则.md                         Knowledge document
├─ 知识                                   ProjectContext 共享入口
├─ 协作                                   ProjectContext 共享入口
│  ├─ main                                Checkout
│  └─ feature/hot-reload                  Checkout
├─ 资产
└─ 视图

locus                                     ProjectContext
├─ 新建会话
├─ 知识
├─ 协作
│  └─ main
└─ ...
```

- `ProjectContext` 根节点和【协作】下的 checkout 子节点由 `ProjectRegistry` 投影，不能被改造成普通虚拟文件夹。
- 多工作区模式只增加 ProjectContext 根层级，不把共享树复制到每个 worktree 下。
- 【知识】、【协作】、会话和虚拟布局继续以 `projectId` 为共享归属；现有左侧树的节点顺序与拖动行为保持不变。
- 展开根节点只加载资源摘要，不自动 focus checkout，不启动 Unity，不抢占正在运行的 pane。
- 点击共享资源并进入右侧编辑器时，Editor Adapter 根据当前 pane、会话执行目标或资源可用 checkout 解析独立 `CheckoutBinding`。
- 点击【协作】下的 worktree 节点时，才把该 checkout 聚焦到目标 pane，并打开对应 worktree 的协作上下文。
- 同一窗口的不同编辑器组最终可以同时绑定不同 checkout。
- 后台正在运行的会话或服务保留轻量状态标记；共享知识与协作节点始终保持单一 ProjectContext 身份。

## 节点类型与点击行为

| 节点 | 身份来源 | 是否可移动 | 点击后的右侧内容 |
| --- | --- | --- | --- |
| ProjectContext | `projectId` | 只可调整根顺序 | 项目及 checkout 摘要 |
| Checkout | `projectId + checkoutId` | 结构节点 | 具体 worktree 的协作上下文、分支、路径和服务状态 |
| 固定分区 | `projectId + sectionKind` | 可在 ProjectContext 共享树内排序 | 对应分区首页或现有页面主体 |
| 虚拟文件夹 | `projectId + explorer node UUID` | 可在同一 ProjectContext 预设内移动 | 文件夹子项列表与操作 |
| 会话 | `projectId + sessionId` | 可在 ProjectContext 共享树内移动 | Chat 正文 |
| 知识 | `projectId + documentId` | 可在 ProjectContext 共享树内布局 | 现有完整 KnowledgeView 或具体文档 |
| 本地文件 / 目录挂载 | explorer node UUID + path | 可在当前预设内移动 | 通用文件预览 / Unity 资产预览 |
| View | View ID | 可在视图分区内移动 | View 运行或配置页面 |
| 协作 | `projectId + collab` | 共享固定分区；checkout 子节点保持结构位置 | ProjectContext 协作入口与 worktree 子上下文 |
| 资产 | 固定分区 | 不预载全部资产 | 现有 Asset 页面 |

展开箭头只负责折叠。点击节点行负责选中并打开内容，避免一次操作同时改变两种状态。双击资源固定为编辑器标签；单击可在后续标签阶段使用 preview tab 语义。

### 固定分区

Workspace Tree 使用以下固定分区语义：

```ts
type WorkspaceSectionKind =
  | "sessions"
  | "knowledge"
  | "collab"
  | "assets"
  | "views";
```

- `knowledge` 与 `collab` 属于 ProjectContext，共享于该项目的主工作树和全部 worktree；左侧树各显示一个节点。
- `sessions` 与虚拟资源布局继续使用当前 ProjectContext 级预设，不按 checkout 复制。
- checkout 级资源或服务状态由 Editor Adapter 和 `CheckoutBinding` 投影，不改变共享节点的布局所有权。
- 分区可以排序、折叠，并通过显示设置隐藏。
- 分区名称不可重命名，不能移出所属 ProjectContext，不能放入另一个分区。
- 当前 `showKnowledgeTab / showCollabTab / showAssetTab / showViewsTab` 迁移为分区可见性设置。
- `showViewsInSessionPanel` 在统一资源树完成后废弃。
- checkout 级 Agent / Plugin overlay、Unity 和扫描状态先进入 checkout 概览；需要独立页面时再增加稳定分区，避免先制造空节点。

## 虚拟树语义

### 用户操作只改变视图

普通拖动、创建文件夹、文件夹重命名、排序和删除虚拟文件夹只修改 ExplorerLayout：

- 不调用知识物理移动接口。
- 不修改知识文档文件路径。
- 不修改知识类型 `Design / Memory / Reference / Skill`。
- 不修改 session `parentSessionId`、`session_type` 或 checkout 归属。
- 不移动 View Package 目录。
- 不修改 Git worktree 或 checkout 根目录。
- 不切换全局 working directory。

需要改变实体本身时使用明确的领域操作，例如“重命名会话”“修改知识类型”“在磁盘中移动”“删除 View Package”。这些命令和树布局命令使用不同的 IPC，不共享拖动处理器。

### 允许的拖动

- 虚拟文件夹和资源可在同一 ProjectContext 预设内移动与排序。
- 固定分区可在同一 ProjectContext 共享树内调整顺序。
- 【协作】下的 checkout 是结构投影，不参与普通虚拟布局拖动。
- ProjectContext 可调整显示顺序。
- 多选移动作为一个原子操作提交。
- 拖入自身或后代、非法跨分区、跨 ProjectContext 和未知目标会被前端预判并由后端再次拒绝。
- worktree 切换只改变 EditorGroup 的 `CheckoutBinding`；普通树拖动不迁移 checkout，也不复制【知识】或【协作】节点。

### 删除与重命名

- 删除虚拟文件夹时，默认把直接子项原序提升到父级，再删除文件夹；操作在一个事务中完成。
- 空文件夹可以直接删除。
- 资源节点名称来自真实资源标题。F2 对资源调用领域重命名，对文件夹调用布局重命名。
- 初期每个资源只有一个规范树节点。同一资源的别名和快捷方式延后设计。

### 初始布局与兼容导入

- 新 checkout 首次打开时按资源当前状态生成默认布局。
- 现有会话文件夹记录只用于一次性导入默认树；导入不修改或删除历史 session 行。
- 新建虚拟会话文件夹只写 ExplorerLayout，不创建 `session_type = folder` 的伪会话。
- 内嵌知识默认只显示一个固定“知识”入口，点击后在右侧打开现有完整 KnowledgeView。
- 用户主动挂载的知识目录作为独立虚拟文件夹显示，不改变内嵌知识分类与文档路径。
- 现有 View `displayPath` 和 ViewTreeMetadata 用于首次导入；统一资源树成为新的显示布局来源后，不再由普通树拖动反写 View metadata。
- 没有布局记录的新资源进入所属分区根部，并按最近更新时间排序；首次用户移动后转为显式顺序。
- 资源归档时保留布局位置并从默认活动视图隐藏；恢复时回到原位置。
- 资源永久删除后由 scoped event 标记并清理孤儿 placement。

## 稳定资源引用

前端使用可判别联合类型表达编辑器输入：

```ts
type WorkbenchResourceRef =
  | { kind: "project"; projectId: string }
  | { kind: "checkout"; projectId: string; checkoutId: string }
  | { kind: "section"; projectId: string; section: WorkspaceSectionKind }
  | { kind: "folder"; projectId: string; nodeId: string }
  | { kind: "session"; projectId: string; sessionId: string }
  | { kind: "knowledge"; projectId: string; documentId: string }
  | { kind: "view"; projectId: string; viewId: string }
  | { kind: "localFile"; projectId: string; nodeId: string };

interface EditorCheckoutBinding {
  checkoutId: string;
  expectedGeneration?: number;
}
```

规则：

- 持久化的 `WorkbenchResourceRef` 保存 ProjectContext 级稳定 ID，不保存 generation，也不把 worktree 当作【知识】或【协作】的资源身份。
- `EditorCheckoutBinding` 属于 EditorGroup 的运行时绑定；打开或恢复时通过 `workspaceContextStore` 获取当前 runtime，再构造 live `WorkspaceRef`。
- 同一知识文档在多个 worktree 下保持同一个 `projectId + documentId`；需要 checkout 级索引、文件预览或工具调用时单独解析 EditorCheckoutBinding。
- 【协作】使用 `projectId` 作为共享入口身份，具体 worktree 子页面通过 `checkoutId` 选择 working tree 状态。
- 会话命令继续以 `sessionId` 在后端解析真实执行目标；pane binding 负责当前编辑器需要读取的 worktree 上下文。
- View 命令以 `viewId` 解析 checkout；Knowledge 与 Asset 命令使用 `WorkspaceRef + resourceId`。
- 标题、物理路径、分支名和图标均为可更新元数据，不参与引用相等判断。
- runtime generation 失效时重新解析资源并恢复编辑器；资源已经删除时显示可关闭的 unavailable editor，不把旧 generation 写回。

## ExplorerLayout 持久化

### 存储位置

虚拟布局以工作目录内的文本文件为权威来源。每个 ProjectContext 使用：

```text
Locus/workspace-trees/
├─ index.json
├─ default.json
├─ preset-<uuid>.json
└─ preset-<uuid>.json
```

- `index.json` 只保存当前激活预设与预设展示顺序。
- 每套预设使用独立 JSON 文件，保存完整节点树、schema version、稳定 preset ID、名称、revision 和最后一次 operation ID。
- 预设重命名只更新文件内名称，文件名继续使用稳定 preset ID。
- Locus 通过目录扫描列出预设；用户、Agent 与外部工具可以直接读取、审查和整理这些文件。
- 写入使用同目录临时文件与原子替换，避免进程中断留下半份布局。
- 布局文件格式错误时返回文件路径与字段错误，原文件保持可修复状态。
- Git 项目优先把声明写入主工作树；已存在声明文件的工作树保持权威位置。

预设文件示意：

```json
{
  "schemaVersion": 1,
  "presetId": "default",
  "name": "Default",
  "projectId": "project-...",
  "revision": 12,
  "lastOperationId": "...",
  "nodes": [
    {
      "nodeId": "folder:...",
      "projectId": "project-...",
      "nodeKind": "folder",
      "folderName": "Gameplay",
      "position": 0,
      "hidden": false
    }
  ]
}
```

`mode`、展开节点、当前选择、侧栏宽度和滚动位置属于窗口 UI 状态。初始版本使用带 schema 版本的 local storage，以 `windowId` 分区；通用窗口恢复实现后迁入 WorkbenchWindowState。

### 数据不变量

- 一个 placement 只属于一个 ProjectContext 和一个 preset。
- `parentNodeId` 必须指向同一预设内的 folder。
- 文件夹图无环。
- 同一预设中，一个真实资源或规范化本地路径最多有一个 placement。
- 同级 `position` 在写回前统一整理为唯一且连续的序列。
- 固定分区由代码和 layout header 生成，不作为可删除普通节点保存。
- 每个预设维护独立 revision；切换预设只更新 `index.json`。
- layout revision 每次成功变更递增一次。
- 相同 `lastOperationId` 的重试返回已提交 revision。
- `hidden` 只控制树投影，资源、物理文件和后台任务保持原状态。

### 多预设规则

- 一个工作区至少保留一套预设。
- 新建预设默认复制当前预设，生成新的稳定 preset ID 与独立文件。
- 预设切换立即重新投影资源树，并继续使用相同的资源稳定 ID。
- 删除当前预设后按 `index.json` 顺序激活下一套预设。
- 每个窗口收到 `projectId + presetId + revision` 事件后重载匹配 ProjectContext。

### 本地文件与目录挂载

- 本地文件使用规范化绝对路径作为 locator，placement 使用独立 node UUID。
- 目录使用 `local_directory` placement 挂载，磁盘子树按需列出并显示为虚拟文件夹。
- `Locus/knowledge` 或用户明确选择的知识目录使用 `sourceKind = knowledge`，整棵目录作为一个挂载节点进入工作区树。
- 挂载、移动、隐藏、重命名显示名和移除挂载只修改预设文件。
- 移除挂载保留源文件和源目录。
- 预览命令只读取当前预设直接引用的文件或已挂载目录的后代。
- 项目内 `Assets / Packages / ProjectSettings` 文件交给 Locus Unity Asset Preview；文本、图片、PDF、音频、视频和普通二进制文件使用通用预览。

## 后端接口

`src-tauri/src/commands/workspace_explorer.rs` 通过 ProjectRegistry 解析 ProjectContext 与声明文件所在工作目录：

```ts
project_explorer_snapshot(projectId): Promise<ProjectExplorerSnapshot>

project_explorer_apply_operations(input: {
  projectId: string;
  expectedRevision: number;
  operationId: string;
  operations: ProjectExplorerOperation[];
}): Promise<ProjectExplorerMutationResult>

project_explorer_create_preset(projectId, name, sourcePresetId?)
project_explorer_switch_preset(projectId, presetId)
project_explorer_rename_preset(projectId, presetId, name)
project_explorer_delete_preset(projectId, presetId)

project_explorer_list_mount(projectId, nodeId)
project_explorer_preview_file(projectId, path)
```

操作联合类型：

```ts
type ProjectExplorerOperation =
  | { kind: "createFolder"; parentNodeId?: string; name: string; position: number }
  | { kind: "renameFolder"; nodeId: string; name: string }
  | { kind: "deleteFolder"; nodeId: string }
  | { kind: "moveNode"; nodeId: string; parentNodeId?: string; position: number }
  | { kind: "placeResource"; resourceKind: string; resourceId: string; parentNodeId?: string; position: number }
  | { kind: "removeResourcePlacement"; resourceKind: string; resourceId: string }
  | { kind: "mountPath"; parentNodeId?: string; path: string; sourceKind: "local" | "knowledge"; position: number }
  | { kind: "setNodeHidden"; nodeId: string; hidden: boolean }
  | { kind: "removeNode"; nodeId: string };
```

### 并发与事务

- 每个声明目录使用独立 mutation lock；不同 ProjectContext 可并发提交。
- `expectedRevision` 与当前预设文件比较，旧 revision 返回结构化 `RevisionConflict`。
- 读取、校验、移动、同级重排、revision 更新和原子写回属于一次提交。
- 后端重复校验 checkout、section、parent、cycle 和资源归属，不能信任前端拖动状态。
- 前端收到 conflict 后重新拉取 snapshot；只有源节点和目标节点仍存在时才重放用户意图。
- 每次请求保留前端 intent epoch。后发请求完成后，先发慢响应不能覆盖新 snapshot。
- layout 修改不获取 checkout runtime 写锁，也不触发 Unity、Knowledge rebuild 或 Agent reload。

### 事件

成功变更发出 ProjectContext 级事件：

```ts
type WorkspaceExplorerChanged = {
  projectId: string;
  presetId: string;
  revision: number;
  operationId: string;
};
```

前端 reducer 按 `projectId + presetId` 更新；预设切换可以接收低于旧预设的 revision，并通过 preset ID 判定重新加载。

## 资源提供器

ExplorerLayout 只保存布局与本地路径 locator。真实标题、状态、是否归档和可执行能力由独立 provider 提供，避免复制业务实体。

### Sessions provider

- 按 ProjectContext 列出活动与归档会话，返回 session ID、title、type、parent session、updatedAt、执行目标和运行状态。
- 新建会话后立即在 Sessions 根部插入默认 placement，并通过 scoped event 更新。
- 点击 session 复用 `ChatWorkspaceView`，传入 `showSessionNavigation=false`。
- `SessionPanel` 的会话导航逐步退役；Chat 正文、输入框、Diff 和运行状态继续复用。
- session 的实际 `parentSessionId` 保持领域关系。默认导入可以按该关系生成层级，用户后续拖动只改变 placement。
- 子 Agent 会话移动后仍通过详情和图标保留来源关系；执行上下文继续由 session ID 解析。

### Knowledge provider

- 每个 ProjectContext 的工作区树只生成一个固定“知识”节点，主工作树和其他 worktree 共用该入口与文档 placement。
- 点击“知识”挂载现有完整 `KnowledgeView`，保留其分类、目录、检索和文档操作能力。
- KnowledgeExplorer 的文档与普通文件夹可以拖到左侧工作区树；文档生成稳定知识 placement，文件夹生成 `sourceKind = knowledge` 目录挂载。
- 知识面板内部拖动继续执行知识领域移动；拖到工作区树使用 copy 语义并只修改当前 tree preset。
- 深链接到具体知识文档时继续使用 `KnowledgeDocumentSummary.id`，path 只作为可更新 locator。
- 用户主动挂载的知识目录通过 `sourceKind = knowledge` 进入虚拟树，使用通用目录与文件预览。
- `KnowledgeExplorer.vue` 的领域操作继续管理真实知识类型、路径和文档。
- Design、Memory、Reference、Skill 继续是知识语义；虚拟目录位置不改变知识类型。
- 知识文档使用 `projectId + documentId` 去重；切换 pane 的 checkout binding 不复制节点、标签或文档模型。

### Collab provider

- 每个 ProjectContext 只有一个固定“协作”节点，点击挂载 ProjectContext 级 `CollabView`。
- “协作”节点按当前 Workspace Tree 行为展开该项目的 worktree / checkout 子节点；这些子节点选择具体 working tree 上下文，不复制协作入口。
- ProjectContext 级历史、分支图和共享协作信息按 `projectId` 路由；staging、冲突、工作目录状态等 worktree 操作使用显式 `CheckoutBinding`。
- checkout scope 由活动 EditorGroup 或被点击的 worktree 子节点传入，不能读取全局 active project。
- 协作对象稳定 ID 明确后再增加子节点，避免提前把瞬态任务写入 layout。

### Assets provider

- 初期只有固定分区节点，点击挂载现有 `AssetView`。
- AssetDB、scan phase、preview 和 Unity 状态从 `workspaceState[checkoutId]` 读取。
- 资产规模大，资源树不承担全量 AssetDB 浏览；后续使用懒加载、筛选和搜索型子树。

### Views provider

- 使用 View ID 作为资源身份。
- 首次从现有 `displayPath` / ViewTreeMetadata 导入逻辑目录。
- 将 `SessionPanel` 中的 View tree 操作提取为 provider 和现有 View 领域命令。
- 统一 Explorer 切换后，普通拖动只写 ExplorerLayout；View Package 物理目录保持不变。
- 点击 View 复用 `ViewPackageView` 或 View host；现有“在独立窗口打开”在通用工作台窗口完成前继续可用。

## 前端状态架构

### 应用级状态

```text
AppNavigationStore
└─ activePage: development | plugins | agent | settings

WorkspaceContextStore
├─ projects[projectId]
├─ checkouts[checkoutId]
├─ runtimes[checkoutId]
├─ workspaceState[checkoutId]
└─ paneContexts[windowId:paneId]
```

`WorkspaceContextStore` 继续负责后端身份、runtime、事件投影和 pane focus。资源树不能复制一份“当前 workingDir”。

### Explorer 状态

```text
WorkspaceExplorerStore
├─ modeByWindow[windowId]
├─ rootPreferences
├─ snapshots[projectId]
├─ providerState[projectId][section]
├─ checkoutProjection[checkoutId]
├─ expandedNodeIdsByWindow[windowId]
├─ selectedNodeByWindow[windowId]
├─ loading / error / requestEpoch
└─ dragIntent
```

- Explorer snapshot、【知识】、【协作】和共享资源 provider 按 ProjectContext 隔离。
- checkoutProjection 只保存 worktree 级 runtime、Unity、文件状态和服务投影，不拥有共享节点布局。
- 单工作区 / 多工作区是窗口投影状态。
- 多窗口共享持久 layout revision，各自保存展开和选择状态。
- provider 首次展开时懒加载；ProjectContext 资源事件只使对应 `projectId / section` 失效，runtime 事件只使对应 checkoutProjection 失效。
- 反向完成的请求分别通过 project 级和 checkout 级 request epoch 丢弃。

### Workbench 状态

初始版本：

```text
DevelopmentWorkbenchState
└─ activeResourceByWindow[windowId]
```

窗口化阶段：

```text
WorkbenchWindowState[windowId]
├─ sidebar
├─ groupLayout
└─ groups[paneId]
   ├─ tabs[]
   ├─ activeTabId
   └─ focusedCheckoutId
```

Explorer selection 和 active editor 分开保存。折叠、右键或拖动树节点不会意外切换正在执行的编辑器。

### 单工作区 checkout 持久化事务

单工作区的持久化身份固定为 `windowId + checkoutId`。每个 scoped layout 只保存该 checkout 的 EditorInput、活动 EditorGroup 与 `focusedCheckoutId`；切换期间由显式目标 checkout 驱动状态，不从正在恢复的 pane context 反向推导目标 scope。

已确认的故障链路：workspace 切换先改变 pane context，scope watcher 恢复目标布局后又从旧 pane context 推导出上一个 checkout；过期同步任务继续创建 EditorInput，`openEditor` 随后按照已经变化的 scope 写入，形成 `存储 key = A / editor checkout = B`。外部 View、Locus Inspector、跨窗口 tab transfer 与跨 checkout 引用拖入也可以在 editor 写入早于 scope 路由时触发同类问题。

修复要求：

- 使用显式 single-workspace scope 作为切换事务的唯一目标；`scope → layout → pane context → active editor` 完成前，旧事务不能继续写 editor。
- 每次异步返回后、每次 `openEditor / replaceEditor / splitPane / updateEditor / acceptTransferredEditor` 前检查目标 scope 与事务 epoch。
- 外部 View、Inspector、跨窗口 transfer 和引用拖入先切换或路由到来源 checkout 的 scoped layout，再写入 EditorInput。
- store 写入层强制 `tab.checkoutBinding.checkoutId === scopeId`，同时约束 `group.focusedCheckoutId`；违规写入保留现场错误并停止覆盖持久化数据。
- 启动时扫描历史 scoped layout，按 EditorInput 的 durable checkout binding 重新归位错槽数据；正确槽优先保留，错槽中的可恢复标签合并到真实 checkout，空槽恢复为空布局。
- 回归测试覆盖首次打开空 workspace、A→B→C 反向完成、错槽迁移、混合 checkout 拒写、外部 View/Inspector、跨窗口 transfer 和重启恢复。

## 前端组件结构

建议新增：

```text
src/components/workbench/
├─ DevelopmentWorkbench.vue
├─ WorkspaceExplorerPanel.vue
├─ WorkspaceExplorerTree.vue
├─ WorkspaceExplorerToolbar.vue
├─ WorkbenchEditorHost.vue
├─ CheckoutOverview.vue
├─ SectionOverview.vue
└─ FolderOverview.vue

src/stores/
├─ workspaceExplorer.ts
└─ workbench.ts                 窗口化阶段

src/services/
└─ workspaceExplorer.ts

src/types/
└─ workbench.ts
```

复用与拆分：

- 复用 `components/explorer/WorkspaceTree.vue` 和 `FileTreeList.vue` 的虚拟化、键盘焦点、展开和 drag 事件。
- 扩展 WorkspaceTree 的语义 icon slot 和 row metadata，不复制一套工作台专用树控件。
- `ChatWorkspaceView` 使用现有 `showSessionNavigation=false` 能力。
- `KnowledgeView` 增加 embedded/editor 模式，避免嵌套第二棵 Explorer。
- Collab、Asset 和 View 先作为 WorkbenchEditorHost 的 adapter 挂载。
- `SessionPanel` 中可复用的 session / View provider 逻辑抽成 composable 或 store，页面 CSS 和菜单不直接复制。

## 页面路由迁移

### `App.vue`

目标类型：

```ts
type ProcessTab = "development" | "plugins" | "agent" | "settings";

const topTabs = computed(() => [
  { id: "development", labelKey: "app.tab.development", visible: true },
  { id: "plugins", labelKey: "app.tab.plugins", visible: ... },
  { id: "agent", labelKey: "app.tab.agent", visible: ... },
  { id: "settings", labelKey: "app.tab.settings", visible: true },
]);
```

删除：

- `ProjectPageTab`、`ProjectTabItem`、`projectTabs`、`visibleProjectTabs`。
- `isProjectPageTab`、project tab fallback watch 和 project tab context menu。
- `.project-tab-bar`、`.project-tab-context` 及对应样式。
- 标题栏 checkout selector 的视图与基于它的焦点写入。
- `ChatWorkspaceView / KnowledgeView / CollabView / AssetView / ViewPackageView` 在 App 根部的平行挂载。

新增：

- `DevelopmentWorkbench` 作为 `development` 唯一内容根。
- App 一级导航只切换进程级页面。
- 当前 workspace / resource 标题由 Explorer 和 EditorGroup 自己表达。

### `uiStore`

- `activeTab` 改名为 `activePage`。
- 读取旧值时执行版本化 migration。
- Chat 内部不得再用 `uiStore.activeTab === "chat"` 判断可见性，改为 workbench editor activation / deactivation 生命周期。
- 会话、知识等当前资源放入 Workbench store，不再占用 app route。

## 初始可验证闭环

第一个可交付版本需要同时满足以下流程：

1. 启动 Locus，一级导航显示“开发 / 插件 / Agent / 设置”，界面中没有第二层栏和标题栏工作区选择器。
2. 开发页默认进入单工作区模式，左侧显示当前 ProjectContext 的共享 Workspace Tree。
3. 点击会话后，右侧显示现有 Chat；运行中的会话继续执行。
4. 点击“知识”后，右侧显示现有完整知识面板；挂载知识目录后可以直接浏览其中的文件。
5. 点击协作、资产和视图分区后，右侧能进入现有功能主体。
6. 切换多工作区模式后，为每个 ProjectContext 增加项目根节点；同一项目的【知识】与【协作】继续各显示一次，【协作】下列出主工作树与其他 worktree。
7. 打开共享知识文档或协作入口时保持同一 ResourceRef；选择具体 worktree 后，checkout 级 IPC 和事件落入对应 EditorCheckoutBinding。
8. 创建虚拟文件夹、移动会话、挂载本地文件或知识目录，重启应用后布局保持。
9. 拖动会话不会改变 `parentSessionId`；拖动 View 不会改变 package 目录或 `displayPath`。
10. A checkout 运行会话时浏览 B checkout，不改变 A 的 AgentExecutionContext 和服务绑定。

该闭环作为首个主窗口拖动分屏验收的内部前置条件，不单独改变或重新验收左侧 Workspace Tree 行为。

## 编辑器标签、分屏与窗口化演进

### EditorInput

后续所有右侧页面统一为 EditorInput：

```ts
interface EditorInput {
  editorId: string;
  resource: WorkbenchResourceRef;
  title: string;
  icon?: string;
  preview: boolean;
  pinned: boolean;
  dirty: boolean;
  capabilities: {
    split: boolean;
    detach: boolean;
    duplicate: boolean;
  };
}
```

- 单击树节点打开 preview tab；再次单击替换同组 preview。
- 双击、编辑内容或显式固定后成为 pinned tab。
- 同一 group 内默认对同一资源去重。
- 跨 group 可以打开同一只读资源；可写资源通过共享 document model 同步 dirty 状态。
- section 和 checkout overview 也可以成为 EditorInput，但默认保持 preview。
- tab strip 的显示条件按 EditorGroup 独立计算：`tabs.length >= 2` 时显示，`tabs.length === 1` 时完全收起。
- 单分页 EditorGroup 不预留标签栏高度，不常驻标题、关闭按钮或额外 header；EditorHost 正文沿用当前顶部位置和视觉结构。
- 同一窗口分屏后，每个只有一个分页的 EditorGroup 都保持无标签栏形态，pane 之间只显示必要的分隔条。
- 加入第二个分页时显示 tab strip；关闭或移动分页后剩余一个时自动收起。显示状态由 `tabs.length` 派生，不单独持久化。
- tab strip 显隐过程中保持活动 Editor 实例、滚动位置、输入草稿和流式渲染状态，不通过卸载正文实现切换。
- pane focus、编辑区边缘 drop zone、分隔调整和快捷命令独立于 tab strip；单分页状态仍可从资源树拖入拆分，并可通过“移动编辑器 / 拆分编辑器”命令操作。

### 分屏

- group layout 使用二叉 split tree，方向为 horizontal / vertical，叶子是 `paneId`。
- 每个叶子 pane 对应 `WindowContextRegistry` 的一个 PaneContext。
- 激活 tab 时，根据资源重新解析 checkout 并调用 `focus_workspace(windowId, paneId, checkoutId)`。
- 一个窗口内可以让左 pane 显示绑定 checkout A 的会话、右 pane 显示同一 ProjectContext 的共享知识文档并按需绑定 checkout B；知识资源身份保持唯一。
- 关闭最后一个 checkout 资源 tab 后释放该 pane 的 runtime lease。
- 运行中会话 lease 由任务所有权持有，不依赖 tab 是否可见。

### 拖出窗口

通用窗口使用两阶段交接：

1. 源窗口创建 transfer token，保留原 tab。
2. 创建目标 WorkbenchWindow 和目标 pane。
3. 目标窗口恢复 ResourceRef、解析 runtime、完成 editor ready ack。
4. 源窗口收到 ack 后移除原 tab。
5. 创建失败或超时则取消 transfer，原 tab 保留。

该协议可参考现有 View host 的 tab merge/detach 和 hidden content pool；抽取通用协议后再迁移 ChatSessionWindow、WorkspacePageWindow 和 ViewHostWindow。不能通过先关闭源 tab 再创建窗口实现。

### 恢复

- 持久化 window bounds、sidebar 状态、split tree、pane ID、tab ResourceRef、active tab 和 preview/pinned 状态。
- 不持久化 runtime generation、WebView handle、Unity connection handle 或进行中的 drag token。
- 启动时先恢复窗口骨架，再逐个解析资源；缺失资源显示 unavailable tab。
- 只有可见 pane 和运行中任务获取高优先级 runtime lease；后台恢复 tab 延迟 hydrate。

## Runtime 与服务活动度

资源树操作和 runtime focus 必须分离：

| 行为 | Runtime 行为 |
| --- | --- |
| 展开 ProjectContext / checkout | 只读 registry metadata |
| 展开 Sessions | 读取 SessionStore，不启动 Unity |
| 展开 Knowledge | 懒加载 runtime knowledge provider；保持后台优先级 |
| 单击资源并打开 editor | 目标 pane focus checkout，获取 VisiblePane lease |
| 会话开始执行 | 获取 RunningTask lease |
| tab 移到后台 | 降为 BackgroundOpen |
| 关闭最后一个 pane/tab 且无任务 | 进入 Idle 并按策略释放 |

Unity 节点状态使用 `Starting -> Connected -> Ready -> Reloading/Degraded -> Stopped`。界面可以在 Connected 时显示连接存在，执行 Unity 命令必须等待 checkout 级 Ready。资源树状态事件始终保留 service instance 和 generation。

## 分阶段实施

### P0：导航与数据契约

目标：建立新页面边界和不会返工的稳定类型。

- 增加 `WorkbenchResourceRef`、section kind、snapshot 和 operation 类型。
- 增加独立预设文件、index、原子写回、revision CAS 和 Rust 单元测试。
- 增加 workspace explorer IPC、revision CAS、cycle / ownership 校验和 scoped event。
- 增加 `workspaceExplorer` service / store，接入 checkout 级 request epoch。
- 将一级“项目”改为“开发”，删除第二层栏的数据结构和样式。
- 引入 `DevelopmentWorkbench`，初始右侧继续挂载当前 focused session。
- 将旧 `activeTab` 值迁移到 `development`。
- 第二层栏没有运行时回退分支。开发阶段通过小步提交和隔离实例验证控制风险，主渲染路径始终只有一级导航。

完成条件：应用只显示一层导航，当前 ProjectContext 的会话可以从共享 Workspace Tree 打开并在解析出的 checkout 中正常执行。

### P1：单工作区资源树闭环

目标：替换 SessionPanel 的主导航职责。

- 使用 WorkspaceTree / FileTreeList 实现 ProjectContext 共享树、固定分区、folder、session，以及【协作】下的 checkout 结构节点。
- 接入单工作区模式、展开状态、键盘导航、上下文菜单和虚拟文件夹。
- 接入预设创建、复制、切换、重命名与删除；切换与管理入口统一放入三个点菜单，工具栏保持单行。
- 接入节点隐藏、本地文件拖入、知识目录整体挂载与按需子树列出。
- 文本、图片、PDF、音视频和 Unity 资产使用对应预览器。
- Chat 以 `showSessionNavigation=false` 嵌入 EditorHost。
- 实现 session 默认布局、legacy folder 导入、archive / restore reconcile。
- 将“新建会话”迁入 Sessions 分区工具项或上下文菜单。
- 【协作】下的 checkout 节点打开 worktree Overview，并显示分支、物理路径和 scoped service 状态。
- 移除新工作台内对 `SessionPanel` 的依赖。

完成条件：单 ProjectContext 的会话创建、打开、运行、归档、虚拟移动和重启恢复全部可用；【知识】与【协作】保持共享单一入口。

### P2：多工作区与全部分区闭环

目标：形成可以验证后端多 checkout 隔离的前端入口。

- 加入单 / 多工作区模式切换；多工作区模式只增加 ProjectContext 根层级，【知识】与【协作】继续保持 ProjectContext 级单一入口。
- 【协作】沿用当前树行为展开 worktree / checkout 子节点，普通虚拟布局拖动继续作用于 ProjectContext 共享预设。
- 根节点展开只做 lazy hydrate；资源打开时才 focus pane。
- 拆分 KnowledgeView 的左树和右内容，接入 document ID provider。
- 接入 Collab、Asset 和 View 的 scoped adapters。
- 导入现有 Knowledge 路径与 View displayPath，切断普通 drag 对物理移动 API 的调用。
- 将显示设置从 tab visibility 迁移为 section visibility。
- 删除标题栏 workspace selector 及其焦点副作用。
- 增加两个 checkout 并发打开、反向完成和后台 event reducer 测试。

完成条件：同一 ProjectContext 的多个 worktree 共用一棵 Workspace Tree、一个【知识】入口和一个【协作】入口；worktree 级执行与服务状态通过 checkout binding 隔离，虚拟拖动不会改变磁盘。

### P3：主窗口内拖动分屏（首个验收节点）

目标：用户可以直接从资源树或现有标签拖动创建 EditorGroup，在主窗口内同时操作多个资源并调整分隔条。

内部实现顺序：

1. 新增 Workbench store、稳定 EditorInput、最小 tab host 和 editor adapter registry。
2. 将 session、knowledge document 和 local file 接入 EditorHost；Chat 的活动会话状态改为按 editor/session 隔离。
3. 实现二叉 split tree、稳定 paneId、pane focus 和 group resize。
4. 为资源树拖动增加 editor drop intent；Explorer 内部落点继续修改虚拟布局，EditorHost 落点打开资源并创建或复用 EditorGroup。
5. 实现按 EditorGroup 分页数量派生的 tab strip：一个分页完全隐藏，两个及以上分页显示；显隐不重建活动 Editor。
6. 实现可见标签在组间移动、拖到组边缘继续拆分，以及关闭 group 后的 split tree 收缩；单分页组继续通过资源树拖放和编辑器命令完成移动与拆分。
7. 将 VisiblePane lease、workspace focus 和 active session 绑定到 `(windowId, paneId)`，接入 scoped event reducer。
8. 持久化并恢复主窗口 split tree、比例、pane ID、标签、活动标签和 unavailable editor。

完成条件：

- 从左侧资源树拖动会话或知识文档到编辑区边缘，可以创建左、右、上、下分屏。
- 两个可见 pane 可以同时运行两个会话，或同时显示会话与知识文档，状态与 workspace scope 互不串联。
- 水平和垂直分隔条可手动调整，释放后保存比例，重启后恢复。
- 每个 EditorGroup 只有一个分页时保持当前无 tab 的表现；打开第二个分页后显示标签栏，回到一个分页后自动收起。
- 可见标签可以在 EditorGroup 之间移动，也可以拖到组边缘继续拆分。
- A/B checkout 同屏运行、反向完成、关闭 group、窄窗口和恢复测试通过。

### P4：编辑器组完整能力

目标：在首个分屏闭环上补齐长期使用所需的标签、编辑模型和资源适配能力。

- 完成 preview/pinned tab、同组资源去重、活动标签和最近使用顺序。
- 加入共享 document model、dirty guard、关闭确认和可写资源跨组同步。
- 将 view、section、checkout overview、collaboration 和 asset adapters 统一接入 tab host。
- 加入资源重命名后的标题刷新、资源删除后的 unavailable 状态和 stale generation 重新解析。
- 完成标签键盘导航、组焦点命令和所有 Editor Adapter 的生命周期测试。
- 保留现有独立窗口菜单作为原生 WorkbenchWindow 完成前的过渡路径。

完成条件：一个主窗口可以长期保留、切换、编辑和恢复全部支持的资源标签，每个 EditorGroup 的 pane focus 与 dirty 状态准确。

### P5：通用独立窗口

目标：资源标签可以拖出成为第二个完整工作台窗口。

- 新增 WorkbenchWindow payload、恢复协议和 transfer token。
- 抽取 ViewHostWindow 的两阶段 tab handoff 模式。
- 支持跨窗口 group/tab 拖动、失败回滚和窗口关闭归并策略。
- 分阶段迁移 ChatSessionWindow、WorkspacePageWindow、KnowledgeWindow 和 ViewHostWindow。

完成条件：任意支持 detach 的资源可以在第二窗口继续使用，同一会话不会因移动丢失执行状态。

### P6：兼容清理

目标：删除双重导航和旧状态源。

- 删除 SessionPanel 导航分支、重复 Knowledge/View explorer 和旧 workspace page 的兼容入口。
- 删除 `showViewsInSessionPanel` 等已废弃设置。
- 删除旧 workspace page route 对当前目录的隐式依赖。
- 所有 UI 资源调用统一由 ResourceRef / WorkspaceRef / sessionId / viewId 解析。
- 增加静态守卫，禁止新增 `workingDir` 作为业务身份、无 checkout scope 的 workspace IPC 和 selected checkout 投影。

## 文件级改造清单

### 第一批

- `src/App.vue`：一级导航、删除第二层栏、挂载 DevelopmentWorkbench。
- `src/styles/app-global.css`：删除 project tab bar 样式，建立连续 workbench surface。
- `src/language/zh.json`、`src/language/en.json`：新增 Development 文案和 explorer 文案。
- `src/stores/ui.ts`：应用路由收敛与旧状态 migration。
- `src/types/workbench.ts`：稳定资源与 editor 类型。
- `src/services/workspaceExplorer.ts`：新 IPC client。
- `src/stores/workspaceExplorer.ts`：多 checkout snapshot 与树 UI 状态。
- `src/components/workbench/*`：工作台 shell、Explorer 和初始 EditorHost。

### 第二批

- `src/components/ChatWorkspaceView.vue`、`src/components/ChatView.vue`：workbench activation 和无内嵌 session 导航。
- `src/components/SessionPanel.vue`、`src/components/sessionTree.ts`：提取 provider，保留兼容入口。
- `src/components/knowledge/KnowledgeView.vue`、`KnowledgeExplorer.vue`：拆分内容模式与物理移动语义。
- `src/components/CollabView.vue`、`AssetView.vue`、`ViewPackageView.vue`：显式 checkout adapter。
- `src/stores/workspaceContext.ts`：公开无 focus 的 runtime/resource hydrate，并保持 pane focus 唯一入口。

### 后端

- `src-tauri/src/workspace_tree.rs`：多预设文本文件、树校验、revision CAS 和原子写回。
- `src-tauri/src/commands/workspace_explorer.rs`：snapshot、operation、preset、mount listing 和 file preview。
- `src-tauri/src/commands/mod.rs`、`src-tauri/src/lib.rs`：命令注册。
- `src-tauri/src/workspace_service/event.rs`：explorer change scoped envelope。
- 必要时在 `src-tauri/src/workspace_service/runtime.rs` 增加只读 provider 入口，避免资源树直接访问全局服务。

### 窗口化阶段

- `src/components/workbench/WorkbenchEditorTabs.vue`
- `src/components/workbench/WorkbenchSplitHost.vue`
- `src/windows/WorkbenchWindow.vue`
- `src/services/workbenchWindow.ts`
- `src/stores/workbench.ts`
- `src/WindowApp.vue`：通用 WorkbenchWindow route。

## 测试计划

### Rust 文本存储

- 首次读取创建 `index.json` 与独立 `default.json`。
- 多套预设各自使用独立文件，切换只更新 index。
- 外部编辑后的合法文件可重新读取，非法父节点与循环被拒绝。
- revision conflict、operation ID 重试和原子写回行为可重复验证。
- 本地文件预览只允许当前预设引用路径及挂载目录后代。
- 历史缺失字段在导出中为 `empty`。
- folder create / rename / promote-delete / multi-move 的事务测试。
- cycle、非法跨 section、跨 ProjectContext、重复 resource placement 被拒绝。
- expectedRevision 冲突不产生部分写入。
- 相同 operationId 重试只提交一次。
- 同一 ProjectContext 的多个 checkout 共用 layout revision；并发 layout mutation 使用 revision CAS 与 operationId 安全重放。
- 不同 ProjectContext 的 layout mutation 互不阻塞。

### 前端单元测试

- 旧 `activeTab` 所有值迁移为正确一级页面。
- App 不再包含 `projectTabs`、`project-tab-bar` 和标题栏 checkout selector。
- 单工作区和多工作区 projection 的节点层级正确。
- WorkspaceTree 键盘上/下/左/右、Enter、Space、F2、Delete 可用。
- 单击 disclosure 不打开 editor；单击 row 不意外折叠。
- drag preview 与后端允许规则一致。
- 先发慢 snapshot 不覆盖后发结果。
- ProjectContext 共享资源事件只更新 matching project；runtime 与工作目录事件只更新 matching checkout。
- stale generation 触发重新解析，不回退到全局 workspace。
- hidden section 可恢复，固定 section 不能删除或跨 ProjectContext。
- `showTabStrip` 按 EditorGroup 独立派生：0/1 个分页为 false，2 个及以上分页为 true。
- tab strip 从隐藏到显示、再收起时保持相同活动 Editor 实例，不清空 scroll、composer draft 或 streaming state。

### 物理隔离回归

每个 drag E2E 在操作前后记录：

- Knowledge 文档绝对路径、文件 hash、类型。
- View Package 根路径、manifest hash、displayPath。
- session `parentSessionId`、checkoutId、session type。
- Git status 与 worktree 根路径。

普通树拖动后上述值必须保持一致，只有 ExplorerLayout revision 和 node placement 改变。

### 多工作区 E2E

使用两个 checkout 验证：

- 多工作区树为 ProjectContext 增加一个项目根节点，内部 Workspace Tree 的顺序、虚拟文件夹和普通拖动行为与当前实现一致。
- 同一 ProjectContext 下只显示一个【知识】节点和一个【协作】节点；【协作】展开后显示主工作树和其他 worktree。
- 切换或新增 worktree 不复制 Knowledge placement、Collab placement、虚拟文件夹和 project-owned session placement。
- 同一知识文档在绑定 A/B checkout 的两个 pane 中使用相同 `projectId + documentId`，checkout 级索引与工具事件各自路由。
- A 会话执行时切换并操作 B，A 的完成事件仍更新 A。
- 反向快速打开 A -> B，B 先完成后 A 的旧响应不覆盖焦点。
- A/B 对同一 ProjectContext 发起 layout move 时共享 project revision，并通过冲突重放得到一个确定顺序。
- Unity A Connected、Unity B Ready 时，命令只等待目标 checkout 的 Ready。
- 重启后根顺序、虚拟文件夹和资源位置恢复。

### 窗口与分屏 E2E

- 两个 pane 分别绑定 A/B checkout，焦点和 active session 独立。
- 每个 pane 只有一个分页时均不渲染 tab strip，Chat / Knowledge / File 内容顶部与当前单页布局一致。
- 同组打开第二个分页后显示 tab strip；关闭、移动回一个分页后收起，同时保持活动 Editor 实例、滚动位置和草稿。
- tab 在 group 间移动后 runtime lease 正确转移。
- tab 拖出窗口完成 ready ack 后才移除源 tab。
- 目标窗口创建失败时源 tab 和运行状态保持。
- 关闭窗口恢复到合理 group；没有可恢复目标时保留后台任务。
- 重启后 split tree 和稳定 ResourceRef 恢复，不复用旧 generation。

### 视觉与可访问性

- 深色 / 浅色主题、125% / 150% 缩放和窄窗口检查。
- 侧栏保持现有 `sidebar-bg`，编辑区使用 `panel-bg`，不新增营销卡片式 surface。
- 单分页 EditorGroup 不增加常驻 header 或标签栏；双分页开始使用现有中性桌面控件风格的轻量 tab strip。
- 仅图标按钮具有 focus-visible、tooltip 和足够点击区域。
- 可点击行使用 button / treeitem 语义，不使用不可聚焦的 click div。
- 拖动是增强能力；键盘和“移动到…”菜单可完成相同操作。
- 状态使用低饱和 icon/dot，不引入说明性 badge、pill 或 chip。

统一验证命令：

```powershell
bun run test
bun run typecheck:test
```

涉及实际多 checkout / Unity 时使用隔离实例与现有 driver，禁止使用 `bun test`。

## 性能预算

- 树继续使用 `FileTreeList` 虚拟化；目标为 10,000 个可见扁平节点仍可滚动。
- 多工作区模式只先加载 ProjectContext 元数据；checkout 元数据在【协作】展开或 EditorCheckoutBinding 解析时加载。
- ProjectContext 级 Sessions、Knowledge、Collab 和 Views 在分区首次展开时独立加载。
- 共享 provider snapshot 按 projectId + revision 缓存；runtime provider 按 checkoutId + generation 缓存，并使用 LRU 释放后台数据。
- Asset 不进入默认全量树。
- 一次共享资源事件只失效对应 project / section，一次 runtime 事件只失效对应 checkout projection，不能重载整棵树。
- 拖动期间只更新本地 projection；后端确认后提交 snapshot，冲突时精确回滚。
- 资源标题变化采用增量 patch，避免重新 flatten 所有 project roots。

## 可观测性

新增结构化日志字段：

- `windowId`、`paneId`
- `projectId`、`checkoutId`
- `layoutRevision`、`operationId`
- `resourceKind`、`resourceId`
- `intentEpoch`
- `workspaceGeneration`、`serviceInstanceId`、`serviceGeneration`

开发构建可提供一个只读 diagnostics 面板，显示当前窗口的 pane -> checkout -> resource 绑定、runtime lease 和最近 event revision。该面板进入开发者工具，不常驻用户一级界面。

## 风险与控制

| 风险 | 控制措施 |
| --- | --- |
| 树拖动误触物理移动 | Explorer operation 与 Knowledge/View 领域 move 使用不同 service、类型和菜单文案；E2E 校验 hash/path |
| 多窗口最后写入覆盖 | ProjectContext 级 layout revision CAS、事务、operationId 和 event reducer；checkout 级 runtime projection 独立路由 |
| 树展开抢占 checkout | hydrate 与 focus 分离；只有 EditorGroup 激活资源才更新 PaneContext |
| 重复 Explorer 嵌套 | Knowledge / Chat / View 提供 embedded content mode，统一树完成后删除内层导航 |
| 旧 session folder 迁移破坏会话 | 只读导入 placement，不删除或改写历史 session；保留 export 回归 |
| 标签恢复引用旧 runtime | 持久化稳定 ResourceRef，恢复时重新解析 generation |
| 首验范围跨越 tabs 与 split | P3 内部按 EditorInput、单组 tab host、按 session 隔离状态、split tree、拖放与恢复逐步提交；这些步骤共同交付主窗口拖动分屏首验，不形成更早的用户验收节点 |
| 大型资源树卡顿 | 虚拟化、section lazy load、分页 provider 和 scoped invalidation |

## 验收门槛

### 首验前置工程门槛

以下能力并入首个验收节点，不单独组织更早的用户验收：

- 主窗口只有一层应用导航，第一项为“开发”。
- 第二层栏和顶部 workspace selector 已删除。
- 工作区级入口全部位于开发页资源树。
- 单 / 多工作区模式可切换。
- 同一 ProjectContext 的【知识】与【协作】在多个 worktree 下保持单一入口和共享资源身份。
- 两个 checkout 绑定的编辑器可并发打开，ProjectContext 资源与 checkout runtime 事件分别按作用域更新。
- 会话、知识和 View 的虚拟拖动持久化且不改变物理数据。
- 协作、资产和视图没有因为删除分页而失去入口。
- Chat 与 Knowledge 页面不再显示重复左侧导航。
- 所有测试命令通过，隔离实例完成双 worktree E2E。

### 首个用户验收：主窗口内拖动分屏

- 从资源树拖动 Session、Knowledge document 或 local file 到主编辑区边缘可以创建分屏。
- 单个 EditorGroup 只有一个分页时保持当前无 tab 的界面；同组存在两个及以上分页时显示标签栏。
- 从已显示的标签栏拖动活动标签到另一组或组边缘，可以移动标签或继续拆分。
- 左右、上下分屏均可创建，分隔条可实时调整并遵守 pane 最小尺寸。
- 两个可见 pane 的会话、流式状态、workspace focus、active session 和 scoped event 保持独立。
- 重启后恢复 split tree、比例、pane ID、标签和活动标签。

### 会话引用拖动与公共资产预览

会话正文中的文件、Unity 资产和知识文档引用进入统一的语义拖动链路。消息引用是只读来源，拖动操作固定为 `copy`；工作区放置只修改 ExplorerLayout，TabGroup 放置只创建或激活 EditorInput，原始文件、资产和知识文档保持原位。

统一载荷使用版本化结构，并显式携带来源作用域：

```ts
interface WorkbenchReferenceDragData {
  version: 1;
  origin: {
    projectId: string;
    workspaceRef: WorkspaceRef;
    workspaceRoot: string;
  };
  entries: Array<
    | { kind: "file"; path: string; isDir: boolean; name?: string }
    | { kind: "asset"; path: string; name?: string; typeLabel?: string }
    | { kind: "sceneObject"; scenePath: string; objectPath: string; name?: string }
    | { kind: "knowledge"; type: KnowledgeDocumentType; path: string; documentId?: string; name?: string }
  >;
}
```

落点语义：

| 引用 | 左侧 Workspace Tree | TabGroup / EditorGroup |
| --- | --- | --- |
| 文件 | 通过 `mountPath` 创建或复用路径挂载 | 打开 workspace file editor |
| Unity 资产 | 挂载对应物理资产文件 | 打开 asset editor |
| 场景对象 | 放置所属 scene 资产 | 打开 scene object inspector editor |
| 知识文档 | 解析 `documentId` 后 `placeResource` | 打开 Knowledge document editor |

- Tab strip 落点按指针所在 tab 的左右半区插入；EditorGroup 正文落点沿用左、右、上、下半组拆分预览。
- Composer 落点优先级高于 EditorGroup，可把同一语义载荷转换为输入附件。
- 多工作区模式下，Workspace Tree 放置只接受来源 ProjectContext；TabGroup 可以承载来源项目的 EditorInput，并依据资源自身的 EditorCheckoutBinding 切换 pane scope。
- 引用解析器由 Chat 点击、右键菜单、内部拖动和原生拖动适配器共同使用，统一覆盖 `design / plan / memory / skill / reference`。
- 原生文件与 Unity 拖动事件由 Workbench 单点订阅并按指针位置定向提交；已挂载的多个 Chat composer 不直接广播消费同一次 drop。

资产编辑器首版复用现有 Locus Inspector / Asset 预览能力。公共前端设施分为显式 scope 的加载状态与现有渲染层：

```text
WorkspaceAssetPreview
├─ 显式 WorkspaceRef、资源身份、受控/自动加载与宿主操作
└─ UnityObjectPreview：加载缓存、目标选择、文本、二进制、结构化资产、Live Inspector 与交互预览
```

- 公共入口必须接收 `workspaceRef`，缓存键包含 `checkoutId + expectedGeneration + assetPath`。
- Asset 页面、Locus Inspector、Workbench asset editor 和 Unity workspace file preview 复用同一入口。
- 首版保持现有 Inspector 展示与控件密度；不同资产类型的专用界面在公共资源身份和生命周期稳定后迭代。
- 资产与场景对象 EditorInput 使用稳定逻辑路径作为资源身份，runtime generation 保留在 EditorCheckoutBinding，恢复时重新校验物理文件与 checkout generation。

实施状态（2026-08-28）：版本化引用载荷、会话引用解析、Workbench Tree/Composer/TabGroup 路由、原生 drop 单点分发、Asset/Locus Inspector/Workbench/Workspace file 公共预览入口与恢复类型已经落地；验证覆盖语义解析、拖动外部化、布局路由、Inspector 集成、store 恢复、完整 Vitest、应用/测试类型检查、生产构建与隔离实例启动。

验收覆盖：会话内三类引用拖入根目录、虚拟文件夹、Tab strip 和组边缘；双 pane 定向 drop；同路径双 checkout 隔离；隐藏 editor 不消费 drop；重启恢复资产与知识标签；原生 Unity / OS 拖出能力保持。

### 完整工作台完成

- 任意资源可进入编辑器标签。
- 一个窗口可分屏显示不同 checkout。
- 支持可靠拖出第二窗口和跨窗口移动。
- 窗口、分屏和资源标签可恢复。
- Runtime lease、Unity Ready 和后台执行不依赖全局 selected checkout。
- legacy workspace page / current directory 路径和重复窗口实现完成清理。

## 推荐实施顺序

1. 先提交稳定类型、文本预设格式、IPC 与存储测试。
2. 再提交一级“开发”导航、DevelopmentWorkbench shell 和 session 单工作区闭环。
3. 接入多工作区 projection、Knowledge content mode、Collab / Asset / View adapter。
4. 完成虚拟拖动、物理隔离 E2E 后删除重复 Explorer 路径。
5. 建立最小 EditorInput、Workbench store 和单组 tab host，同时把 Chat 活动状态改为按 editor/session 隔离。
6. 在 PaneContext 上实现 split tree、资源树与标签拖放、group resize 和主窗口布局恢复，交付首个“主窗口内拖动分屏”用户验收节点。
7. 补齐 preview/pinned、dirty guard、共享 document model 和全部 Editor Adapter。
8. 抽取通用 WorkbenchWindow transfer，支持拖出、拖回和跨窗口移动。
9. 最后迁移现有专用窗口并删除兼容代码。

这个顺序把前置架构能力合并进一个用户可直接操作的主窗口分屏闭环，同时让 ProjectRegistry、WorkspaceRuntime、WindowContext 和 scoped event 在双 pane 场景中接受第一次完整验收。原生窗口化在主窗口 split tree 与编辑器生命周期稳定后继续复用同一套状态模型。
