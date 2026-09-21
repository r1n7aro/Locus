# macOS 适配规模与 Windows 回归风险

日期：2026-09-21。基线：`cf70db9636ee61e74c779bec1472111ff389e00d`。目标同时覆盖 Apple Silicon 和 Intel；不参考旧 macOS 分支。本次仍只分析和更新文档。

配套文档：[平台扫描报告](./macos-portability-audit-2026-09-21.md)。

## 1. 直接结论

**有可能影响 Windows，而且底层通信/生命周期重构造成的风险明显高于新增几个平台分支。风险可以通过后端隔离、兼容约束和真实 Unity 回归控制，但不能仅凭条件编译承诺零回归。**

当前较好的条件是：Windows named-pipe 客户端、native broker、后台 hook 和大量 Win32 功能已经有条件编译边界。可以保留这些实现，为 Mac 新增并列后端；不必先重写 Windows 通信，也不必为了 Mac 基础连接修改 Windows 的 hook。

当前较大的风险是：请求协议、domain reload、状态共享内存、后台 hook 归属、C# 执行线程和插件安装彼此关联。它们分布在共享入口与 C# 文件中，不能全部依靠 Rust 的 `cfg` 隔离。

建议收紧首次实施范围：**先做平台边界和 Mac 新后端，Windows 后端内部不做整理式重构；等 Mac 基础闭环稳定后，再单独评估公共代码提取。**

## 2. 规模估算

下表是根据当前源码形成的拆分前预算，不是已生成的 diff、工期承诺或精确统计。文件数包含实现和配置，不含测试、生成文件、第三方源码、二进制资源；行量指预计新增/实质改写的文本实现量。各阶段可能修改同一文件，不能直接相加文件数。

| 阶段 | 预计规模 | 不确定性 | 对 Windows 的主要风险 |
| --- | --- | --- | --- |
| A：启动、构建、资源、运行时和更新入口 | 约 20–35 个文件，约 1,000–3,000 行 | 中；内置 Git/Python 与系统依赖路线会改变范围 | 共享构建脚本、资源查找优先级、runtime/installer 选择 |
| B：Unity 基础闭环，新增 Mac 通信/broker/状态面/进程后端 | 在 A 上再涉及约 12–20 个文件，约 3,000–6,000 行 | 中高；需要 Mac 实机反馈确认 Unix IPC、进程身份和 native 加载 | 如果保持 Windows 后端不动，风险集中在共享入口、C ABI/C# 接入、项目标识与安装器 |
| C：原生增强对齐，包括后台 hook、热更、嵌入窗口、原生探针和图形调试 | 大型独立工作包，不适合在可行性验证前承诺行数上限 | 高；两个 CPU 架构、Unity 版本、原生符号与运行时行为叠加 | 若同时升级共享 MonoMod、统一 patch 引擎或重写窗口生命周期，Windows 风险最高 |

A+B 是数十个文件、数千行实现的量级，不是小补丁。测试代码、fixture 和双平台验证需要另计。完整功能对齐 C 不应被捆绑成“编译适配的顺手修改”。

作为规模参照，以下是当前相关文件的物理行数，含注释与测试，**不代表这些代码都要修改**：

| 当前代码范围 | 物理行数 | 与适配的关系 |
| --- | ---: | --- |
| `transport.rs` + `transport/requests.rs` | 1,495 | Windows 请求关联、ACK、断线、超时和事件路由 |
| `locus_native_plugin/src/lib.rs` | 2,387 | broker、共享内存、后台 hook、overlay 客户端、FFI 和测试混在同一文件 |
| `LocusBridge.Native.cs` | 745 | native 加载、managed lifecycle、心跳、线程调度和 overlay 接入 |
| `unity_bridge/mod.rs` | 8,050 | 对外编排、状态面、连接恢复、请求重试、hook 归属等共享逻辑 |
| `process.rs` + `plugin.rs` | 2,846 | Editor 身份/关闭和插件升级 |
| `state_probe.rs` + `background_hook.rs` | 4,003 | 状态语义及原生探测/patch |
| `unity_embed.rs` + `LocusEditorWindow.cs` | 8,955 | 嵌入窗口与 overlay 控制，适合另做阶段 |
| `LocusBridge.HotReload.cs` + `unity_hotreload/coordinator.rs` | 5,867 | 热更应用、detour、回滚与编译协调 |

## 3. 通信并不是单个 OS API 的替换

当前至少有三条不同职责的底层通道：

1. **命令/事件通道**：Locus 的 Windows pipe 客户端连接 Unity 进程内 native broker，由 C# managed executor 执行 Unity API。见 [transport.rs](../../src-tauri/src/unity_bridge/transport.rs) L15、[native broker](../../locus_native_plugin/src/lib.rs) L90。
2. **独立状态通道**：broker 发布共享内存，Locus 读取生命周期、状态与事件游标。`query_native_broker_status_payload` 直接读取共享内存，当前不是查询命令 pipe 的普通 fallback。见 [mod.rs](../../src-tauri/src/unity_bridge/mod.rs) L456 / L686。
3. **overlay 控制通道**：Unity 中的 native 客户端连接 Locus 的控制 pipe，方向与命令通道相反，并保留跨 domain reload 的窗口状态。见 [native overlay](../../locus_native_plugin/src/lib.rs) L1919、[unity_embed.rs](../../src-tauri/src/commands/unity_embed.rs) L2722。

Mac 只实现第一条通道可以收到消息，却不等于连接/重载状态判断和 overlay 已经正确。第二条通道也不能随意改成“和长任务共用同一把锁的查询”，否则可能让监测链在主线程繁忙时一起失效。

### 对 Windows 最敏感的通信语义

| 语义 | 当前证据 | 重构失误可能造成的后果 |
| --- | --- | --- |
| 请求 ID 与响应关联 | `reader_loop` 使用 `reply_to`；非响应事件只由 active connection 派发 | 旧连接事件污染新连接、响应串线、任务永不结束 |
| ACK 与结果先后顺序 | [requests.rs](../../src-tauri/src/unity_bridge/transport/requests.rs) 明确允许结果早于 ACK，并保留近期请求记录 | 将正常迟到 ACK 当异常，或把“已接收”误当“已完成” |
| 取消和超时 | pending guard 清理本地等待；已发送的 frame 不会因此从 Unity 撤回 | 错误重试带来重复执行，或让取消后的请求继续占用资源 |
| 重连 | broker 的 `discard_non_reattachable_on_disconnect` 只保留可重接的执行请求 | 将普通写操作重新执行，或丢失原本可以接续的长任务 |
| domain reload | `interrupt_for_reload` 清理队列/inflight；旧 completion 被丢弃；generation 随域变化 | 旧结果写入新会话、永远卡在 reloading、错误报告任务成功 |
| C# 执行线程 | [LocusBridge.Native.cs](../../locus_unity/Editor/LocusBridge.Native.cs) L279 起明确切换线程池，Unity API 再回主线程 | 把序列化/编译/反射工作移回 Editor 主线程，出现卡顿或死锁 |
| 弹窗屏障 | [transport.rs](../../src-tauri/src/unity_bridge/transport.rs) L495 起在发送前订阅并检查模态窗口状态 | Editor 对话框出现后请求挂死，或误判执行状态 |
| workspace/service generation | transport、state probe 与 workspace scope 绑定 | 工作区销毁/重建后继续消费旧事件，产生跨项目串扰 |

这些都不是“替换 `NamedPipeClient` 类型后编译通过”能证明等价的。

## 4. hook 必须拆成三类看

### 4.1 后台运行 hook：属于原生代码 patch

[background_hook.rs](../../src-tauri/src/unity_bridge/background_hook.rs) L430 当前使用固定补丁字节 `B8 01 00 00 00 C3`，并通过 PDB/DbgHelp 寻找 `Unity!IsApplicationActive` 与 `Unity!IsApplicationActiveOSImpl`，再调用 `VirtualProtectEx` / `WriteProcessMemory` 等 API 修改目标进程代码。

native broker 内还有一份进程内 hook，位于 [lib.rs](../../locus_native_plugin/src/lib.rs) L1454。两份实现约定相同符号和补丁，但各自记录 patch 所有权，避免恢复别人的补丁。

这类功能不能将 Windows patch 字节或 PDB 查询流程直接用于 Apple Silicon。Mac 要单独验证符号可得性、代码页权限、指令集、缓存同步、失败回滚和 Unity 版本兼容性；Intel Mac 也不能因同为 x64 就复用整个 Windows 实现。

**Windows 的高风险点在 patch 所有权与恢复：** [mod.rs](../../src-tauri/src/unity_bridge/mod.rs) L2206 / L2584 会读取 broker 的 `background_patched`，据此让外部 patch 停手；部分 marker/连接状态还会延后外部 patch。若在统一 Mac/Windows 状态时改变这些判断，可能触发重复 patch、漏恢复，或恢复了另一份实现仍依赖的代码。

第一阶段 Mac 可以明确报告后台 hook 不可用，但需要验证 Unity 未聚焦时的行为并限定产品能力；不能因此关掉 Windows hook 或改变其原有归属判断。该取舍需要在 Mac 实测后确定，当前没有把它当成已接受的最终产品范围。

### 4.2 热更 hook：MonoMod Detour / NativeDetour

[LocusBridge.HotReload.cs](../../locus_unity/Editor/LocusBridge.HotReload.cs) L397 先尝试 `Detour`，失败后转 `NativeDetour`。它与后台运行 patch 是两套机制。

[build-locus-detour-bundle.mjs](../../scripts/build-locus-detour-bundle.mjs) 固定 MonoMod `21.12.13.1`，使用 net452/net40 产物与 Unity Mono 兼容。本次没有证明该固定版本能覆盖目标 Apple Silicon Unity 组合。

若后续为 Mac ARM64 升级 MonoMod 或更换 detour 引擎，同一 `Locus.Detour.dll` 也可能被 Windows 加载，直接影响 Windows 的方法替换、签名兼容、连续热更和 Dispose/回滚。这是共享二进制依赖风险，即使大部分 Rust Windows 源码不改也会出现。

建议把依赖升级做成单独变更；如要按平台分包，要先处理同一 assembly identity 的 importer 互斥与安装校验，并实测加载结果，不能简单放两份同名托管程序集进 Unity。

### 4.3 窗口/输入 hook 与原生状态探针

Win32 窗口 subclass、鼠标/焦点/弹窗同步、overlay 和读取 Unity 原生状态的探针也有各自的生命周期，不应与 IPC 移植一起重写。它们可保留 Windows 路径，Mac 另做平台实现或明确降级。

## 5. “只加 Mac 分支”仍可能波及 Windows 的位置

| 边界 | 原因 | Windows 保持兼容的约束 |
| --- | --- | --- |
| 项目标识 | pipe、共享内存、marker 依赖同一项目 hash；Rust 客户端、native broker、C# 各有对应规则 | Windows 路径归一化、hash、命名空间保持兼容；Mac 引入独立规则，不全局替换为 Unix 路径规则 |
| FFI / wire protocol | C ABI、managed state 整数、JSON 字段与 protocol version 被两端共同使用 | 保留现有 Windows ABI 和字段；新增能力用协商表示，别把旧缺失字段解释为成功 |
| 共享状态结构 | magic/version、slot 布局、序列号和事件游标由 reader/writer 配合 | Windows MMF 格式和读写顺序不随 Mac 实现变动；必要格式升级单独设计兼容性 |
| 插件安装 | [plugin.rs](../../src-tauri/src/unity_bridge/plugin.rs) L619 对过滤后的整个插件目录内容计算 hash，安装时整包替换 | 新增 Mac 文件也可能让 Windows 项目提示插件更新；这是分发影响，不等于 Windows 后端出错，但必须纳入升级验证 |
| 运行中插件 | Unity native DLL 跨 domain reload 保持加载 | 更新后重启 Unity 验证，不能把加载着旧 DLL 的结果当成新实现通过 |
| 构建和依赖 | `package.json`、生成脚本、Cargo.lock、托管 DLL 和资源配置为共享入口 | 避免顺带升级 Windows 依赖；两个平台输出目录隔离，避免 Mac 产物覆盖 Windows 产物 |
| 配置与功能开关 | 一个全局布尔值可能被误用为所有平台支持判断 | 启用状态与平台支持能力分开；Mac 不支持某能力不能改变 Windows 默认值 |

分支/worktree 只隔离源码。运行新分支时还必须使用独立 Locus profile 和独立 Unity 测试项目；向同一个 Unity 项目安装测试插件，或共用正式配置/数据库，会影响其他分支的运行态。仓库已有 `locus:test:app` 和 Unity driver，可继续沿用其隔离约定。

## 6. 两条实施路线的风险对比

| 路线 | 做法 | Windows 风险 | 代价 |
| --- | --- | --- | --- |
| 推荐：并列平台后端 | 现有 Windows 内部逻辑保留；通过当前对外函数分派到新增 macOS backend；C# 仅在必要平台入口分支 | 相对较低，仍需重点验证共享入口/依赖/安装 | 短期会有部分实现重复，需用同一组协议用例约束行为 |
| 一开始统一全部底层 | 把 pipe/socket、队列、共享内存、reload、hook、进程处理同时改成一套公共框架 | 高，Windows 的锁顺序、时序、drop/cleanup 和生命周期都进入改动面 | 代码可能更统一，但排错难以区分原有行为与移植引入的变化 |

“并列”不是复制整个应用：上层工具协议、业务逻辑和数据模型继续共享；暂缓的是已经稳定运行的底层状态机大搬迁。若确有小范围公共类型需要提取，单独提交并先证明 Windows 行为保持，再接 Mac 实现。

### 建议拆分成可独立回退的变更

1. 平台目标/构建资源配置与测试，不碰 Unity 通信和 hook。
2. 资源目录、运行时、更新入口；Windows 安装包和已有数据路径回归。
3. 新增 Mac native IPC、状态面和进程后端；Windows 内部实现保持原状。
4. C# loader/endpoint、插件 importer 与安装逻辑；验证 Windows 的旧插件升级与重启加载。
5. Mac 功能门控与真实基础闭环验收。
6. 后台 hook、热更依赖升级、原生探针、窗口增强分别实施，不与第 3 步合并。

不建议为了目录整齐同时移动整个 `transport.rs` / native `lib.rs`，也不建议在这一轮顺便统一旧错误码、重命名 protocol 字段或改 Windows 路径大小写策略。

## 7. 合入前如何证明 Windows 仍正常

每个阶段按影响范围选取测试，不在纯文档/纯样式修改上重复全部 Unity 集成测试。触及通信、broker、状态面或 C# bridge 后，至少要覆盖下列真实行为：

| 场景 | 必须观察的结果 | 已有入口/补充需求 |
| --- | --- | --- |
| 冷启动与已运行 Editor 连接 | 找到正确项目/进程，首次并发连接不相互拆连接 | `connect`；transport 已有 single-flight 测试 |
| domain reload | native 通道持续可观测，generation 变化正确，旧 completion 不进入新请求 | `native-bridge` 自测明确包含真实 script reload |
| Edit/Play/Pause 与主线程繁忙 | 不将 pause、重载、弹窗或卡顿错误识别为退出/ready | `state-probe`、`modal-dialog`，结合实际状态输出 |
| 超时、取消与断线重连 | 可重接执行不重复开始；普通请求不在新连接意外继续执行 | native/transport 单测 + `execute` 的真实长任务场景 |
| 后台 hook 开关与恢复 | 开关生效、未聚焦行为正常、reload/退出后恢复符合归属 | 增加/执行明确的 hook 生命周期验收；现有常量测试不足 |
| 连续热更与回滚 | 多轮方法替换、失败恢复、Dispose 和 reload 正确 | `hot-reload` / `hot-reload-release`，仅在相关修改后执行 |
| 多工作区/多个 Editor | 无跨项目事件与状态串扰，不误关其他 Editor | `workspace` / `workspace-switch`；独立运行其 suite |
| 插件升级 | 正确处理已加载 DLL，重启后确认实际加载的新版本 | `--install-plugin` 的升级流程及 native 状态验证 |
| 资源/安装包 | 开发版和安装版均能找到运行时、插件、Agent/skill，数据路径正常 | Windows NSIS 安装与启动 smoke；只跑 dev 不够 |

通信时序变化还应与同基线比较：连接耗时、请求完成/取消延迟、状态心跳连续性、重载恢复时间、CPU/内存以及子进程/连接清理。先固定用例与基线再判断回归，不设没有测量依据的“性能不受影响”承诺。

回退也需覆盖应用与 Unity 插件的组合。保留旧 Windows endpoint/协议会降低回退风险；如果确实升级协议，应增加版本拒绝/兼容策略，不能让旧应用在新插件上静默运行。

## 8. 本轮实际验证与尚未证明的部分

在 Windows 的同一代码基线执行：

```powershell
cargo test --manifest-path locus_native_plugin/Cargo.toml --lib --locked --offline
bun run test src/__tests__/nativeBridgeMigration.test.ts src/__tests__/unityBridgeCompatibility.test.ts src/__tests__/unityModalDialogBarrier.test.ts src/__tests__/unityExecuteProgress.test.ts
```

结果为 native **8 项通过**，Vitest **4 个文件、35 项通过**。native 测试覆盖队列限制、接收确认、迟到 completion、重接请求保留、共享状态事件及 payload 上限、overlay 消息和 hook 常量。

证据边界：上述 Vitest 文件主要是源码约束检查；native hook 测试检查补丁字节/符号常量，不会向真实 Unity 注入后验证恢复。因此这些结果建立当前基线，**不能证明未来重构没有 Windows 回归，也不能代替 Windows Unity 集成验收**。

本轮未改产品代码、依赖或 Unity 插件产物，未运行真实 Unity 集成测试，未触碰正式项目/配置/数据库；只新增本风险分析并修订扫描报告中的实施顺序。
