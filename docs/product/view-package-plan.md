# View Package 功能计划

状态：P0 / P1 / P2 / P3 已实现基础版本并进入验证  
日期：2026-05-21  
范围：Locus View / View Package / View Runtime / View tools / Unity 数据绑定

## 背景

Locus 需要让 agent 使用前端技术栈制作 Unity 编辑器界面。界面、样式、前端逻辑、Unity C# 脚本需要保存在一个可编辑目录中，后续可以重新加载。这个能力命名为 **View**，对应的目录化资源称为 **View Package**。

View Package 是 Locus 的一等产品资源。它面向可运行的 Unity 编辑器界面，和 Skill 的职责区分如下：

- Skill 负责约束 agent 工作流、选择模板、加载工具、校验产出。
- View Package 负责承载可运行界面、数据绑定、脚本入口和前端资源。
- View Runtime 负责加载 View、显式 reload、Unity 数据同步、C# 命名编译缓存和脚本调用。

## 产品命名

- **View**：用户看到并使用的 Unity 编辑器界面实例。
- **View Package**：承载一个 View 的可编辑目录，包含 Vue / TypeScript / CSS / C# / 绑定声明 / manifest。
- **View Runtime**：Locus 内置的加载、显式 reload、数据同步和脚本调用运行时。
- **View Binding**：声明式绑定前端状态字段到 Asset / GameObject / Component 字段。
- **View Script**：View Package 内的 C# 脚本入口，通过 Unity Bridge 动态编译和调用。
- **Views**：产品界面中的 View 列表入口。

界面文案优先使用“视图”。开发者文档、目录和 API 使用 `View Package`、`View Runtime`、`View Binding`、`View Script`。

## 核心目标

- Agent 能创建、编辑、运行 View Package。
- View Package 存在独立目录中，agent 可以直接修改 Vue / TS / CSS / C# 文件。
- Locus 构建发布后仍可独立创建 View Package。
- 发布版创建 View 时不依赖源码仓库、`bun tauri dev`、本地 `node_modules` 或用户自行安装前端工具链。
- View 前端支持 Locus 显式 reload 和文件监听整页刷新，开发期先保证稳定反馈。
- View 前端优先使用 Vue.js 响应式框架、状态管理和 Locus 自身组件库。
- Agent 可以编写 View Package 内的 TypeScript / Vue `<script>` 来决定前端状态、交互逻辑、动态目标选择和脚本调用流程。
- Agent 避免从零散写大块静态 HTML / CSS，优先基于模板和组件扩展。
- C# 脚本走现有 Unity Bridge / Roslyn 编译机制。
- C# 脚本保存在 View Package 目录内，运行时动态编译，不复制到 Unity 项目的 `Assets/` 或 `Packages/`。
- 支持命名编译缓存，通过唯一名称和源码 hash 复用已编译脚本。
- 支持灵活数据同步和写回。
- 支持字段级 View Binding，前端某一状态字段可直接绑定到 Asset / GameObject / Component 的字段。
- 支持脚本化读入和写回，例如读取 shader、生成 shader graph、用户编辑后调用脚本写回。
- 支持纯只读 View，例如资产审计、引用图、状态看板。

## 非目标

- View Package 初期不替代 Skill Package。
- View Package 初期不发布到 Unity Package Manager。
- View Script 初期不作为 Unity 项目源码参与 Unity 常规脚本编译。
- View 初期不要求完整 IDE 能力，文件编辑继续使用 agent 的常规文件工具。
- View 前端不展示 package 文件列表、源码预览或内置代码编辑器。

## 存储模型

默认项目级存储：

```text
<UnityProject>/Locus/View/<project-name>/<view-id>/
```

该位置适合版本管理、项目迁移和多人协作。后续可以增加用户级存储：

```text
<LocusAppData>/views/<view-id>/
```

发布版内置模板目录：

```text
<app-resources>/view-templates/
├── blank/
├── inspector-form/
├── canvas-board/
├── field-blocks/
├── node-graph/
├── link-board/
├── serialized-table/
├── scripted-transform/   (规划中，未实现)
└── readonly-dashboard/   (规划中，未实现)
```

项目级 View Package 结构：

```text
Locus/View/<project-name>/<view-id>/
├── view.json
├── README.md
├── src/
│   ├── App.vue
│   ├── main.ts
│   └── style.css
├── unity/
│   └── ViewApi.cs        (Unity 模板生成；其余模板按需创建)
└── .locus-view
```

`src/store.ts` 等附加模块不再默认生成，需要共享状态时按需创建。

## Manifest

`view.json` 是 View Package 的入口清单。

```json
{
  "schema": "locus.view.v1",
  "id": "material-inspector",
  "name": "Material Inspector",
  "version": "0.1.0",
  "template": "inspector-form",
  "icon": "InspectionPanel",
  "entry": "src/main.ts",
  "style": "src/style.css",
  "bindings": "bindings.json",
  "scripts": [
    {
      "name": "MaterialTools",
      "path": "unity/ViewApi.cs",
      "entryType": "MaterialTools"
    }
  ],
  "capabilities": {
    "unity": true
  }
}
```

约束：

- `id` 使用 kebab-case。
- 所有路径使用 package 相对路径和正斜杠。
- `entry` 指向 TypeScript 入口。
- `icon` 可选，使用 Locus icon library 名称；内置模板会写入默认图标。
- `scripts[].name` 是命名编译缓存的稳定名称。
- manifest 不写绝对 Unity 项目路径。
- 目标对象、字段绑定、读取范围放入 `bindings.json` 或运行时上下文。

## 内置模板

### blank

最小 View。

适用场景：

- 自定义工具面板。
- 手工组合组件。
- 快速原型。

基础文件：

- `src/App.vue`
- `src/main.ts`
- `src/style.css`
- `bindings.json`

### inspector-form

字段表单模板。

适用场景：

- Material / ScriptableObject / Prefab / GameObject 字段编辑。
- 字段级 View Binding。
- 小型工具型编辑器。

内置能力：

- 标准字段列表。
- 基础输入控件映射。
- `loadBindings()` / `saveBindings()`。
- 绑定状态、dirty 状态、写回状态。

### node-graph

图形化节点 Graph 模板。

适用场景：

- Shader graph 类编辑器。
- 状态机、行为树、任务图。
- 资源依赖图、生成流程图。

内置能力：

- `nodes` / `edges` 标准状态结构。
- `GraphView`、`GraphViewController`、`defineGraphView` 公开 SDK。
- pan、zoom、fit view。
- 节点拖拽、选择、多选、删除。
- 节点连接、端口连接和连接校验。
- 节点参数控件，支持 string / text / number / boolean / select / color。
- `#node` slot 支持替换默认节点内容。
- 自动布局，节点可省略 `x` / `y`，由共享 Graph 组件用 ELK layered layout 计算位置和端口连线路径。
- 图数据序列化。
- `loadGraph()` / `saveGraph()` / `applyGraph()` 入口。
- C# `GraphViewApi` 示例：读取 Unity 数据、保存 graph JSON、执行自定义写回。

### canvas-board

自由画布模板。

适用场景：

- 自定义块编辑器。
- 需要自由拖拽、缩放、摆放的工具界面。
- 没有端口、边和 graph 语义的空间化编辑器。

内置能力：

- `CanvasView` 公开 SDK。
- pan、zoom、fit view。
- 自定义块渲染。
- 块拖拽、单选、多选、框选。
- `Ctrl+C` / `Ctrl+V` / Delete 事件。
- 右键坐标事件，模板默认用于空白处添加块。
- `editBehavior` 配置只读、禁删、禁移动、禁复制粘贴等编辑能力。

### field-blocks

Unity 字段块模板。

适用场景：

- 块状组织的 `SerializedProperty` 编辑器。
- 每个块绑定一组 Asset / GameObject / Component 字段。
- 需要自由摆放字段面板，但不需要 graph 连线。

内置能力：

- `CanvasView` 自由画布。
- `UnityPropertyEditor` 字段控件。
- `property.readProperty` / `property.write` 字段读写。
- 可扩展到 `property.apply` 批量写回。

### link-board

连线匹配 / 连连看式关系编辑模板。

适用场景：

- 左侧数据源到右侧目标字段的绑定。
- 资源映射、字段映射、导入规则匹配。
- 组件引用修复、表格字段到 Unity 字段的映射。

内置能力：

- 左右两栏数据源。
- 中间连线层。
- 拖拽连接。
- 连接规则校验。
- 冲突提示。
- 一键应用写回。
- 只读、自动绑定、脚本写回三种模式。

### serialized-table

Unity 项目资产序列化数据表格模板。

适用场景：

- 聚合多个项目资产的 `SerializedProperty`。
- 按行配置数据源对象。
- 用脚本化来源生成行，例如所有挂载指定组件的 Prefab、所有使用指定 ScriptableObject 类型或继承关系的资产。
- 按列配置要展示和写回的 property。
- 快速检查并统一配置 Prefab、Material、ScriptableObject 等资产字段。

内置能力：

- `tableSources` 固定行配置与 `tableSourceProviders` 脚本化行配置。
- `tableColumns` 属性列配置。
- 中间属性表格。
- 脚本化来源可通过 Locus 资产数据库搜索，支持 `component:`、`script:`、`uses:`、`inherits:` 脚本引用过滤。
- Unity 脚本按资产路径或 GUID 定位对象。
- 支持常见标量、枚举、颜色、向量、对象引用的写回。
- C# `SerializedTableApi` 负责读取与写回配置源对象。

### scripted-transform（规划中，未实现）

脚本化转换模板。

适用场景：

- shader 转 shader graph。
- 批量读取 Unity 数据，前端编辑后调用脚本写回。
- 导入、转换、生成资源。

内置能力：

- `read()` / `transform()` / `write()` 三段入口。
- 进度状态。
- 预览结果。
- 写回确认。

### readonly-dashboard（规划中，未实现）

只读看板模板。

适用场景：

- 资产审计。
- 引用图。
- 性能摘要。
- 场景统计。

内置能力：

- 数据刷新。
- 过滤和排序。
- 只读图表 / 列表 / 树。
- 导出结果。

## 前端运行时

View 前端使用 Vue 3、TypeScript、CSS、Pinia 或等价的 Locus 状态封装。Locus 暴露 View SDK：

```ts
import { CanvasView, GraphView, defineView, property, useViewState, useViewScript } from "@locus/view-runtime";
import { BaseButton, BaseSegmented, BaseCheckbox } from "@locus/components";
```

运行时职责：

- 加载 View Package manifest。
- 编译并加载 package `entry` 指向的 TypeScript / Vue 前端入口。
- 注入 Unity Bridge client。
- 注入 Unity property client。
- 注入 View Script client。
- 注入 Locus 主题 token、基础控件样式和组件语义类，模板默认继承 Locus 的字体、surface、border、text、button、input 风格。
- 支持 agent 编写 `src/main.ts`、`src/store.ts`、`src/App.vue` 中的 TypeScript 逻辑，并在 reload 后重新编译运行。
- 维护 reload 通道。
- 将运行错误展示到 View 宿主界面。
- View 宿主主区域只渲染创建出的页面，不展示文件列表、源码预览或内置代码编辑器。

开发期：

- 源码仓库开发时可以使用 Vite dev server。
- `bun tauri dev` 和 `bun tauri dev-mcp` 保持现有工作流。

发布版：

- 发布版内置 View compiler/runtime。
- 创建 View 不要求用户安装 bun、node 或 npm 包。
- P0 使用 Locus 调用 `view_reload` 和 WebView 整页 reload 实现快速反馈。
- 模块级 HMR 作为后续性能优化能力，在 View Runtime 稳定后评估。
- 模板依赖的 Vue / Pinia / Locus components 由 Locus runtime 提供。

## Unity C# 脚本运行时

View Script 保存在 View Package 的 `unity/` 目录下。脚本通过 Unity Bridge 发送到 Unity Editor 动态编译执行。

新增命名编译模型：

- `view_compile_script`
- `view_call_script`
- 底层 Unity Bridge 新增 `compile_named` / `invoke_named` 能力。

编译 key：

```text
projectPath + viewId + scriptName + entryType + sourceHash + unityDomainFingerprint
```

编译返回：

```json
{
  "name": "MaterialTools",
  "hash": "blake3-source-hash",
  "cacheHit": true,
  "assemblyId": "__LocusView_MaterialTools_..."
}
```

调用参数：

```json
{
  "viewId": "material-inspector",
  "scriptName": "MaterialTools",
  "hash": "blake3-source-hash",
  "method": "Read",
  "args": {
    "assetPath": "Assets/Materials/M.mat"
  }
}
```

缓存失效条件：

- View Script 源码 hash 变化。
- Unity domain reload。
- Unity 脚本重新编译完成。
- metadata reference cache 失效。
- Unity 项目路径变化。
- Locus Unity Bridge 版本变化。

脚本约束：

- 脚本不复制到 Unity 项目源码目录。
- 脚本不触发 Unity 常规脚本重编译。
- 脚本入口必须是显式方法，例如 `Read`、`Write`、`Apply`。
- 脚本入口支持零参数或一个强类型 request DTO；`args` 到 DTO 的转换由 Unity Bridge 协议层完成。
- 长任务通过现有 `ExecuteCodeContext` 风格报告进度和响应取消。

## 数据绑定

`bindings.json` 描述前端状态字段和 Unity 数据字段的关系。

```json
{
  "schema": "locus.view.bindings.v1",
  "bindings": [
    {
      "id": "materialColor",
      "statePath": "material.color",
      "target": {
        "kind": "asset",
        "path": "Assets/Materials/M.mat",
        "propertyPath": "m_SavedProperties.m_Colors.Array.data[0].second"
      },
      "mode": "readWrite"
    },
    {
      "id": "objectName",
      "statePath": "selection.name",
      "target": {
        "kind": "gameObject",
        "scenePath": "Assets/Scenes/Main.unity",
        "objectPath": "Root/Player",
        "propertyPath": "m_Name"
      },
      "mode": "readOnly"
    }
  ]
}
```

绑定模式：

- `readOnly`：读取 Unity 数据并展示。
- `readWrite`：读取、编辑、写回字段。
- `scriptedRead`：通过 View Script 读入。
- `scriptedWrite`：通过 View Script 写回。
- `manual`：前端维护状态，用户触发脚本执行。

目标类型：

- `asset`
- `gameObject`
- `component`
- `scriptableObject`
- `projectSetting`
- `custom`

字段写回优先级：

1. `SerializedObject` + `SerializedProperty` 字段写回。
2. Unity Editor API 专用写回，例如 Material / Shader / Animator。
3. View Script 自定义写回。

写回流程：

1. 读取当前 Unity 值。
2. 前端状态更新。
3. 记录 dirty 字段。
4. 用户或脚本触发 apply。
5. Runtime 校验目标仍存在。
6. Runtime 写回字段或调用脚本。
7. 标记场景 / 资源 dirty。
8. 按目标类型保存 Asset / Scene。
9. 返回写回结果和错误详情。

## View tools

View 相关 agent tool 仅由 Skill 加载。默认 agent 上下文保持轻量。

工具建议：

- `view_create`
- `view_list`
- `view_reload`
- `view_run`
- `view_compile_script`
- `view_call_script`
- `view_binding_read`
- `view_binding_discover`
- `view_binding_write`
- `view_binding_apply`

加载规则：

- View tools 注册为 `ToolLoadMode::Skill`。
- 默认 dev agent 工具列表不直接暴露 View tools。
- 内置 `/view` 或 `/create-view` Skill 负责加载工具。
- Skill 负责选择模板、创建目录、调用 reload、运行校验。
- 文件编辑继续使用常规文件工具，范围限定在 `packageRoot`。
- Agent-facing View tools 只负责 View Package 生命周期、运行、reload、脚本编译和脚本调用。
- `view_create` 返回 summary / manifest / packageRoot 元数据，不返回 package 内每个文件内容。
- package 内文件读取和编辑交给通用 `read` / `edit` / `write` 文件工具。
- Tauri 内部可保留 runtime 用的 package 读取 command；该能力不作为 agent tool 暴露。

Rust 注册示例：

```rust
registry.register_builtin_with_load_mode(view::view_create(), ToolLoadMode::Skill);
registry.register_builtin_with_load_mode(view::view_list(), ToolLoadMode::Skill);
registry.register_builtin_with_load_mode(view::view_reload(), ToolLoadMode::Skill);
registry.register_builtin_with_load_mode(view::view_run(), ToolLoadMode::Skill);
registry.register_builtin_with_load_mode(view::view_compile_script(), ToolLoadMode::Skill);
registry.register_builtin_with_load_mode(view::view_call_script(), ToolLoadMode::Skill);
```

内置 Skill 工作流：

```text
/view <需求>
1. 判断 View 类型：blank / inspector-form / canvas-board / field-blocks / node-graph / link-board / serialized-table（scripted-transform / readonly-dashboard 规划中）
2. 加载必要 view_* 工具
3. 创建或定位 View Package
4. 使用通用文件工具读取和编辑 packageRoot 内文件
5. reload
6. run
7. 校验前端加载、数据读取、脚本编译、写回路径
```

产品约束：

```text
View 是产品能力，View tools 是 Skill 执行能力。
```

## UI 入口

产品界面建议新增 Views 入口：

- 列出项目内 View。
- 显示模板元数据。
- 打开 View。
- 显示 View 运行错误。
- 显示 View Package 路径。
- Views 列表入口只显示 View 列表、元数据和打开操作，不显示 package 文件目录结构、manifest 源码或源码预览。
- Views 列表入口不常驻新建 View 表单；创建通过 `/view` Skill workflow 和 `view_create` 工具完成，后续如需 UI 创建入口再单独设计。
- View 打开后只展示创建出的页面，文件和源码编辑通过 agent 常规文件工具完成。
- 会话列表下方可以显示 Views 区域；该区域由设置项控制显示，边界支持用户拖拽调整高度。
- Views 列表使用 `view.json.icon` 渲染左侧图标；agent 创建 View 时从 Locus icon library 中选择图标。
- Views 列表使用类似文件树的层级结构展示文件夹和 View Package。
- Views 树支持右键新建文件夹、删除文件夹或 View Package。
- Views 树支持拖拽移动文件夹和 View Package；移动后同步移动磁盘上的 package 目录。
- 删除操作会删除对应磁盘目录，删除文件夹时会递归删除其内部 View Package。

UI 风格要求：

- 保持桌面工具 / IDE 风格。
- Views 列表入口可以使用顶部标签栏、左右分栏、面板、列表、工作区结构。
- Views 列表入口右侧使用元数据表展示名称、ID、模板、版本、能力、更新时间和位置。
- 单个 View 运行窗口以页面画布为主，不提供文件树、源码面板或源码预览区。
- 内置模板默认使用 Locus 主题 token 与基础控件语义类，避免在 package 模板中写死独立色板和控件皮肤。
- 创建入口使用标准轻量控件。
- 模板选择使用列表或分段控件。
- 状态标签只用于强语义状态，例如 running、error、modified。
- 避免营销页式卡片堆叠。
- 避免装饰性背景、夸张动效和大面积强调色。

## 安全与边界

文件边界：

- `view_create` 只在允许的 View roots 下创建目录。
- `view_*` 工具只能读取和写入目标 View Package。
- package 相对路径禁止 `..`、绝对路径、空段。
- View Script 源码读取范围限定在 package `unity/`。

Unity 写回边界：

- Play Mode 下默认限制持久写回。
- 写回前检查 Unity Editor 状态。
- 高风险写回需要用户确认。
- Scene / Prefab / ScriptableObject / ProjectSettings 写回需要明确目标。
- 批量写回需要返回影响范围。

运行边界：

- View frontend 运行在 Locus 托管的 View host 页面内。
- View 默认以真实 Tauri 子窗口打开，保留突破主窗口边界、跨屏和系统级窗口管理能力。
- P3 起 View host 采用可信 View Runtime，直接编译运行 package TypeScript / Vue 逻辑。
- View Runtime 暴露 `@locus/view-runtime`、Locus components、Unity property 和 View Script API。
- 文件读写仍通过 View Package 路径边界和工具权限约束。
- Unity 写回仍通过 View Binding / View Script API 执行状态检查、影响范围返回和高风险确认。

## 与现有系统的关系

可复用现有实现：

- Vue / Vite / Pinia 前端基础。
- Unity embed 窗口和 `LocusRuntime`。
- Unity Bridge named pipe transport。
- Roslyn metadata reference cache。
- `unity_execute` 的异步执行、进度、取消模型。
- Skill package manifest 扫描经验。
- Unity YAML / inspector / diff 的字段解析经验。
- Locus 组件库和主题 token。

需要新增的核心模块：

- `view` Tauri command 模块。
- `view` tool builtins。
- View Package manifest 读取和校验。
- View template copier。
- View frontend compiler/runtime。
- View runtime host route。
- View Binding engine。
- Unity Bridge named compile cache。
- View Script invoke protocol。
- 内置 `/view` Skill。

## 分阶段实现

### P0：View Package 生命周期与运行基础设施

目标：

- 发布版可以创建 View Package。
- 发布版内置可运行的 View host / compiler / runtime，打开 `blank` View 不依赖源码仓库、本地 `node_modules`、`bun` 或用户安装前端工具链。
- Agent 可以通过 Skill-only `view_create` 创建模板目录。
- Locus 可以列出、读取、reload、打开 View。
- `/view` Skill 激活后才授予 View tools，`ToolLoadMode::Skill` 之外还需要 Skill 授权门控。
- View workflow 内的文件访问收敛到当前 `packageRoot`。
- P0/P1 使用静态 View host 渲染模板和样式，P3 替换为可信 View Runtime 并运行 package TypeScript / Vue。

工作项：

- 新增 `Locus/View/<project-name>/<view-id>/` 根。
- 新增 `view.json` schema 校验。
- 内置 `blank`、`inspector-form` 模板。
- 新增 View host route / window，负责读取 manifest、加载前端入口、注入主题 token、基础控件样式和组件语义类、展示运行错误；主区域只渲染创建出的页面。
- 发布版内置 P0 View host，支持 `blank`、`inspector-form` 的模板和 CSS 加载；P3 补齐 TypeScript / Vue 编译运行。
- P0 热反馈使用 Locus 调用 `view_reload` 和 WebView 整页 reload。
- 新增 agent-facing `view_create`、`view_list`、`view_reload`、`view_run`。
- View host runtime 内部保留读取 package 文件的 Tauri command；该 command 不作为 agent tool 使用。
- View tools 注册为 `ToolLoadMode::Skill`。
- 新增 View tool 授权门控：只有当前 `/view` Skill workflow 明确声明并加载 View capability 时，`view_*` 工具才进入 allowed tools。
- 新增 View package 文件边界：`view_*` 工具只能访问目标 View Package；常规文件工具在 View workflow 内只能访问当前 `packageRoot`；路径统一走 package 相对路径校验。
- 新增 P0/P1 静态 View host；P3 引入可信 View Runtime，允许 package TypeScript / Vue 通过 `@locus/view-runtime` 调用 View Binding 和 View Script 能力。
- 新增 `/view` Skill。
- UI 新增 Views 列表入口，只展示 View 列表、元数据和打开操作。

验收：

- 发布版创建 `blank` View 成功。
- 发布版打开 `blank` View 成功，并渲染 package 内前端入口。
- Views 列表入口不显示 package 文件目录结构、manifest 源码或源码预览。
- View 运行窗口只显示创建出的页面，不显示 package 文件列表、源码预览或内置代码编辑器。
- `blank` View 默认继承 Locus 的字体、背景、面板、边框、文本、按钮和输入框风格。
- 移除本地 `node_modules` 或未安装 bun 的环境中，发布版仍可创建并打开 `blank` View。
- Agent 拿到 `packageRoot` 后可以编辑 package 内文件。
- View workflow 内访问 `packageRoot` 外文件会被拒绝。
- reload 后加载最新文件，P0 使用整页 reload 即可。
- 未激活 `/view` Skill 时调用 `view_*` 工具返回未授权。
- 激活 `/view` Skill 后可以加载并调用声明的 `view_*` 工具。
- View 前端通过 View Runtime API 读取 manifest、触发 reload、接收错误状态成功。

### P1：模板扩展和 reload 体验

实现状态：已实现基础版本。

目标：

- View 前端编辑后可通过 Locus reload 快速刷新。
- 内置 graph 和 link-board 模板。

工作项：

- 完善 View frontend runtime 的 reload 入口。
- 文件监听触发 View host 整页刷新。
- 增加 `node-graph` 模板，内置可拖拽节点、连线渲染和图数据序列化。
- 增加 `link-board` 模板，内置左右连接和连接数据序列化。
- 通过受控 View host runtime 注入 Graph / Link Board 基础交互。

验收：

- 调用 `view_reload` 后 View 加载最新 `App.vue`、样式和绑定文件。
- 文件监听触发的整页 reload 能在开发期提供稳定反馈。
- `node-graph` 模板可创建、拖拽节点、连线、保存图数据。
- `link-board` 模板可创建左右连接并序列化。

### P2：C# 命名编译缓存

实现状态：已实现基础版本。

目标：

- View Script 保存在 package 目录。
- C# 脚本按名称和 hash 编译缓存。
- 后续调用复用缓存。

工作项：

- Unity Bridge 增加 `compile_named`。
- Unity Bridge 增加 `invoke_named`。
- Tauri 增加 `view_compile_script`。
- Tauri 增加 `view_call_script`。
- 缓存 key 包含 Unity domain fingerprint。
- domain reload 后通过新的 Unity AppDomain static cache 自动失效。
- 编译错误映射回 package 相对路径和行号。

验收：

- 第一次编译返回 `cacheHit=false`。
- 相同源码再次编译返回 `cacheHit=true`。
- 修改源码后 hash 变化并重新编译。
- Unity 重新编译或 domain reload 后缓存失效。

### P3：TypeScript View Runtime 与 View Binding

实现状态：已实现基础版本。真实 Tauri 子窗口是当前默认运行形态。

目标：

- Agent 可以编写 TypeScript / Vue `<script>` 来决定 View 前端行为。
- View Package 的 `src/main.ts`、`src/store.ts`、`src/App.vue` 脚本会被编译并运行。
- 前端状态字段可直接绑定到 Unity 字段。
- 单个字段可以根据前端状态动态选择 Material、ScriptableObject、GameObject、Component 或 Asset 目标。
- TypeScript 可以调用 View Script，让 C# 动态决定读取、写回、校验和转换行为。
- 常规字段读写无需 agent 编写同步脚本。

工作项：

- 实现发布版内置 View frontend compiler/runtime。
- 编译 package `entry` 指向的 TypeScript 入口，支持 Vue SFC `<script setup>` / `<script>`、CSS 和 store 模块。
- 提供 `@locus/view-runtime` SDK：`defineView`、`useViewState`、`property`、`useViewScript`、`view.reload`。
- 提供 component-only `@locus/components` 运行时导入：`BaseButton`、`BaseSegmented`、`BaseCheckbox` 等常用控件。
- reload 时重新编译并运行 package TypeScript / Vue。
- 新增 `bindings.json` schema。
- 实现 `view_binding_read`。
- 实现 `view_binding_discover`，只读按字段名、类型名或查询文本发现 `SerializedProperty.propertyPath`。
- 实现 `view_binding_write`。
- 实现 `view_binding_apply`。
- Unity 侧实现 SerializedProperty 读写。
- `SerializedPropertySnapshot` 返回反射得到的 `fieldTypeFullName` / `fieldTypeAssembly`，Generic 自定义类型可被识别并用于专用 UI 匹配。
- 支持 asset / gameObject / component 目标。
- 支持 readOnly / readWrite / scriptedRead / scriptedWrite。
- 支持 TypeScript 动态生成 binding `target`，用于单个字段按当前前端状态切换 Material、ScriptableObject、GameObject、Component 或 Asset 目标。
- `pathFrom`、`scenePathFrom`、`objectPathFrom`、`componentTypeFrom`、`propertyPathFrom` 这类声明式表达式后续按模板需要扩展；P3 先以 TypeScript 生成 target 作为主路径。
- 支持 TypeScript 根据当前选择对象生成 binding 请求。
- 支持 TypeScript 调用 `view_call_script`，由 C# View Script 动态决定行为。

已落地范围：

- View host 使用可信 Vue runtime component，执行 `view.json` 的 `entry`。
- 支持标准 `createApp(App).mount("#app")` 入口拦截，把根组件挂到 View host。
- 支持 Vue SFC `<script setup>` / `<script>`、`src/store.ts`、`src/` 下拆分的 `.ts` / `.vue` / `.css` / `.json` 模块。
- 支持 `@locus/view-runtime`、`@locus/components` 和常用 Vue API 导入。
- 支持 `view.callScript` / `useViewScript` 调用 View Script。
- 支持 `property.readProperty`、`property.write`、`property.apply` 和 `property.fromPath`。
- Unity 侧支持 selection / asset / material / scriptableObject / gameObject / component 的 SerializedProperty 读写。
- `readOnly` binding 在写入路径返回明确错误。
- package 文件编辑继续走 agent 通用文件工具；`view_read` 只作为 View host 内部读取 command。

验收：

- Agent 修改 `src/main.ts`、`src/store.ts` 或 `src/App.vue` 中的 TypeScript 后，`view_reload` 会重新编译并运行最新逻辑。
- TypeScript 能维护前端选择状态并驱动 UI 行为。
- TypeScript 能调用 View Script 并展示返回结果。
- 单个字段能在 Material、ScriptableObject、GameObject 之间按当前选择动态切换绑定目标。
- Material 字段读取成功。
- GameObject 名称读取成功。
- 常规数值 / 字符串 / bool / Color / Vector 字段写回成功。
- 只读绑定无法写回并返回明确错误。
- 写回后 Unity 对象 dirty 状态正确。

### P4：脚本化转换和复杂写回

目标：

- 支持 shader 到 shader graph 等复杂转换。
- 支持 View 前端编辑后调用 View Script 写回。

工作项：

- 完善 `scripted-transform` 模板。
- 提供 `read` / `transform` / `write` 示例。
- 支持长任务进度。
- 支持结果预览。
- 支持写回前 diff 或影响摘要。

验收：

- 脚本可读取 Unity 数据生成前端状态。
- 前端修改后可调用脚本写回。
- 长任务可报告进度并取消。
- 写回失败返回结构化错误。

### P5：模块级 HMR 优化

目标：

- 在 View Runtime、模板体系和 binding 写回稳定后降低前端编辑反馈延迟。
- 保留整页 reload 作为默认稳定路径。

工作项：

- 评估模块级 HMR 对多 View 窗口的内存、watcher 和 runtime 复杂度影响。
- 为开发期 View host 接入模块级 HMR。
- 复用 Locus runtime、组件库和编译缓存，避免每个 View 重复加载完整工具链。
- 失败时回退到 `view_reload` 整页刷新。

验收：

- 修改样式或局部组件时可在当前 View 中局部更新。
- HMR 失败后自动执行整页 reload。
- 同时打开多个 View 时 runtime 内存增长可观测、可限制。

## 测试计划

Rust / Tauri：

- `view.json` schema 校验。
- View id 和路径安全校验。
- 模板复制。
- View roots 解析。
- ToolLoadMode 为 Skill。
- View Package reload。
- bindings schema 校验。
- 编译缓存 key 生成。

前端：

- Views 列表布局。
- Views 树的文件夹创建、右键删除、拖拽移动和磁盘目录同步。
- View host 错误态。
- View frontend compiler 编译并运行 package TypeScript / Vue。
- 模板创建结果展示。
- Graph canvas 基础交互。
- Link Board 连接状态。
- TypeScript 动态选择 binding 目标。
- TypeScript 调用 View Script。
- Binding dirty 状态。

Unity：

- named compile cache 命中。
- domain reload 后缓存失效。
- SerializedProperty 读写。
- Asset 写回。
- Scene object 写回。
- Play Mode 写回限制。

统一命令：

```text
bun run test
bun run typecheck:test
```

禁止使用：

```text
bun test
```

## Open Questions

- View Package 是否需要支持项目级和用户级双根的优先级策略。
- 发布版 View frontend compiler 采用内置 esbuild / rolldown / 预打包 runtime 的具体选型。
- View Package 是否需要导入 / 导出格式。
- View 是否需要权限声明，例如 file read、Unity write、network。
- Graph / Link Board 基础组件放在 View Runtime，还是作为模板内可编辑源码复制。
- 模块级 HMR 的触发范围、缓存策略和多 View 限流策略。
- 复杂写回是否默认生成影响摘要并要求确认。

## 推荐落地顺序

1. 先实现 P0，确定 View Package 生命周期和 Skill-only tool 入口。
2. 再实现 P2，打通 C# 命名编译缓存，保证 View Script 不进入 Unity 项目。
3. 接着实现 P1 的模板和 reload 体验，优先 `node-graph` 与 `link-board`。
4. 再实现 P3 / P4 的 TypeScript runtime、声明式绑定和复杂写回。
5. 最后评估 P5 模块级 HMR 优化。

P0 完成后，View Package 就具备稳定产品形态。P2 完成后，View 可以成为真正的 Unity 编辑器扩展运行环境。P3 / P4 负责把它扩展成可编写 TypeScript 行为的数据编辑和生成工具。P5 用于优化开发期反馈速度。
