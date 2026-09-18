# 统一 Unity 资产 API

Python、TypeScript View 和 merge plan 共用资产快照、精确对象身份、字段路径与修改操作。后端在创建上下文时绑定，后续调用保持一致。

```python
assets = locus.assets.backend("yaml", workspace_ref=workspace_ref)  # 或 live
snapshot = await assets.read("Assets/Config.asset")
operations = [{"op": "set", "object_id": "11400000",
               "property_path": "/MonoBehaviour/amount", "value": 731}]
await assets.preview("Assets/Config.asset", operations,
                     expected_revision=snapshot["revision"])
await assets.apply("Assets/Config.asset", operations,
                   expected_revision=snapshot["revision"])
```

View 内从 `@locus/frontend` 导入 `locus`，使用 `locus.assets.backend("yaml" | "live")`，同样调用 `read/discover/preview/apply`；revision 参数放在第三个 options 对象中。`preview_batch/apply_batch` 接收 `{path, expected_revision, operations}` 列表。

## 执行路径

| 入口或后端 | 读取与修改位置 | 持久化 |
| --- | --- | --- |
| Python SDK | HTTP RPC `assets.*`，绑定 WorkspaceRef | 由公共 Rust 服务执行 |
| TypeScript View | IPC `unity_assets_execute`，绑定 View 的 checkout 与生命周期 | 由公共 Rust 服务执行 |
| YAML，Editor 未运行 | Rust 解析磁盘快照、校验并生成结果字节 | 文件事务；不启动 Unity |
| YAML，Editor 已连接 | 修改仍在 Rust 完成；一次 Editor 主线程批量校验、替换和导入 | 拒绝 dirty 目标，保留无关 dirty 状态 |
| live | 读取 Editor 当前对象；使用 SerializedObject 执行操作 | 保存目标资产或场景 |
| merge plan | 同一核心编辑冻结的当前合并结果 | 保存 plan 选择；外层 `plan.apply()` 才写项目文件 |

`execute_typescript` 保留修改工具的权限语义，但等待前端回调期间不持有外层工作区写锁；资产端点自行获取事务范围，避免回调重新进入同一写锁而死锁。后端选择不会自动切换，也不会自动启动 Editor。

## 公共契约

- Snapshot 包含 `revision`、`objects`、`diagnostics`。对象和字段使用字符串 `object_id` 与根类型完整的 RFC 6901 `property_path`，例如 `/MonoBehaviour/numbers/0`。managed reference 使用返回的 `@rid=...` 路径。
- 操作包括 `set`、`array_insert`、`array_remove`、`array_move`、`array_resize`。扩容需要显式 fill `value`；操作按给定顺序执行。
- fileID/rid 使用十进制字符串。超出 JavaScript 精确整数范围的数值使用 `{kind:"int64",value:"..."}`；无符号 64 位数据使用 `{kind:"uint64",value:"..."}`。传递读取到的包装对象可保留完整精度。
- 有符号数值使用 Python 的 `locus.asset_integer` / `assets.integer`，或 TypeScript 的 `assetInteger` / `assets.integer`。无符号数值使用 Python 的 `locus.asset_unsigned_integer` / `assets.unsigned_integer`，或 TypeScript 的 `assetUnsignedInteger` / `assets.unsignedInteger`。全部对象、fileID/rid 引用身份仍受有符号 64 位范围约束。
- 每次 apply 都要求输入快照的 `expected_revision`。preview 输出描述修改后的快照；应用时仍使用原输入 revision。过期 revision 在修改前失败。
- preview 返回 `applied=false,persisted=false`；磁盘 apply 成功返回二者均为 `true`。merge 的 `plan.assets.apply(...,persist="plan")` 返回 `applied=true,persisted=false`，表示仅保存合并选择。
- 批处理最多 256 个资产、10,000 项操作；数组上限为 1,000,000 个元素。同一批不能重复指定同一个路径。更大任务需要显式划分事务。

## 支持范围与恢复

`capabilities()` 返回实际后端能力。当前入口接受 `Assets/` 下的 Unity 文本序列化资产：`.asset`、`.prefab`、`.unity`、`.mat`、`.anim`、`.controller`、`.overridecontroller`、`.playable`、`.mask`。文件扩展名通过检查后，具体对象、字段类型和操作仍需校验。

共同操作域覆盖已支持的序列化标量、结构、对象引用、现有 managed-reference 数据及数组；packed primitive arrays 依赖类型信息解码。现有 C# schema 可提供类型与范围校验，无法确定的 schema 必须保留诊断。静态检查不能替代 Unity 导入与实际行为验证。

只读 package cache（例如 `Library/PackageCache`）中的 C# schema 与 GUID 目前尚未完整解析。依赖这些包缓存的字段类型和引用，不属于当前已验证的完整解析范围。

共同操作域明确不支持纯 Prefab Variant，即使请求没有改变拓扑。Prefab 继承后的有效字段投影、override 创建、nested Prefab 拓扑修改、任意 C# 类型实例化、运行时临时对象以及二进制导入资产，同样不在支持范围内。完整支持范围以 capabilities、返回诊断和实际验收证据为准。

批处理先验证全部输入，再依次替换文件，失败时执行回滚。YAML 的前后状态与 journal 位于项目 `Library/Locus/AssetApi/<transaction-id>/`；进程中断后可在 Editor 关闭时调用 `assets.recover(transaction_id)`。恢复只接受 journal 记录的前后状态，遇到后续外部修改会停止；已提交事务不能作为撤销命令回退。live 调用中断后应重新读取状态，避免重放数组操作。

在线文件事务在 Editor 主线程释放缓存文件句柄，再执行原子替换和目标导入；Windows 仅对确认的共享占用进行有限重试，每次重试重新验证原内容。缓存句柄释放用途见 [Unity AssetDatabase API](https://docs.unity3d.com/cn/2022.3/ScriptReference/AssetDatabase.html)。只修改现有序列化资产的 merge 计划复用同一事务；含增删文件、脚本或 metadata 的通用 merge 仍使用原有落盘流程和目标脏状态预检查。merge 的磁盘恢复统一要求 Editor 关闭。

live 在全批校验后，逐资产应用暂存修改、保存并记录实际字节，避免提前把后续资产标脏。整个 Asset API 调用通过有作用域的 `OnWillSaveAssets` 过滤器限制 Unity 的隐式保存；read/preview 和 YAML 导入不允许额外保存，live 保存仅允许当前目标。这样可以阻止保存场景时顺带保存无关脏资产；作用域结束后恢复正常用户保存。过滤器使用 Unity 提供的路径子集机制，见 [OnWillSaveAssets](https://docs.unity3d.com/2022.3/Documentation/ScriptReference/AssetModificationProcessor.OnWillSaveAssets.html)，并在下述 Unity 2022.3 实测中验证。

capabilities 明确返回 `atomicity="rollback_on_failure"`、`multi_file_atomic_visibility=false`；`crash_recovery` 和 `durability` 分别说明后端是否提供进程中断恢复及其持久化方式。

## 实际验收与性能记录

在仓库根目录执行：

```powershell
bun run locus:test:unity -- --project C:\Projects\UnitySample --suite asset-api --install-plugin --connect-timeout-ms 120000 --timeout-ms 1200000
```

共享开发实例占用 `src-tauri/target/debug/locus.exe` 时，单独构建测试入口，继续复用同一个 Cargo 增量缓存，无需停止共享 Locus 实例：

```powershell
cargo build --manifest-path src-tauri/Cargo.toml --no-default-features --example unity_test_driver
bun run locus:test:unity -- --driver-binary .\src-tauri\target\debug\examples\unity_test_driver.exe --project C:\Projects\UnitySample --suite asset-api --install-plugin --connect-timeout-ms 120000 --timeout-ms 1200000
```

`--driver-binary` 直接运行指定 EXE，保留隔离 runtime、Unity pipe namespace、日志、结果解析与 `compile-server:ensure`，跳过 Tauri 构建和开发服务器启动。上述 debug 构建的 View 验收需要 `bun run dev` 提供前端；已有开发服务可直接复用。需要避免占用测试入口的输出文件时，先把 EXE 复制到本次独立目录，再将该绝对路径传入此参数。不要为此停止其他 Agent 或共享开发实例。

该 suite 单独运行，入口检查目标 Editor 未运行，然后管理本次测试所需的启动与关闭。它在 `Assets/LocusAssetApiTests/run-<id>/` 创建专用测试资产，分别验证离线 YAML、Editor 协调下的 YAML、live 修改，并比较持久化字段。这个初始状态要求用于覆盖离线阶段；公共 YAML API 同时支持已有 Editor 的协调修改。

测试内容包括 ScriptableObject、Prefab、Scene、Material、共享及循环 managed reference，以及 13 种 primitive arrays：byte、sbyte、short、ushort、int、uint、long、ulong、float、double、bool、char、enum。另有 dirty 状态保护、错误对象 ID、批量失败和过期 revision 检查，以及原生 View 中真实 TypeScript 读写。

证据目录为 `Library/Locus/AssetApiAcceptance/run-<id>/`。主脚本输出 `LOCUS_ASSET_API_ACCEPTANCE:`，记录 `asset_count`、`operations`、`arrays_checked`、`read_all_ms`、`preview_batch_ms`、`apply_batch_ms`、`total_ms`。初次读取耗时包含被修改资产和共享引用资产的 SDK 读取；apply 耗时仅包括成功批次；total 包含负例、读回校验，不包含 Editor 启动。性能比较必须使用相同语料及相同机器，不直接推广为任意项目的吞吐量。

2026-09-07 在 `C:\Projects\UnitySample`、Unity **2022.3.47f1** 上完成完整验收，driver 返回 `finished.ok=true`。资产目录为 `Assets/LocusAssetApiTests/run-969cabcad55541938a8b52fc55cec388/`；[完整结果](../../artifacts/asset-api-acceptance/verified-result.json) 已保存在仓库测试产物中，项目原始证据位于 `Library/Locus/AssetApiAcceptance/run-969cabcad55541938a8b52fc55cec388/`。

- 三种运行方式各修改 **36 个资产、662 个操作**，共 108 个修改副本；36 组对比的 **1,775 个测试字段一致**。比较完整自定义字段和 Material 字段（排除资产名称），不以整个 Scene 原始 YAML 字节完全相同作为标准。
- **13 类数组**包含整数组提交和插入、删除、移动、扩缩容，实际往返 `uint64` 最大值；成功写入后全部目标及共享引用资产的 `.meta` 保持不变。
- Unity 重新导入并读取 **96 个 ScriptableObject**，验证共享引用、循环引用身份与修改值。
- **9 项安全检查通过**：live 读取未保存状态、YAML 拒绝 dirty 目标、修改干净资产保留无关 dirty，以及 live 保存资产、Scene、Prefab 后无关资产的内存值、dirty 和磁盘内容仍保持原样；错误对象 ID 不回退到主资产。
- 原生 View 实际调用 `locus.assets.backend("yaml" | "live")` 完成预览、保存、精确整数读回、过期 revision 拒绝和恢复测试。
- 相关 Rust 回归 **77 项通过**（核心 47、资产服务 19、merge 10、前端启动分类 1），Python **49 项通过**，相关 Vitest 与 `bun run typecheck:test` 通过；C# 引用编译和独立原子文件/保存作用域测试通过。

| 执行方式 | 首轮读取 37 个资产 | 预览 36 个资产 | 成功写入 36 个资产 | 整个 Python 验收 |
| --- | ---: | ---: | ---: | ---: |
| YAML，Editor 关闭 | 10.838 秒 | 0.794 秒 | **0.961 秒** | 34.853 秒 |
| YAML，Editor 已连接 | 11.284 秒 | 0.817 秒 | **2.334 秒** | 37.682 秒 |
| live | 1.097 秒 | 0.675 秒 | **2.197 秒** | 7.648 秒 |

这组数据表明 Rust 批量改写已能在不启动 Editor 时完成，也能复用在线协调路径；逐文件读取时重复构建源码/GUID 索引仍有明显成本，本次结果不支持“YAML 整个流程总是更快”的结论。上文列出的 Prefab 继承、包缓存解析等边界仍然存在，未被这批用例覆盖的 Unity 类型不能据此视为已保证双后端等价。
