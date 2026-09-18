# Property Tree 实现 Review（2026-09-18）

后续状态：下列 6 项发现已建立失败回归并修复；最终验证及性能数据见
[Review 修复与规模验证](./property-tree-shared-semantics-plan.md#review-修复与规模验证2026-09-18)。
本文保留发现时的代码位置和证据，作为修复前基线。

本次检查 API backend 选择、共享寻址、源码类型校验、Prefab 覆盖、模板创建、事务和性能。
结论：基础架构可以继续使用，但当前不能按“任意复杂序列化数据均可安全编辑且适合大规模批处理”验收。
已复现 6 项问题，其中两项在 Unity 6000.5.8f1 重载后产生与服务成功响应不同的结果。
本次未修改生产实现；仅保存 review 文档、隔离探针与日志。

## 发现的问题

### 1. P1：嵌套 managed 类型身份校验允许写出 Unity 无法解析的类型

位置：`src-tauri/src/unity_assets/schema.rs:84–88`。

源码中的 `Config.Node` 被记录为点分隔名称。`find_type` 会将 `/` 和 `+` 规范化成 `.`，
但创建校验随后用原始 `full` 与声明名直接比较；输出也没有转换成 Unity 的 wire identity。
服务探针结果：

- `class: Config/Node`：拒绝，`property.creation_schema_unverified`。
- `class: Config+Node`：拒绝。
- `class: Config.Node`：返回 `ok=true,saved=true`，该字符串原样进入 YAML registry。

Unity 实测自己保存的名称为 `Config/Node`；将其改成服务接受的 `Config.Node` 后，导入日志
出现 Missing types，`node == null`。已有 nested type 的读取/子字段编辑和创建新 nested type
必须分别评价，前者可寻址不代表后者可正确创建。

建议：区分源码全名、CLR 名称和 Unity registry 的 class/ns/asm；从已验证声明生成唯一合法
wire identity，禁止把源码检索名称直接写盘；增加嵌套两层、同名 namespace/type 和继承模板的重载测试。

### 2. P1：未知 schema 的诊断在 Property API 丢失，类型不等价写入被报告为成功

位置：`src-tauri/src/unity_assets/property/advanced.rs:755`；
普通路径的 `property.rs:read_result` 同样没有传出底层 schema diagnostics。

Schema 层对 partial、条件编译、缺失源码等情况有意返回 warning，而不是失败；高级调用方
丢弃返回的 `Vec<Diagnostic>`。这时 span editor 的数值形状校验允许整数变为小数。

复现：`public partial class Config` 中 `public int amount` 的 YAML 初值为 7，写入 2.5。
Property API 返回 `saved=true,value=2.5`，没有 diagnostics；Unity 实际导入为整数 2。
底层 raw API 的“结构性编辑＋显式诊断”契约不能直接当作 Property API 的类型等价保证。

建议：至少贯通诊断及校验等级；需要保证语义等价的写入在类型证据不足时拒绝，或明确提供
独立的宽松策略。覆盖 partial、`#if`、DLL 类型、包缓存类型和未知字段的拒绝/诊断测试。

### 3. P1：实体对象模板缺少原生对象的完整性校验

位置：`src-tauri/src/unity_assets/schema.rs:51–66`、
`src-tauri/src/unity_asset_core/validation.rs:64–80`。

创建校验仅要求 MonoBehaviour 的脚本字段完整；GameObject/Transform/RectTransform 只检查
实际提供的字段。所有权校验也主要在链接字段存在时执行。

复现：`editObjects.add` 添加 `{classId:"1",rootType:"GameObject",data:{m_Name:"NoTransform"}}`
成功持久化；没有 `m_Component`，也没有 Transform。当前图校验能发现若干错误链接，不能证明
对象模板本身构成合法的 Unity 实体。本项已验证服务会落盘，未对该不完整对象进行 Unity 导入。

建议：对新增原生对象要求版本化完整模板和必需字段；验证 GameObject 必须拥有合法 Transform、
组件类型及归属，拒绝通过省略链接字段绕过校验。增删结构应增加非法模板的反例测试。

### 4. P2：自定义对象列表扩容后，提交响应与重新读取的字段类型不同

位置：`src-tauri/src/unity_assets/storage.rs:148–151`、
`src-tauri/src/unity_assets/property/advanced.rs:759–760`。

两条路径都复用编辑前按索引生成的 hints。已有 primitive packed array 的类型传播修复，
没有覆盖 `List<Item>` 内新增元素的子字段。

复现：`Item { bool flag; string label; }` 列表新增 `{flag:false,label:"007"}`。
普通路径和高级路径的提交响应均为 `flag: Integer/0`，重新读取为 `Boolean/false`。
YAML 数据本身正确，但 View 会获得错误的控件类型、显示和后续编辑值。

建议：最终候选图重新生成受结构变化影响的类型证据，并同步刷新响应 Tree；测试至少覆盖
新增/移动嵌套 class、bool/float 子字段、空列表和多层列表包装对象。

### 5. P2：携带依赖的实体批次绕过批量编译，退化成每请求全量渲染/解析

位置：`src-tauri/src/unity_assets/property/advanced.rs:751–769`；
入口选择见同文件 `required`。

非空 `expectedDependencies` 会选择高级路径。此路径只合并连续继承标量覆盖；实体写入每次
执行 render、semantic 构造、edit、AuthoringAsset 重建、rebase，然后清空有效图缓存。
正常通过 discover 或创建返回的依赖继续写入，也会触发这一行为。

本次同机 debug、离线完整服务探针：

| 工作负载 | 普通路径 | 携带依赖的高级路径 |
| --- | ---: | ---: |
| 小资产，同一字段 128 次写入 | 71 ms | 177 ms |
| 小资产，同一字段 1,000 次写入 | 277 ms | 1,110 ms |
| 15,940 字节、约 1,000 字段资产，同一字段 128 次写入 | 173 ms | 6,727 ms |

最后一行另一轮为 170/6,867 ms。高级路径分别构建有效图 129/1,001 次；约 16 KB 场景
6.64 秒花在 prepare，commit 约 45 ms。因此当前瓶颈首先是候选构造，而非磁盘或 Unity 导入。
这是用于暴露路径差异的采样，不是 release 性能承诺。

建议：连续实体写入也按文件编译为一次 `lower_writes + edit_with_hints`，保留结构、跨文件
依赖和 Apply/Revert 的必要顺序边界。不能通过删除 `expectedDependencies` 换取性能。

### 6. P2：创建 managed 对象捕获全项目源码，使合法后续请求超过依赖上限

位置：`src-tauri/src/unity_assets/schema.rs:74–75`，
依赖上限见 `src-tauri/src/unity_assets/property/advanced.rs:585–587`。

创建为搜索类型加载所有源文件；`load_source` 同时将每个源文件及 metadata 标成捕获依赖。
这些文件最终全部写入响应 dependencies。

复现：1 个实际类型脚本＋2,050 个无关脚本及 metadata；创建成功，响应包含 4,103 条依赖。
直接带上这个响应的 revision/dependencies 修改 `node.amount`，被拒绝为
`property.dependency_limit`。也会使无关源码变动导致冲突，并扩大每次 dirty/hash 检查和传输。

建议：类型检索索引与真正参与语义判定的依赖分开；只捕获实际声明、基类、字段类型及程序集
来源链。保证 API 返回的依赖凭据可以被后续请求接受。上调上限不能解决全项目耦合。

## 序列化数据覆盖面

| 数据/操作 | 当前结论 |
| --- | --- |
| 已有 inline `[Serializable]` class/struct 多层嵌套 | 可按实际字段路径修改；类型证据不足有发现 2 的风险 |
| `List<自定义类>`、数组中对象、对象内数组 | 实体资产可改现有字段、插入/删除/移动/扩容；扩容要求显式值，提交投影有发现 4 |
| 已有 SerializeReference 多态/共享/循环图 | 实体宿主内按 rid 寻址，可修改子字段；别名仍共享，不会无限展开 |
| 新建/替换 SerializeReference | 须完整显式模板、类型可赋值、源码可证明；不是调用构造函数；nested type 有发现 1，大项目有发现 6 |
| Prefab 普通嵌套值的叶字段、已有数组元素 | 在来源图可以解析的前提下支持标量 Override/Revert，标量跨层 Apply |
| Prefab 继承的 managed 子字段 | 不具有与实体路径相同的逻辑解引用能力；`value_at` 仅遍历 JSON 键/索引，`node.amount` 不能穿过 `{rid}` |
| Prefab 数组 size/结构覆盖、继承 managed 创建、AddedComponent/AddedGameObject | 当前拒绝；已存在不支持的覆盖布局也可能阻止整个有效树读取，并非仅禁用相应写按钮 |
| 实体 GameObject/Transform/MonoBehaviour 增删 | 有模板入口，但需先修发现 3，不能视作完整 Inspector 拓扑语义 |
| 曲线/渐变 | 支持已识别版本的标准 API payload；非有限切线拒绝，YAML Tree 浮动编辑器仍只读 |
| ISerializationCallbackReceiver 自定义字典/图 | 只能改它真实保存的 backing fields；不能自动理解回调中的业务约束和重建逻辑 |
| Odin/自定义二进制、JSON 字符串、压缩载荷 | 没有相应语义 codec；最多把实际载荷字段作为整体修改，不能直接寻址内部逻辑对象 |
| 构造函数、CustomEditor、OnValidate、原生 Undo | 不属于离线语义等价保证；导入仍可能执行 Unity 回调 |

例如 `config.groups.Array.data[3].settings.threshold` 是可表达的字段路径；
若某个对象用两条列表保存字典，可以在一个批次中编辑 keys/values，但现有层不验证等长、
键唯一和自定义回调不变量。应增加显式业务 codec/validator，而不是从 YAML 形状推断业务语义。

Unity 原生不直接序列化 Dictionary、多维数组及裸嵌套容器；通常通过包装类或回调编码。
参见 [Unity 序列化规则](https://docs.unity3d.com/6000.0/Documentation/Manual/script-serialization-rules.html)
和 [ISerializationCallbackReceiver](https://docs.unity3d.com/6000.0/Documentation/ScriptReference/ISerializationCallbackReceiver.html)。

## 性能与事务评价

可以保留的部分：API 实例选 backend、请求积累后 flush、精确 fileID/rid/大整数、
immutable 候选、revision/CAS、失败回滚和未知结果不自动重放。YAML 写数据不依赖
SerializedProperty setter；在线 import/重载仍需要 Unity。

已有 Prefab 服务优化有效：8 实例、33 字段、1,000 请求的历史 debug 样本约 626 ms，
含 129 个源码文件的样本约 658 ms，仅构建两次有效图。但该结果不能代表所有 materialized、
createManaged、数组结构或跨文件场景，发现 5 已给出反例。

另外，每请求的项目 metadata 扫描仍存在；高级加载与 storage 还会分别初始化 schema。
响应按每条原始写请求输出最终 snapshot 和 beforeSnapshot，重复同一字段的 1,000 次请求仍有
约 150–190 ms 的响应投影成本。后续可增加紧凑结果模式，不应把所有批处理都绑定完整 UI 快照。

尚无足够证据证明 10–100 MB 资产、万次不同字段写入、数百文件、深层 Prefab 图的内存峰值和
p95 延迟。应分别测普通/高级路径、重复/不同字段、冷/热索引、结构操作、离线/在线及失败回滚。
多文件事务具备回滚能力，不保证外部观察者看到多文件同时原子变化。

## 证据与验证

- 本次纯 Rust 核心回归：74 通过，`.tmp/property-review-core.log`。
- 本次 API/Tree/Assets/Binding Vitest：4 文件、36 通过，`.tmp/property-review-vitest.log`。
- 服务探针调用生产 `unity_assets::execute`，未模拟编译/事务层；源码保存于
  `.tmp/property-review-probe.rs`，输出 `.tmp/property-review-probe-complete.log`。
- fixture：`.tmp/property-review-probe-86552`。性能补充轮次：
  `.tmp/property-review-probe-final.log`。
- 本次 Unity 6 原生反例结果：
  `<runtime-root>/property-review-native-<run-id>/review-native-results.json`；
  `actualClass=Config/Node, validNode=true, dottedNode=false, fractionalInteger=2`。
  原生探针源码及启动脚本在 `.tmp/property-review-native/`。
- 历史两版本各 55 通过的结果已复核，其中 37 项为原有 live Inspector 检查，18 项为 YAML
  增量用例，并非 55 项全部都是 YAML/live 差分。轻量 YAML driver 调用 core，不包含生产
  ProjectSchema/advanced service；这也是类型创建和服务性能问题未被那些测试发现的原因。
  本次没有重跑完整双版本 55 项套件，不能把历史结果当作新增反例已通过。
- Review 时相关源码 hash：`.tmp/property-review-source-hashes.json`。

优先修类型身份、未验证写入和原生模板完整性；再补最终类型投影、实际依赖闭包和实体批量编译。
新增一致性用例应贯通生产 Property API → schema/compiler → transaction → Unity 重载，
同时保留纯内核快速回归。
