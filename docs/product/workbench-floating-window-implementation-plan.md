# Workbench 浮动窗口实施计划

状态：阶段 1—4 已完成；阶段 5 Development Workbench 共享主链路与全部 Tab 原生 DnD 已完成，View Host 业务 runtime 共享化待单独收口

日期：2026-08-31

## 目标

将开发工作台的 EditorGroup 扩展为可恢复的多原生窗口模型。用户可以把编辑器 Tab 拖到任意 Locus Workbench 窗口的 Tab strip 或 EditorGroup，释放在所有 Workbench 窗口之外时创建辅助窗口；辅助窗口继续支持 Tab、分屏、跨窗口移动和重启恢复。

## 产品行为

1. 主窗口内部拖动继续使用现有排序、组间移动与四向分屏。
2. 拖到另一 Workbench 窗口的 Tab strip 时，按指针所在 Tab 左右半区确定插入位置。
3. 拖到另一 Workbench 窗口的编辑区时，中心区域并入目标组，边缘区域创建对应方向的分屏。
4. 释放点位于所有 Workbench 窗口之外时，在释放点附近创建辅助窗口。
5. 辅助窗口单个 Tab 时保持 Tab strip 可见，继续提供明确的拖入目标。
6. 源 Tab 在目标 editor ready 前保持原位；目标失败或超时后取消交接。
7. 辅助窗口最后一个 Tab 被移走后关闭；主窗口保留空 EditorGroup。
8. 应用正常退出时保存辅助窗口、位置、尺寸、分屏和 Tab；下次启动恢复。
9. 用户主动关闭辅助窗口时关闭该窗口承载的 editor，并清理窗口恢复记录；dirty editor 先经过现有保存确认流程。

## 状态与协议

### Window identity

- 主窗口：`main`
- 辅助窗口：`workbench-<uuid>`
- `windowId` 与 Tauri window label 保持一致。

### 跨窗口落点

目标窗口根据物理屏幕坐标、`innerPosition` 和 `scaleFactor` 转换为本地 CSS 坐标，再复用 Workbench 的 Tab 插入与 EditorGroup 四向落点算法，返回：

```ts
interface WorkbenchWindowDropIntent {
  windowId: string;
  paneId: string;
  direction: "center" | "left" | "right" | "top" | "bottom";
  index?: number;
}
```

### 两阶段交接

1. 源窗口生成 transfer token，并导出 `WorkbenchEditorInput` 与可用的编辑器瞬态快照。
2. 目标窗口 provisional insert，解析 checkout/runtime 并挂载 editor。
3. 目标窗口发送 ready ack。
4. 源窗口移除原 Tab，释放空 pane context；目标窗口提交并持久化。
5. 失败或超时后目标回滚 provisional editor，源窗口保留原 Tab。

transfer token 不进入持久化布局。

## 实施阶段

### 阶段 1：窗口基础

- `DevelopmentWorkbench` 接收显式 `windowId`、辅助窗口模式与 Explorer 可见性。
- 新增 `WorkbenchWindow` 路由、bootstrap、标题栏和窗口控制。
- 辅助窗口强制显示单 Tab strip，并继续复用现有 split tree。
- 新增 Workbench 窗口服务与 Rust 窗口创建命令。

### 阶段 2：跨窗口交接

- `WorkbenchEditorTabs` 转发拖动激活与完成事件。
- 将原窗口级 Tab 命中升级为 `windowId + paneId + direction + index`。
- 增加 transfer prepare/ready/commit/cancel 事件。
- 支持拖出、拖回、辅助窗口之间移动、最后 Tab 清理和失败回滚。

### 阶段 3：编辑器状态

- Session editor 交接输入草稿，并通过 durable session/run 重新 hydrate 流式状态。
- Workspace file editor 交接未保存文本、基础 content hash、行尾格式与光标位置。
- Knowledge/复杂 editor 使用 adapter capability 控制 detach；具备可靠恢复快照后开放 dirty detach。
- 资源型只读 editor 通过稳定 ResourceRef 恢复。

### 阶段 4：恢复与性能

- 持久化辅助窗口清单、bounds、最大化状态与 Workbench 布局。
- 应用启动时先创建隐藏窗口骨架，再延迟 hydrate 后台 Tab。
- 增加一个隐藏 Workbench 预热窗口；首次拖出优先 claim，随后异步补充。
- 记录 `dragActivated -> targetReady -> sourceCommit -> windowShown` 各阶段耗时。

## 性能门槛

- 已有目标窗口的跨窗口移动：交接提交 P95 小于 120ms。
- 命中预热池的首次拖出：释放到窗口首帧 P95 小于 180ms。
- 直接创建回退：释放到窗口首帧小于 500ms。
- 拖动采样保持约 60Hz，任一时刻最多一个目标解析请求在途。
- 拖动过程中不触发布局持久化、资源重新扫描或同步 runtime 启动。

## 验证

### 自动测试

- Workbench store：跨窗口 prepare/commit/rollback、资源去重、空 pane 收缩。
- 落点几何：主窗口标题栏偏移、多个 pane、Tab 插入、四向分屏、DPI 转换。
- Window route：直接创建、池 claim、恢复 URL、辅助窗口关闭。
- 编辑器状态：Chat 草稿、dirty 文件、ready timeout 回滚。
- Rust：窗口 label、物理/逻辑坐标、恢复清单、池 claim/replenish。

### 实际测试

1. 使用 `bun run locus:test:app -- --skip-onboarding` 启动隔离实例。
2. 使用 WebView2 CDP 定位主窗口和辅助窗口，执行真实指针拖动。
3. 验证拖出、拖回、跨辅助窗口、Tab 插入、四向分屏和最后 Tab 关闭。
4. 记录预热命中与直接创建的性能埋点。
5. 使用 `bun run locus:test:unity -- --project <project> --suite connect --install-plugin` 验证窗口化后 workspace/runtime 与 Unity 连接保持有效。
6. 重启隔离实例，验证窗口 bounds、split tree、Tab 和活动 editor 恢复。

## 完成条件

- 四个阶段全部完成。
- `bun run test`、`bun run typecheck:test` 和生产构建通过。
- CLI driver 的 `connect` suite 通过。
- CDP 实际拖动路径全部通过并生成性能记录。
- 主窗口与辅助窗口保持现有中性桌面工具风格，没有新增 badge、chip、营销式卡片或私有按钮体系。

## 实施结果

### 阶段 1：窗口基础

- `DevelopmentWorkbench` 已按 Tauri window label 隔离状态，主窗口使用 `main`，辅助窗口使用 `workbench-*` / `workbench-pool-*`。
- `WorkbenchWindow` 已接入 `window.html` 自揭示路由、独立标题栏、窗口控制、单 Tab strip 与无 Explorer 的辅助窗口布局。
- 辅助窗口继续复用 Workbench split tree、EditorGroup、Tab strip 和现有设计 token。

### 阶段 2：跨窗口交接

- 已实现物理屏幕坐标到目标 WebView CSS 坐标的 DPI 转换，并支持 Tab 插入、中心并组与四向分屏意图。
- transfer record 使用共享 IndexedDB；源窗口等待目标 ready ack 后再移除 editor，超时或目标失败时回滚 provisional editor。
- 已修复 Vue 响应式落点对象直接写入 IndexedDB 时的 `DataCloneError`，所有跨存储边界的数据均转为 plain object。
- 辅助窗口最后一个 Tab 移走后关闭；主窗口允许保留空 EditorGroup。

### 阶段 3：编辑器状态

- Session / New Session 交接 Chat composer 草稿，包括文本、图片、资源引用、本地文件、Console 文本与输入意图。
- Workspace File / Local File 交接未保存文本、基础 content hash、行尾、选区与滚动位置；目标文件基线变化时拒绝提交并保留源 Tab。
- dirty Knowledge editor 在具备可靠快照前阻止跨窗口移动，并给出保存提示；只读资源 editor 继续通过稳定 ResourceRef 恢复。

### 阶段 4：恢复与性能

- 辅助窗口 registry 已持久化 label、bounds、最大化状态；窗口内 split tree、Tab 与活动 editor 由现有 Workbench 持久化按 windowId 隔离。
- 应用启动会恢复辅助窗口，并同时准备一个已完成 bootstrap 与 Workbench mount 的隐藏预热窗口；claim 后立即显示已预热 shell，editor 内容异步完成两阶段交接。
- 拖动窗口句柄缓存 250ms、窗口 bounds 缓存 96ms；跨窗口命中循环维持单请求在途，减少每帧 Tauri IPC。
- 已增加 `drag-activated`、`drag-frame-summary`、`pool-claimed`、`window-shown`、`window-content-ready`、`target-editor-ready`、`source-transfer-committed` 与失败路径指标。

## 实际验收记录

隔离实例：`<runtime-root>\locus-app-test-<run-id>`

- CDP + 原生屏幕游标完整执行：主窗拖出、第二 Tab 拖入辅助窗、辅助窗 Tab 拖回主窗，三段全部通过。
- 预热窗口 shell：释放后 `120ms` 可见；首个 editor：`206ms` ready；源端提交：`240ms`。
- 已有辅助窗口并入提交：`88ms`；辅助窗口拖回主窗提交：`76ms`。
- 三段拖动采样：`57.1 FPS`、`49.6 FPS`、`59.0 FPS`，最低 `49.6 FPS`；平均命中检查 `4.82ms`、`10.61ms`、`6.55ms`。
- 自动化门槛固化为：预热 shell `<= 180ms`、内容 ready `<= 300ms`、最低采样 `>= 40 FPS`。
- 重启验收通过：registry 中 3 个辅助窗口全部恢复，3 个 CDP 辅助窗口目标恢复 4 个 Tab。
- Unity CLI driver `connect` suite 通过：`passed=1`、`failed=0`、`finished.ok=true`，transport 为 `native_broker`。
- `bun run typecheck`、生产构建、Workbench 相关 4 个测试文件 29 项均通过。
- 当前全量 Vitest 结果为 2220 项通过、1 项失败；失败来自并行改动中的 `src/__tests__/locusUnityCliDriver.test.ts:95` 未定义变量 `script`，与 Workbench 浮动窗口改动无关。`typecheck:test` 同样被该文件阻断。

CDP 截图：

- `<runtime-root>\locus-app-test-<run-id>\logs\workbench-window-main.png`
- `<runtime-root>\locus-app-test-<run-id>\logs\workbench-window-auxiliary.png`

## 阶段 5：共享子窗口运行时与 Chromium 原生 DnD

### 迁移目标

所有承载可拖动 Tab 的辅助窗口迁移为共享业务运行时：主窗口只初始化一次 Vue、Pinia、Workbench Store、View Host 状态、拖拽会话和应用服务；每个辅助窗口保留独立原生窗口、独立 WebView2 与独立 document，主窗口把 Vue 子树渲染到目标 document。普通进度、确认、编辑器等非 Tab 工具窗口继续使用轻量 `window.html` 独立入口。

Tab 拖动迁移为 HTML 标准 DnD。Chromium 直接从真实 Tab DOM 生成系统拖拽位图，鼠标热点使用按下点在 Tab 内的真实相对坐标。窗口内排序、跨已有窗口移动和拖出创建窗口使用同一个原生拖放会话。

### 调研依据

- VS Code 普通 Editor Tab 在 `dragstart` 中直接执行 `dataTransfer.setDragImage(tab, 0, 0)`；多选和 shrink sizing 才回退到专用文本拖拽块。Locus 采用同一 DOM 快照机制，并把热点从 VS Code 的 `(0, 0)` 改为真实点击偏移。来源：<https://github.com/microsoft/vscode/blob/main/src/vs/workbench/browser/parts/editor/multiEditorTabsControl.ts#L1111-L1142>。
- VS Code 在 `dragend` 后读取原生屏幕光标位置，确认释放点位于现有窗口之外，再创建 `AuxiliaryEditorPart` 并移动 Editor。来源：<https://github.com/microsoft/vscode/blob/main/src/vs/workbench/browser/parts/editor/editorTabsControl.ts#L455-L502>。
- VS Code 的辅助窗口使用 `window.open('about:blank', ..., 'popup=yes,...')`，克隆主窗口 stylesheet、根节点属性与主题状态，并持续同步后续样式变化。来源：<https://github.com/microsoft/vscode/blob/main/src/vs/workbench/services/auxiliaryWindow/browser/auxiliaryWindowService.ts#L303-L370>、<https://github.com/microsoft/vscode/blob/main/src/vs/workbench/services/auxiliaryWindow/browser/auxiliaryWindowService.ts#L412-L497>。
- HTML Living Standard 定义 `setDragImage(element, x, y)`：浏览器从 element 生成拖拽位图，并把 `x/y` 作为位图热点坐标。来源：<https://html.spec.whatwg.org/multipage/dnd.html#dom-datatransfer-setdragimage>。
- WebView2 允许宿主把同 environment、同 profile、未导航的目标 WebView 设置为 `NewWindow`；目标顶层 window 会作为真实 `WindowProxy` 返回给 opener。来源：<https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2newwindowrequestedeventargs?view=webview2-1.0.3719.77>。
- Locus 当前使用 Tauri `2.11.5` / Wry `0.55.1`。`WebviewWindowBuilder::on_new_window`、`window_features(features)` 和 `NewWindowResponse::Create` 已封装 WebView2 `NewWindowRequested`、opener environment 与 `SetNewWindow`，无需维护自定义 WebView2 COM 窗口宿主。

### 已完成的运行模型证明

验证实例：`<runtime-root>\locus-app-test-<run-id>`，WebView2 DevTools：`http://127.0.0.1:19223`。

主窗口通过 Tauri `on_new_window` 接管 `window.open('about:blank#locus-shared-workbench-…')`，实测结果：

```json
{
  "opened": true,
  "closed": false,
  "documentAccessible": true,
  "ownerDocumentAdopted": true,
  "mainRealmElement": true,
  "eventClosureShared": true,
  "piniaFound": true,
  "piniaIdentityShared": true,
  "tauriWindowFound": true,
  "directCreateElapsedMs": 600.8
}
```

- 主窗口可同步读写子窗口 `document`，主 document 创建的元素挂入子窗口后 `ownerDocument` 正确。
- 子窗口节点的事件闭包继续访问主窗口同一对象。
- Vue 组件挂载到子窗口后响应式点击由 `count:0` 更新为 `count:1`。
- 子窗口组件取得的 `useWorkbenchStore(pinia)` 与主窗口 `pinia._s.get('workbench')` 对象身份相同。
- 子窗口是独立 WebView2 CDP target，Tauri 可按独立 label 获取并控制原生窗口。
- 克隆 stylesheet，并同步 `html/body` 的全部属性后，对现有顶部 Tab 抽样比较 `color`、`backgroundColor`、`fontFamily`、`fontSize`、`fontWeight`、边框、padding 与几何尺寸，全部一致。
- 直接创建约 `601ms`，阶段 5 必须提供未导航空白 WebView2 池；池仅预热 environment、profile、Controller 与 HWND，领取前不导航、不访问 DOM。

### 目标运行模型

```text
Main Vue / Pinia / Workbench Store / View Host Store
  ├─ main document
  ├─ child document A  ← Teleport: WorkbenchWindow
  ├─ child document B  ← Teleport: WorkbenchWindow
  └─ child document C  ← Teleport: ViewHostWindow

window.open(about:blank#token)
  → Tauri on_new_window
  → claim same-environment un-navigated WebView2
  → NewWindowResponse::Create
  → WindowProxy returned to main runtime
  → sync styles/attributes + mount Vue subtree
  → show native window
```

共享业务运行时保留一个 Store 与一个服务图。子窗口仍拥有自己的窗口尺寸、焦点、document、事件循环入口和原生 HWND。窗口相关操作必须携带显式 Tauri label，DOM 相关操作必须从元素解析 `ownerDocument/defaultView`。

### 实施范围

#### 5.1 Tauri 共享窗口与未导航池

- 主窗口 builder 注册严格白名单 `on_new_window`；仅接受 Locus 生成的 `about:blank#locus-shared-*` token。
- Rust 管理一个未导航共享窗口池。Windows 使用主 WebView2 的同一个 environment/profile；其他桌面平台使用 Tauri `window_features` 提供的 opener 配置直接创建，并保留无池回退。
- 池窗口创建在屏幕外并保持隐藏。领取后应用请求的 bounds、标题栏配置和主题背景；主线程立即补充下一窗口。
- 记录 `shared-pool-ready`、`shared-pool-claimed`、`window-proxy-ready`、`styles-synced`、`vue-mounted`、`window-shown`。
- 旧的已导航 `window.html?workbenchWindowPool=1` 池停止作为默认路径；迁移期保留独立窗口回退，完成验收后移除 Workbench 路由。

#### 5.2 前端共享宿主

- 新增 shared auxiliary runtime，维护 `label -> { WindowProxy, Tauri Window, container, stylesReady }`。
- 克隆主 document 的 stylesheet/link；同步新增、删除与文本变化；同步 `html/body` 的 `class/style/data-*` 与主题 meta。
- `App.vue` 在同一 Vue 应用树中通过跨 document Teleport 渲染 `WorkbenchWindow` / `ViewHostWindow`，继承现有 provide/inject、Pinia、i18n 与 diff overlay。
- `WorkbenchWindow` 接收显式 `windowId`、目标 DOM Window 和 Tauri window handle；窗口控制、拖动、bounds、关闭全部作用于该 handle。
- 引入 `ownerDocumentOf`、`ownerWindowOf`、`activeElementIn`、`requestAnimationFrameIn` 等轻量多窗口 DOM 工具。修改 Workbench/View Host 的全局 document/window 监听、焦点、查询、Teleport、菜单和快捷键。
- Workbench 状态直接共享；编辑器瞬态状态继续使用现有 snapshot adapter，在 Store 原子移动前导出、目标 mount 后应用。IndexedDB token/跨 WebView ACK 从共享主链路移除，仅供独立窗口回退。

#### 5.3 Chromium 原生 HTML5 DnD

- Workbench 与 View Tab shell 设置 `draggable=true`；关闭按钮、输入控件和其他交互子节点阻止启动拖动。
- `dragstart` 写入 Locus 专用 MIME token和最小 `text/plain` 兼容数据，设置 `effectAllowed='copyMove'`。
- 使用真实 Tab shell 调用 `setDragImage(shell, anchorX, anchorY)`；`anchorX/Y` 为 `DragEvent.clientX/Y - shellRect.left/top`，按 shell 尺寸钳制，不使用固定偏移。
- `dragenter/dragover` 仅执行本地几何命中与 CSS 插入反馈；`dragover` 调用 `preventDefault()` 并设置 `dropEffect`。目标窗口 `drop` 直接提交共享 Store 事务。
- `dragend` 在没有成功 drop 时读取一次原生 cursor position：释放于全部现有窗口之外则领取共享子窗口；释放在应用窗口非目标区域则取消。
- 保留现有 `useInternalDrag` 服务于 Workspace/Knowledge/资源布局拖动；Tab 不再被该控制器禁用 `draggable`。
- 移除 Tab 路径的 16ms Tauri 窗口轮询、GDI 预览、computed-style 跨 IPC 复制和 native preview 窗口。

#### 5.4 恢复、关闭与故障回退

- registry 继续持久化 label、bounds、最大化状态、split tree 与 Tab；启动恢复通过共享 runtime 逐个打开 `about:blank` 子窗口并挂载已有 Store 状态。
- 主窗口退出统一关闭所有共享子窗口。子窗口单独关闭执行 dirty 检查、卸载 Vue 子树、清理 document observer 与窗口状态。
- 主窗口 renderer 失效时所有共享子窗口随主窗口生命周期结束，避免遗留无业务运行时的空白 HWND。
- `window.open`、WindowProxy、样式同步或 Vue mount 任一步失败时关闭半成品窗口，源 Tab 保持原位；迁移期可以调用独立 Workbench 窗口回退。

### 延迟优化

- Chromium 合成拖拽位图；拖动期间不更新 Vue 浮层、不调用 GDI、不做逐帧 Tauri IPC。
- 同一运行时直接读取目标 DOM 和 Store；已有目标窗口落点不经过 emit/ack/IndexedDB。
- 未导航 WebView2 池提前支付 environment、profile、Controller、原生窗口和 CDP 注册成本。
- 样式列表与根属性快照在主窗口缓存；claim 后批量 append，在一个微任务内完成。
- 子窗口隐藏状态完成样式同步与首个 Vue commit；使用 `nextTick + setTimeout(0)` 确认 DOM 后显示，规避隐藏窗口 rAF 不触发。
- pool claim 后立即异步补池；全局最多保留一个空白池窗口。
- 高频 `dragover` 仅比较最近落点与目标 key，目标未变化时跳过响应式写入。

### 阶段 5 性能门槛

- 真实 Tab 拖拽图像与窗口内 Tab：尺寸误差 `<= 1` 物理像素，鼠标热点误差 `<= 1` 物理像素。
- 已有共享窗口 drop 到 Store 提交：P95 `< 32ms`。
- 空白池命中：释放到窗口显示 P95 `< 100ms`，释放到 editor ready P95 `< 180ms`。
- 直接创建回退：释放到窗口显示 `< 650ms`；当前实测基线 `600.8ms`。
- `dragover` 处理 P95 `< 4ms`，插入反馈延迟 `< 16ms`，拖动期间无超过 `50ms` 的 Locus 主线程 Long Task。
- 辅助窗口恢复 3 个窗口时，只出现 1 个 Vue/Pinia/应用 bootstrap；每个子窗口不加载 `window.html` bundle。

### 验证矩阵

1. Rust 单元测试：token 白名单、池状态、重复 claim、异常关闭、未导航约束与恢复 registry。
2. 前端单元测试：样式克隆/属性同步、跨 document host、ownerDocument 工具、共享 Pinia 身份、窗口卸载。
3. DnD 单元测试：MIME、effectAllowed、真实 DOM drag image、鼠标热点、插入/分屏意图、取消与外部释放。
4. CDP + CLI：主窗拖出、拖入已有子窗、子窗互拖、拖回主窗、最后 Tab 关闭、四向分屏、脏状态回滚。
5. 原生屏幕截图：拖动中捕获系统拖拽图像，和源 Tab 做尺寸、热点与像素差验证。
6. 性能：分别记录直接创建、池 claim、已有窗口 drop；重复至少 20 次计算 P50/P95。
7. 重启：恢复多个共享 Workbench/View Host 窗口，验证 bounds、split、Tab、active editor 与主题。
8. Unity CLI driver：运行 `connect`、`state-probe`，确认共享窗口不改变 workspace/runtime 所有权。

### 阶段 5 完成条件

- 所有可拖动 Tab 辅助窗口使用共享 Vue/Pinia 运行时；非 Tab 工具窗口保持现有独立入口。
- Workbench/View Tab 使用 Chromium 原生 HTML5 DnD 和真实 DOM drag image。
- 主路径不再使用 GDI Tab 预览、逐帧窗口轮询与 IndexedDB 跨 WebView交接。
- 自动测试、typecheck、生产构建、Rust check、CLI driver 与 CDP 验收通过。
- 达到阶段 5 性能门槛，并在本文件追加真实 P50/P95、截图与隔离实例路径。

## 阶段 5 实际实现与模型修正（2026-08-31）

### 已证明的运行模型

- Tauri `on_new_window` 接管主窗口发起的白名单 `window.open('about:blank#locus-shared-workbench-*')`，创建同 WebView2 Environment 的隐藏无边框子窗口并把真实 `WindowProxy` 交回 opener。
- 预热池由主 Vue 运行时持有一个已经建立 `WindowProxy`、完成样式同步并挂载空 `WorkbenchWindow` 的隐藏宿主。领取时只更新 bounds、写入 transfer token、显示与聚焦；这比只预建原生 HWND 更接近 VS Code 的 auxiliary window warm path。
- `App.vue` 通过跨 document `Teleport` 把 `WorkbenchWindow` 渲染到子 document；Vue app、Pinia、Workspace Context Store、Workbench Store 与服务实例只初始化一次。
- HTML5 DnD 的源端完全由 Chromium 处理：真实 `.base-tab-shell` 设置 `draggable`，`dragstart` 调用 `setDragImage(shell, anchorX, anchorY)`，拖拽图像直接来自当前 Tab DOM，热点保留鼠标在 Tab 内的实际相对位置。
- WebView2 不会把自定义 HTML5 DnD 的目标 DOM 事件转交给另一个 Controller。Locus 保留 Chromium 原生拖拽源与系统 drag image，同时用 Rust 在拖动期间以 8ms 周期发布物理鼠标点；共享运行时在目标 `ownerDocument` 内直接执行 `elementFromPoint`、Tab 插入与四向分屏命中。释放时只查询一次原生光标与 Tauri 窗口边界。
- 跨共享窗口交接直接调用目标运行时注册的 receiver；IndexedDB transfer record 与 Tauri emit/ack 保留为独立窗口兼容回退，已经退出共享主链路。

### 预热成本与延迟策略

隔离实例 `<runtime-root>\locus-app-test-<run-id>` 的启动与热态测量：

- 主窗口 WebView 页面完成：进程启动后约 `3.53s`。
- 首个共享 Workbench 预热窗创建：进程启动后约 `6.69s`，位于主窗口首屏完成之后，不进入首屏关键路径。
- 原生子窗创建到 `about:blank` page load start：约 `31ms`；空白页面 load 本身低于 `1ms`。
- 热态 `WindowProxy + document/style 同步 + 空 Workbench host` 补池：`180–207ms`。
- 进程采样中，共享 Workbench 没有新增独立 renderer；主 renderer 与原有独立 `sub-pool-1` renderer 保持两个，WorkBench 预热的常驻增量集中在隐藏 Controller、HWND、document 与空 Vue 子树。

池深度固定为 1。claim 后的补池延迟 `450ms`，随后通过 `requestIdleCallback(timeout=1000ms)` 创建，避免 `window.open` 的 180–207ms 同步成本阻塞当前 Tab 的首次渲染。空池异常关闭会同步清除 `preparedWindow` 引用，下一次调度可重新补池。

### 白窗故障修复

辅助窗口最后一个 Tab 迁走时，关闭顺序固定为：

1. 清理 registry、Workbench window state 与 pane workspace context。
2. 设置 close-request 放行状态。
3. `await appWindow.close()` 销毁 Tauri 原生窗口与 WebView2 Controller。
4. 从共享 host 列表移除 Vue Teleport，断开 style/attribute observer 并移除 container。

该顺序避免先卸载 Vue 内容、后原生关闭失败时留下可见的白色 `about:blank` 窗口。

### CDP + 原生鼠标验收

隔离实例：`<runtime-root>\locus-app-test-<run-id>`；独占 WebView2 调试端口：`19246`。

- 主窗 Tab 拖到桌面外：通过。
- 第二个 Tab 拖入已存在辅助窗：通过。
- 辅助窗 Tab 拖回主窗：通过。
- 辅助窗最后一个 Tab 拖回主窗：通过；原辅助窗 CDP target 消失，`lastTabWindowClosed=true`。
- 预热窗口显示：`34ms`。
- 首个 editor ready：`108ms`。
- 已有共享窗口接收 editor：`5ms`；源端提交：`19ms`。
- 目标命中计算最大值：`0.3ms`。
- 主窗回收 editor ready：`43–67ms`。

截图：

- `<runtime-root>\locus-app-test-<run-id>\logs\workbench-window-main.png`
- `<runtime-root>\locus-app-test-<run-id>\logs\workbench-window-auxiliary.png`

### 当前剩余范围

- Development Workbench 已完成共享运行时与 Chromium 原生 Tab drag image 迁移。
- `ViewHostWindow` 的 Tab 已迁移到 Chromium 原生 HTML5 DnD：复用真实 Tab DOM drag image、Rust 8ms 物理鼠标流与释放时单次窗口查询；原 16ms Tauri 轮询和 GDI Tab 预览已从 View Tab 路径移除。
- `ViewHostWindow` 的业务 host 与独立 View content pool 仍使用 `window.html` runtime。若范围要求 View Host 也共享主 Vue/Pinia runtime，需继续迁移 View Host 的窗口创建、组件 ownerDocument、Tab receiver、恢复协议与内容池；该工作不计入当前 Development Workbench 共享运行时验收。

### 原生 drag image 像素验收

隔离实例：`<runtime-root>\locus-app-test-<run-id>`；独占 WebView2 调试端口：`19246`。

- 源 Tab DOM：`189×30` 物理像素。
- Chromium 系统 drag image 差分边界：`190×30`，尺寸误差 `1px`。
- 预期热点约 `(94,15)`；截图热点 `(95,15)`，误差 `1px`。
- 截图：`<runtime-root>\locus-app-test-<run-id>\logs\chromium-native-drag-preview.png`。
- 同一轮真实鼠标操作完成拖出、拖入子窗、拖回主窗、最后 Tab 关闭；预热窗显示 `59ms`、首 editor ready `96ms`、已有子窗接收 `4ms`、命中决策最大 `0.1ms`。
- Tauri 文件拖放 handler 保持启用时该完整 Tab 路径通过，证明 Chromium drag source/drag image 可与 Tauri `CF_HDROP` 目标兼容并存。

### 启动预热成本结论

当前池预热的是同一主 renderer 下的共享辅助窗口 Controller、隐藏 HWND、空白 document、样式副本和一个空 Workbench Vue 子树。它不会重新执行 `main.ts`、创建第二套 Pinia/服务图，也没有在实测进程树中增加第三个 WebView2 renderer。

成本分成三段：

1. WebView2 原生子窗口与 `about:blank`：约 `31ms`，空文档加载低于 `1ms`。
2. `WindowProxy`、Tauri handle、样式与根属性同步：和 Vue host 挂载合计约 `180–207ms`。
3. 常驻资源：一个隐藏 Controller/HWND/document、克隆的 stylesheet/link 引用与空 Workbench 组件状态；业务 Store、后端连接、编辑器内容和会话数据继续复用主运行时，不重复初始化。

启动时序上，主页面约 `3.53s` 完成，首个共享池约 `6.69s` ready，池工作落在首屏之后。当前实现的风险集中在短时主 renderer CPU/JS 占用，表现为一次 `window.open + Teleport mount` 的 180–207ms 热身；池深度 1 将常驻成本和风暴风险限制在单窗。领取后补池先等待 `450ms`，再进入 `requestIdleCallback(timeout=1000ms)`，避免与刚显示窗口的首个 editor commit 竞争。

收益是把首次外拖的直接创建基线约 `600.8ms`，压到窗口显示 `34–59ms`、editor ready `96–108ms`。以用户可感知延迟换算，单个池窗支付约 0.2 秒后台热身，首次拖出减少约 0.5 秒等待；桌面端进程启动后预热 1 个 WebView 的收益明显，继续提高池深度会线性放大 Controller/document/Vue host 常驻成本，当前没有数据支持超过 1。

### 最终自动化与 Unity CLI 验证

- 本次窗口相关 Vitest：`9/9` 通过。
- 全量 Vitest：`2230/2232` 通过；两个剩余失败是并行分支中的 CDP Debug 停止函数签名旧断言、Embedded Chat 重构后的旧函数名断言，均未覆盖本次窗口迁移文件。
- `bun run typecheck:test`：通过。
- `bun run build`：通过。
- Unity CLI `connect`：插件 `upToDate`，`native_broker` 连接成功，`1 passed / 0 failed`，`finished.ok=true`。
- Unity CLI `state-probe`：编辑、域重载、Play/Pause/Resume/退出 Play 全矩阵 `28 passed / 0 failed`，`finished.ok=true`；20 次原生栈采样最大冻结窗口 `57µs`，低于 `5ms` 预算。
