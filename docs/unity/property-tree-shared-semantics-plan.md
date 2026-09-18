# Property Tree 共享语义层实施方案

## 目标与边界

View SDK、Agent YAML read/search 与批量资产编辑复用 Rust 语义规则。模式在 API
实例上选择，现有控件不新增模式参数。YAML 保证支持范围内的序列化数据及 Unity
重载结果一致；任意构造函数、CustomEditor、OnValidate 和原生 Undo 不属于通用
离线等价承诺。不支持的能力明确报错，不自动切换 live。

## 分层

```text
View / Agent / SDK
  ├─ API 实例、作用域、草稿、批次、取消
  └─ Rust 共享语义层
       ├─ 精确对象身份、逻辑路径 ↔ 持久化路径
       ├─ 类型证据、值规范化、宿主内引用图
       ├─ YamlPropertyTree 投影（字段 / hierarchy / subassets）
       ├─ 后续：Prefab 来源链、有效值、覆盖归属
       └─ 写操作编译 → unity_asset_core → storage 事务 → Unity 集中导入
```

`unity_asset_core` 保持无 IO 的不可变编辑内核。共享地址与引用解析放在其独立模块，
能够被轻量 Rust harness 直接测试。`YamlPropertyTree` 继续作为现有 Snapshot 的
投影实现；View 传 target，Agent 传 semantic path，二者复用规范化文档值及投影逻辑。
前端只传递请求、精确整数和批次，不继续维护 YAML AST/引用解析器。

## 契约

- 对象身份使用资产路径及十进制字符串 fileID；managed ID 属于宿主，不跨宿主共享。
- 逻辑路径如 `node.next.amount` 经引用图解析到稳定 `@rid=...` 持久化路径；数组索引
  仍表示当前顺序。写入同一共享对象的别名应命中相同持久化节点。
- 树展开按访问过的对象身份去重；循环/共享节点保留 canonical 信息，不无限展开。
- 批次保留操作顺序，同文件共用版本。提交时统一预检，继续沿用事务日志、磁盘版本
  校验、dirty 检查与集中导入。未知结果禁止自动重放。
- 后续 Prefab 解析必须返回来源链及写入层，并纳入所有读取依赖的 revision。
  Revert 是删除覆盖，不能用“写入当前来源值”代替。
- 类型创建必须显式选择初始化数据或经过版本校验的模板；新增 registry、rid 分配、
  字段重定向和旧图清理在一个候选图中校验。不得声称复制模板等价任意 `new T()`。

## 分阶段交付

| 阶段 | 内容 | 通过标准 | 状态 |
| --- | --- | --- | --- |
| P0 | API 实例模式选择、显式批次、基础 YAML/live 对照 | 两版本 Unity 基础回归通过 | 已完成 |
| P1 | Rust 共享地址/值层；View 移除本地 YAML 投影；复用 Agent Tree；已有 managed graph 字段读写 | 前端仅 IPC；逻辑地址与原始 rid 地址同值；别名/循环编辑一致；原有分页、整数、批次不回退 | 已完成 |
| P2 | 文本 Prefab 递归继承读取；普通字段 Override/Revert | Base→Variant→Scene 传播、重复嵌套身份、同值覆盖与删除覆盖差异、来源版本冲突 | 已实现，见下方边界 |
| P3 | 显式数据/模板的 SerializeReference 实例创建与类型替换 | 空值创建、类型兼容、rid 冲突、别名保留、循环模板重映射、缺失类型、保存重载 | 已实现，见下方边界 |
| P4 | 跨层 Apply、实体对象模板增删、复杂内建值和优化 | 引用拓扑、跨文件事务、两版本差分与性能采样 | 已实现首版，见下方边界 |

P1 不提前声明 P2/P3 能力。阶段完成后更新实际交付、运行命令与结果；不以暂时跳过
失败用例的方式通过验收。

### P1 实际交付（2026-09-17）

- `unity_asset_core/semantic.rs`：精确身份、逻辑路径、宿主内 registry 索引、按顺序变化的
  引用图及写操作编译。批内重定向引用或移动数组后，后续编辑跟随新地址。
- `YamlPropertyTree`：Agent 与 View 共用规范化文档值、节点构建和引用投影；为直接
  属性读取建立路径索引，分页与循环展开继续遵循现有 Snapshot 协议。
- `unity_assets/property.rs`：`read_property` / `apply_properties` 统一入口，复用既有
  作用域锁、版本检查及事务层。前端 `unityYamlProperty.ts` 仅做精确值编码和 IPC。
- `AssetField.type_hint`：携带源码证明的布尔、浮点、字符串类型，不改变原始资产值
  协议；修复数组扩容后提交响应与重新读取的类型提示不一致。
- 保留既有 Property Tree 控件与样式。引用槽本身仍只读，已有 managed 对象子字段
  可以编辑；完整 Editor 元数据、Prefab 继承写入与类型创建仍按后续阶段开放。

验收结果：

| 测试 | 结果 |
| --- | --- |
| 纯 Rust 编辑内核与语义测试 | 57 通过（含 10 项共享语义测试） |
| Rust 资产服务、存储与入口测试 | 24 通过（含 5 项共享 Property 服务测试） |
| 原有 Agent Property Tree 回归 | 42 通过 |
| Vitest API、Tree、View runtime 和控件测试 | 81 通过 |
| `bun run typecheck:test` | 通过 |
| Unity 6000.5.8f1 隔离回归 | 44 通过 |
| Unity 2022.3.47f1 隔离回归 | 44 通过 |

Unity 证据保存在本次独立目录 `<runtime-root>/property-review-<run-id>` 和
`<runtime-root>/property-review-<run-id>` 的 `property-review-results.json`。
这些通过项不代表 P2/P3 已通过；后续继续增加继承传播、覆盖归属和创建后的图身份测试。

## 一致性测试矩阵

| 范围 | 比较内容 |
| --- | --- |
| 地址与数值 | fileID/rid 精度、Int64/UInt64、数字形状字符串、数组分页、escaped pointer |
| 已有引用图 | 逻辑字段/原始 registry 字段一致；别名写入可见性；自循环、双节点循环、不同宿主同 rid |
| 批次 | 新增元素后写子字段、删除先前目标、父子字段顺序、同文件重复写、取消和失败重放 |
| Prefab（P2） | Base/Variant/Scene 有效值与来源、嵌套实例不串写、Revert 后继续传播、来源变化拒绝提交 |
| 创建（P3） | 类型标识、assignability、初始化策略、共享身份、模板 rid 重映射及旧图存活性 |
| Unity 差分 | 同一初始 fixture，YAML 与真实 PropertyTree API 编辑后比较值/身份/覆盖，保存重载后再比较 |

测试分为纯 Rust 语义测试、Vitest IPC/交互测试、隔离 Unity 2022/6 集成测试。
现有 `locus:test:property` 的 Rust driver 应调用生产共享解析器，避免测试自己实现
另一套逻辑路径转换。完整存储事务和 SDK 贯通继续使用现有 asset-api suite。

### P2–P4 实际交付（2026-09-17）

- `unity_asset_core/prefab.rs` 递归合成来源图；优先使用 stripped 身份，再使用两版本
  Unity 实测的虚拟 fileID 映射。多实例按实例身份隔离，循环来源、身份冲突明确拒绝。
  Agent read/search 和 View 使用同一来源图；Agent 缓存同时校验来源内容版本。
- `property.discover()` 在 YAML 模式返回有效字段及精确 target，View 无需推算虚拟 ID。
  `read()` 返回 `prefabLayers`、`revision` 和 `dependencies`；直接写入携带
  `expectedRevision`、`expectedDependencies`，bound Tree 自动保存这些版本。
- `{action:"revert"}` 删除当前层覆盖；`{action:"applyToSource",level:1}` 将当前值
  应用到下一层并删除被跨过的覆盖。多层 Apply 显式给出 level，所有变化文件进入同一日志事务。
- `{action:"createManaged",template:{rootRid,entries}}` 创建或替换当前宿主中的引用槽。
  template 提供完整 type/data；所有内部 rid 重映射，循环关系保留，原有别名继续指向旧图。
  验证实际类声明、Serializable、assembly、assignability 和全部序列化字段；缺失/抽象/
  泛型/不明确声明拒绝。该操作不执行构造函数。旧图保留，由 Unity 保存时收集不可达条目。
- `{action:"editObjects",add,remove,updates}` 以完整模板编辑实体 GameObject、Transform、
  RectTransform 和 MonoBehaviour（含 ScriptableObject）文档。新增 ID 不得碰撞；所属
  GameObject 的组件列表、Transform 父子关系和引用在最终候选图中统一校验。
- 曲线和渐变接受与 live API 相同的标准 payload，共享层负责序列化版本及枚举编码转换。
  两版本实测修复了曲线 infinity 编码、空 rid registry 和不透明 Hash 字段保留问题。
- 连续同一标量的 Set 共享渲染补丁，但所有请求仍逐条验证；数组和父子路径保持顺序屏障。
  大规模基础编辑继续走批量 span editor，创建和拓扑操作只重写变化字段/文档。
- 只读来源及类型证据进入提交前后检查；Unity 在线时同时检查来源 dirty 状态和 SHA256。
  新外部引用按整个事务的最终文件集合验证，虚拟 Prefab 对象通过来源图验证。

当前明确边界：继承数组扩容、继承 managed 创建、Prefab 的 AddedComponent/AddedGameObject
覆盖结构、跨层引用 Apply、非有限曲线切线尚不支持；遇到这些布局或命令明确报错。
结构模板当前面向实体层，不能据此宣称完整 Prefab topology authoring。
曲线/渐变的 API 读写已支持，默认 YAML Tree 只展示预览；现有浮动编辑器仍拥有 live
写通道，因此不从 YAML Tree 开放它，避免误写 live。未增加任何模式选择 UI。

累计验收：纯 Rust 内核 69 项、资产服务 29 项、原有 Agent Tree 42 项、Vitest 8 文件
84 项及 `bun run typecheck:test` 均通过；两版本 Unity 各 53 项通过。Unity 证据：
`<runtime-root>/property-review-<run-id>/property-review-results.json`（6000.5.8f1）及
`<runtime-root>/property-review-<run-id>/property-review-results.json`（2022.3.47f1）。
Rust 运行记录见 `.tmp/property-p4-rust.log` 和 `.tmp/property-p4-agent-tree.log`，
纯内核/语义测试通过独立 driver 验证。

性能采样：同一文件 1,000 次连续标量写入，两版本均仅导入一次；Unity 6 候选准备
69.03 ms、写盘及导入 29.22 ms；Unity 2022 分别为 69.52 ms、24.02 ms。
这是 debug driver 的单次样本，准备时间含进程启动，未计入服务层源码扫描，写盘及导入
当前合并计时；不作为
大资产、不同字段、复杂 Prefab 图的吞吐承诺。后续性能工作仍需扩展独立阶段计时及规模矩阵。

## 后续性能策略

每批捕获一次文档及类型证据，复用规范化值、对象索引和 registry 索引；投影按深度与
分页预算展开。Prefab 来源图按内容版本缓存，批量解析共享依赖。保留原始 asset batch
入口给已知持久化路径的大规模编辑，避免强制生成完整 UI 树。性能记录分别报告解析、
编译、写盘、Unity 导入时间，不用单一总耗时掩盖主线程导入成本。

## 后续强化：批次性能与一致性

此次优化保持 API 实例选择 backend，不增加界面模式控件。

- 来源图、Tree 和依赖版本在一个事务阶段内复用；连续同文件的继承标量写入按来源
  合并校验，并按实例一次更新覆盖列表。每条输入仍经过 schema、值形状和精确数值
  校验，非法中间输入不会被最后一次赋值掩盖。
- Revert、跨层 Apply、创建、结构操作和文件切换是顺序边界。边界后刷新阶段缓存，
  后续写入使用当前候选图；没有跨请求复用磁盘快照。解析、候选准备及大批响应组装
  移到阻塞工作池，Unity 主线程仍只负责必要的预检、替换协调和导入。
- Prefab stripped 身份建立索引，不再对每个继承对象反复扫描所有文档；显式 stripped
  ID 优先于推算 ID，即使候选 XOR 为零也不拒绝已有的合法显式映射。
- Unity `PropertyModification.value` 固定先按字符串读取，再由来源字段解释。
  `007`、`1e3`、`null`、空文本和数字形状长文本保留原样；小整数来源被覆盖为大整数后
  转换为精确 Int64/UInt64 envelope，跨层 Apply 不再经过不安全的普通 JSON 数字。
- 批量覆盖在修改任何实例之前验证所有记录，失败不会删除原有覆盖。GUID 比较不区分
  大小写；无关重复 GUID 不阻止读取，实际来源 GUID 歧义仍拒绝。Windows 路径别名在
  进入事务前拒绝，单独使用不同大小写路径仍能捕获对应 `.meta` 版本。
- 已使用的脚本及 `.cs.meta` 纳入 Prefab read 返回的依赖版本；携带依赖的写入不会
  绕过版本检查。保留 GUID 索引时读取的脚本 metadata，避免索引后再次读取产生新旧
  身份混用。提交前后继续核对这些证据。
- 字节不变的离线事务不替换目标文件，保留文件修改时间；在线路径继续跳过无变化资产导入。

可选诊断：`yaml.apply({writes, profile:true})` 返回 `profile`。`prepareMs` 包括加载、
编译、校验和候选准备；`commitMs` 包括日志事务及在线 Editor 往返；`projectionMs`
记录提交后响应组装。高级路径额外返回 `effectiveBuilds`、`treeBuilds`、
`overrideValidationPasses`（批量覆盖的来源校验轮数）和实际字节变化的 `changedFiles`。
Unity 在线时 `editor` 包含 `preflightMs`、`writeMs`、`importMs`、`changedFiles`；
`writeMs` 含 Start/StopAssetEditing 包裹的文件替换，`importMs` 含显式导入和重载。
离线时 `editor` 为 null。计时不包含前端 IPC 编解码，不应直接等同于 UI 总延迟。

完整 Rust 服务基准使用 8 个实例、每实例 33 个字段；包含校验、事务写盘和全部请求结果。
相同的 128/1,000 请求两组测试，基线合计 26.17 秒，优化后约 0.88 秒。
一次优化样本分别为 167.52 ms 和 626.15 ms；再加入类型证据及 129 个源码文件，
1,000 请求样本约为 641–658 ms。两个 1,000 请求场景都只有 2 次来源图构建、2 次 Tree
构建和 1 轮批量覆盖来源校验。上述为同机 debug 单次样本，不能推广到所有资产规模。

基准日志：`.tmp/property-perf-baseline.log`、`.tmp/property-perf-optimized.log`、
`.tmp/property-perf-typed.log`。基准不设脆弱的时间阈值，常规回归断言重建次数、顺序、
精确值及失败不落盘；性能用例显式运行：

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --lib batch_service_benchmark --no-default-features -- --ignored --nocapture
```

Unity 新增 YAML17–18，比较实际 Unity 保存的字符串覆盖及小值到 Int64 最大值的
Override/Apply。两版本各 55 项通过：
`<runtime-root>/property-review-<run-id>/property-review-results.json`（Unity 6），
`<runtime-root>/property-review-<run-id>/property-review-results.json`（Unity 2022）。
YAML14 同时检查单次导入和分阶段计时；本轮没有扩大之前明确拒绝的 Prefab 拓扑等能力范围。

本轮累计验证：纯内核 74 项、资产服务 34 项、Agent Tree 42 项、前端 85 项及类型
检查通过；两个默认 ignored 的服务性能用例已单独执行通过。服务最终记录为
`.tmp/property-perf-rust-final.log`，类型证据基准补充记录为
`.tmp/property-perf-typed-final.log`。沿用原有 Tree/控件样式，未新增界面元素。

## Review 修复与规模验证（2026-09-18）

先将 review 的 6 项发现固化为生产服务回归，6 项均在修复前失败，记录为
`.tmp/property-fix-red.log`。另增加完整服务合成测试：真实 Unity 创建 fixture 并退出，
生产 `unity_assets::execute` 执行类型校验、批次编译及磁盘事务，再由 Unity 重载并与
SerializedObject 操作对照。该用例修复前因合法嵌套类型被拒绝而失败，记录为
`.tmp/property-fix-native-red.log`，不是 mock 或测试专用写盘路径。

修复与实现变化：

- 分离源码类型名和 Unity registry identity，嵌套类严格使用 `Outer/Inner`；拒绝可能
  写出缺失类型的点分隔名与 CLR `+` 名称。模板仍须完整、可赋值且不执行构造函数。
- 逻辑 Property 写入统一要求源码或受支持内建 schema 的类型证据，不能证明时返回
  `property.schema_unverified`。raw asset API 继续保留结构编辑与诊断契约。
- 新增原生模板要求必需字段、支持的序列化版本、合法组件/Transform 所属关系，GameObject
  恰有一个 Transform；脚本模板还须符合 MonoBehaviour/ScriptableObject 的宿主角色。
- 对象列表插入/结构变化后重新生成类型 hints，提交响应与重新读取的 bool/float/string
  类型一致。单批新增、移动、子字段写入和 managed 别名重定向保持顺序。
- 所有 Property 请求共用同一编译路径。连续实体请求按阶段批量 lower/edit；独立实体
  标量可以跨文件交错收集后按文件合并。继承、类型创建和结构命令保留必要顺序边界。
- 源码声明使用共享不可变字段索引；类型搜索与真正使用的依赖分开。2,051 个脚本项目
  不再把无关脚本全部放入提交凭据，创建后的依赖可继续使用，无关脚本变更不会触发冲突。
- 项目 metadata 索引供 schema 和 Prefab 共用，候选提交不再重复初始化未使用的 schema。
  事务内缓存读取投影、类型和对象身份；没有跨请求复用未验证的磁盘快照。
- 标量批次按路径检查祖先/后代冲突，索引一次生成一次补丁；重复且独立的标量写入只在
  每条输入验证后合并。保留引用原子性、最终图验证、未知结果防重放和无变化字节保留。

汇总响应是新增的可选 API，不改变现有 Tree 和 batch 的完整响应：

```ts
const receipt = await locus.unity.property.backend("yaml").apply({
  writes,
  resultMode: "summary",
  profile: true,
});
// receipt: {ok,message,writesApplied,transactionId,assets:[{path,revision,dependencies}],profile?}
```

summary 保留相同的类型、顺序和事务校验，省去逐请求 Tree/beforeSnapshot 的构建及传输。
每个资产返回新的 revision/dependencies，供后续批次使用。live 明确拒绝 summary。
profile 增加 `materializedCompilePasses` 与 `readProjections`。

### 性能证据

完全重放 review 的 **15,940 字节、128 次同字段写入、携带依赖** 的相同 fixture 与请求形状：

| 模式 | 服务调用耗时 |
| --- | ---: |
| 修复前完整响应 | 6,726.67 ms |
| 修复后完整响应 | 210.02 ms |
| 修复后汇总响应 | 181.27 ms |

完整响应约提升 32 倍；有效图重建从 129 次降为 2 次，实体编译一次。此组采用旧基线相同的
服务调用计时口径，不包含额外的完整响应 JSON 编码。输出为 `PROPERTY_BASELINE_REPLAY`。

另外新增全部字段具有源码类型证据的规模矩阵，**所有场景均确实改变文件字节**：

| 字段/请求 | 完整响应 | 汇总响应 | 完整/汇总响应字节 |
| --- | ---: | ---: | ---: |
| 1 字段、1,000 请求 | 297 ms | 101 ms | 3,994,338 / 759 |
| 1,000 字段、128 次同字段请求 | 242 ms | 178 ms | 511,582 / 757 |
| 1,000 字段、1,000 个不同字段请求 | 690 ms | 219 ms | 4,020,803 / 746 |
| 1,000 字段、10,000 请求 | 2,976 ms | 743 ms | 40,224,935 / 750 |
| 10,000 字段、1,000 个不同字段请求 | 2,281 ms | 1,503 ms | 4,022,816 / 750 |

矩阵包含一次完整响应 JSON 编码，汇总模式均无 Tree 构建和读取投影。两轮万请求汇总样本
为 720–743 ms。旧 Prefab 1,000 请求基准也通过，含 129 源码文件的样本约 392 ms。
以上均为本机 debug、离线单次采样；有前置 read，源码解析缓存已热，不能等同于发行版、
冷项目或 Unity 在线导入的延迟。metadata 仍在每请求捕获，避免无版本全局缓存带来错误。

规模与原基线重放日志：`.tmp/property-fix-scale-verified.log`；补充轮次：
`.tmp/property-fix-scale-final.log`；Prefab 基准：`.tmp/property-fix-prefab-perf.log`。
测试断言编译/重建次数、最终值和实际事务结果，不使用依赖机器速度的耗时阈值。

### 验证结果

| 范围 | 结果 |
| --- | --- |
| 纯 Rust core | 76 通过 |
| 资产服务/schema/storage | 44 通过，4 个 ignored 用例均已显式运行 |
| Agent Property Tree | 42 通过 |
| 前端 8 个测试文件 | 86 通过 |
| `bun run typecheck:test` | 通过 |
| 原有 Unity 2022 / 6 套件 | 各 55 通过 |
| 新生产服务合成测试 Unity 2022 / 6 | 两版本均通过，6 项原生断言全部为 true |

新合成测试验证：嵌套 managed 类型、循环身份、嵌套列表 bool/float/string、与 live 数据一致、
未验证类型写入被拒绝且旧值不变，以及真实 GameObject/Transform 模板的新增父子拓扑。
还包含 500 个累计标量请求；服务响应及前后 Editor 日志保留在每次独立 fixture 目录中。

- 修复前合成 fixture：`<runtime-root>/property-service-<run-id>`。
- Unity 2022 最终合成 fixture：`<runtime-root>/property-service-<run-id>`。
- Unity 6 最终合成 fixture：`<runtime-root>/property-service-<run-id>`。
- 原有 55 项 fixture：`<runtime-root>/property-review-<run-id>`（Unity 6）、
  `<runtime-root>/property-review-<run-id>`（Unity 2022）。
- 日志：`.tmp/property-fix-final-service.log`、`.tmp/property-fix-agent-tree.log`、
  `.tmp/property-fix-core-verified.log`、`.tmp/property-fix-native2022-verified.log`、
  `.tmp/property-fix-native6-verified.log`、`.tmp/property-fix-vitest-final.log`、
  `.tmp/property-fix-typecheck-final.log`。

运行方式见 `scripts/tests/unity-property/README.md`。本轮解决已复现的问题和已测量的瓶颈，
上述 Review 修复阶段没有扩展此前明确拒绝的 Prefab 结构覆盖、继承 managed 创建或自定义回调业务语义，也未承诺
任意规模资产的固定延迟。未新增 UI 元素，沿用原有编辑控件和交互。

后续复杂 Prefab 扩展已按失败样例推进：继承数组、空自定义列表、已有 added/removed 拓扑、
managed 字段、数组 Revert/Apply 和 SceneRoots 投影。测试结果及边界见
[复杂 Prefab 覆盖扩展](./property-tree-prefab-coverage-2026-09-18.md)。回调业务逻辑继续留给 View。
