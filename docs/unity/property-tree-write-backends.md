# Property Tree 写入后端与批次

## 当前链路

前端的 `PropertyTree` 负责 Snapshot、Drawer、节点状态和布局，本身不决定存储方式。
`UnityBoundPropertyTree` 通过 adapter 调用读写服务，现有原生 Inspector 使用
`unity_serialized_property_*` IPC，最终在 Unity 主线程执行 `SerializedObject`。
原有 `writeMode: commit/preview` 表示提交和拖动预览，不表示 YAML / Unity 后端。

仓库另有 `unity_assets` API：Rust 的 `unity_asset_core` 负责 YAML 的保留格式编辑、
精确整数、数组、引用验证；`storage` 负责版本检查、事务日志、文件替换和恢复。
Unity 在线时，`disk_apply` 在主线程统一检查 dirty 状态、替换文件并导入；
不通过 `SerializedProperty` setter 逐字段修改。Unity 关闭时可离线写入。

## View API

模式属于 API 实例，由编写 View 的 Agent 选择一次。控件不新增模式参数或切换器。

```ts
import { locus } from "@locus/frontend";

const yaml = locus.unity.property.backend("yaml");
const live = locus.unity.property.backend("live");
// 原有 locus.unity.property 默认保持 live。

const target = { kind: "asset", path: "Assets/Data.asset", targetFileId: "11400000" };
const tree = await yaml.readTree(target);
const before = await yaml.read({ target });
const batch = yaml.batch();
batch.enqueue({ target: { ...before.target, propertyPath: "amount" },
  value: 42, expectedRevision: before.revision, expectedDependencies: before.dependencies });
batch.enqueue({ target: { ...before.target, propertyPath: "note" },
  value: "updated", expectedRevision: before.revision, expectedDependencies: before.dependencies });
await batch.flush();
await tree.refresh();
```

`backend()` 返回独立实例，不会改变已经创建的 Tree 或批次。YAML Tree 的默认绘制
继续复用 `UnitySerializedPropertyTree`、`UnityPropertyEditor` 和现有 Drawer，
数字拖动仅在控件内预览，提交才发生写入。API 未新增界面标签、卡片或状态装饰。

## 性能与交互

推荐保留本地草稿，累积请求，结束交互或显式 Apply 时 flush：

- 入队不执行 IO，也不通知 Unity；请求值在入队时复制，调用方后续修改不影响批次。
- Property API 在 flush 时只发一个 `apply_properties` IPC。Rust 捕获各文件的规范化
  快照，按更新中的引用图解析后续逻辑路径，再编译为一个 `apply_batch` 事务；
  前端不读取原始资产来执行转换，数组操作、父子字段和重复赋值保留原顺序。
- Unity 在线时只进行一次事务往返，每个变化资产集中导入。导入、OnValidate 和
  Scene/Prefab Stage 重载仍有 Unity 主线程成本；本次不宣称完全消除该成本。
- 大规模操作如果已有原始字段路径，可直接使用 `locus.assets.backend("yaml").batch()`，
  `enqueue(path, operations, {expected_revision})` 后 `flush()`，避免 Property Tree 转换读取。
- 单批上限为 256 个文件、10,000 个操作；更大的作业按批执行。

不需要逐条 UI 快照的批量脚本可使用汇总响应：

```ts
const receipt = await yaml.apply({ writes, resultMode: "summary", profile: true });
// receipt.writesApplied、transactionId、assets[{path,revision,dependencies}]
// 继续编辑时按资产使用 receipt.assets 中的新版本与依赖。
```

汇总模式保留相同的类型、顺序、版本和事务校验，只省略逐请求 Tree/beforeSnapshot 的生成
和传输；默认 `full` 响应与现有 Tree/batch 契约保持一致。`live` 不接受 summary。

“每次写盘、延迟通知”会继续支付文件替换成本，且 Unity 自动刷新可能提前读取
中间结果。当前采用显式批次，不跨异步交互长期持有 `StartAssetEditing`，也不依赖
关闭用户的自动刷新来维持正确性。

## 正确性边界

- 逻辑 Property 写入要求源码或受支持的内建 schema 能证明类型；partial、条件编译、
  缺失类型等不能证明的写入返回 `property.schema_unverified`，不以磁盘数值形状代替类型。
  底层 raw asset API 仍是结构编辑接口并返回诊断，不能视为同等类型保证。
- 所有逻辑写入共用同一编译路径，传递依赖不会切换到逐条解析。实体批次按顺序编译一次；
  创建、结构模板和 Prefab 命令是阶段边界。插入数组后重新生成最终类型提示。
- managed 模板使用 Unity 的准确 type identity，嵌套类为 `Outer/Inner`；不接受 CLR 的
  `Outer+Inner` 或源码检索用的 `Outer.Inner`。原生实体模板须包含必需字段和合法归属。
- YAML 使用持久化路径与精确 fileID，不解析运行时 selection/globalObjectId。
  支持文本 Prefab 的递归有效字段、标量覆盖和 Revert；多对象文件通过 `discover()`
  获取 target。继承写入必须传 read 返回的 `dependencies` 为 `expectedDependencies`。
- 每个文件只接受一个读版本；提交前仍由生产事务层检查磁盘版本和 Unity dirty 状态。
  每棵 Tree 单独保留版本，不能用另一棵 Tree 的新读取掩盖旧数据。
- 并发 flush 复用同一个 Promise；在途不能追加或清空。错误后保留请求并禁止自动重放，
  先核对结果，再清空并从新读取重建，避免重复执行数组操作。
- YAML 默认 Tree 复用 Rust 的 Agent 字段投影，不提供所有 Editor 属性或 Enum 选项。
  Curve/Gradient 支持标准 API payload，默认 Tree 保留只读预览，避免浮动编辑器走 live。
  引用本身在默认 Tree 中只读，已有托管引用的子字段
  可按逻辑路径编辑；API 也可直接提交序列化引用值。
- 数组新增和扩容必须显式提供 value；也可整体设置数组。Editor 的隐式构造、
  restore 命令不在 YAML 适配范围内。managed 创建使用显式 `createManaged` 模板命令，
  不仿真构造函数。跨层 Apply 和实体结构模板见共享语义层方案中的 P2–P4。
- `live` 的 `saved` 沿用现有 Property Tree 提交语义，资产可能仍等待 Unity 保存；
  YAML 的成功结果表示文件事务已持久化。两者不冒充完全相同的 Undo/持久化语义。

## 验证

`unityPropertyApi.test.ts` 验证模式隔离、Tree 继承、分页、精确 ID、本地预览、版本冲突、
取消、批次顺序、请求复制、并发 flush、失败重放保护，以及批内新增/删除数组目标。

`locus:test:property` 在独立 Unity 项目中运行真实生产代码。新增四组对照：

| 用例 | 验证 |
| --- | --- |
| YAML01 | Rust YAML 编辑与 Property Tree 主线程写入的整数、浮点、文本、布尔、向量成员和空引用一致 |
| YAML02 | 数组移动、删除、缩小、插入后的值与顺序一致；共享和循环引用未受影响 |
| YAML03 | 准备阶段不改变资产，合并编辑后每个文件只导入一次 |
| YAML04 | Unity 未保存修改及保存后的新版本均能阻止过期 YAML 覆盖 |
| YAML05 | 通过循环/别名逻辑路径编辑托管对象，值与 Unity API 一致且 rid 不变 |
| YAML06 | 逻辑数组插入、写入新元素、移动后与 Unity API 一致 |
| YAML07 | 未开放的类型创建在写盘前明确拒绝 |
| YAML08–09 | 显式模板创建/替换、循环和旧别名保存，与 live 初始化后结果一致 |
| YAML10–11 | 嵌套 Variant Override/Revert/Apply 与重复实例隔离 |
| YAML12–13 | 加权曲线、渐变标准 payload 差分；组件新增/删除与归属拓扑 |
| YAML14–16 | 1,000 请求单次导入；来源 dirty/stale；Variant→Scene 传播 |

Rust 测试 driver 直接包含生产编辑内核；Unity 使用生产 `disk_apply` 和
`WritePropertyTree`。完整 SDK → Rust 存储事务的跨层验证继续由现有 `asset-api` suite 承担。

后续阶段与验收矩阵见 [共享语义层方案](./property-tree-shared-semantics-plan.md)。

`unity_assets::property::regression_tests::review_unity_service_roundtrip` 另行贯通真实 Unity
生成资产、Editor 退出、生产 Property API/schema/事务写入、Unity 重载及 SerializedObject
差分，避免仅测试内核而跳过源码校验。命令见 `scripts/tests/unity-property/README.md`。

复杂 Prefab 的后续能力（继承数组、空自定义列表、已有 added/removed 拓扑、managed 字段、
数组 Revert/Apply）及明确限制见 [复杂 Prefab 覆盖扩展](./property-tree-prefab-coverage-2026-09-18.md)。
