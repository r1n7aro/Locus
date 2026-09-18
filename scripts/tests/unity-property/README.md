Unity Property 回归用例使用独立临时项目，复制仓库当前 Unity 插件及本目录的 fixture，不连接已打开的项目。覆盖 SerializeReference 的类型、ID、共享/循环/隐藏字段，数组完整撤销和分页，数值边界、结构化值、ObjectReference，嵌套 Prefab / Variant、Override、Static flags 以及原生 Undo/Redo。

将 `UNITY_EDITOR` 环境变量设为要测试的 Unity Editor 可执行文件路径，在仓库根目录执行：

```powershell
bun run locus:test:property -- --unity-editor "$env:UNITY_EDITOR"
```

可用 `--output-root <dir>` 指定隔离项目根目录。每次运行生成独立子目录，打印 PID、日志和 `LOCUS_PROPERTY_TEST_JSON` 结果；任意功能用例失败返回非零退出码。性能用例输出采样，不使用易受机器负载影响的时间阈值。新检出仓库应先按常规开发流程准备其余 Unity bundle；本入口会重建 JSON bundle。

前端运行真实 Vue 控件和 View runtime 的单元测试：

```powershell
bun run test src/__tests__/unityPropertyEditingRuntime.test.ts src/__tests__/unityPropertyEditorsRuntime.test.ts
bun run typecheck:test
```

Rust 精确目标身份和恢复载荷的 IPC 测试：

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --lib property_restore_wire
```

`PropertyReviewRunner.cs` 的每个 Check 都有明确预期，结果保存在隔离目录的 `property-review-results.json`。通过项表示预期行为成立；原审查中“断言错误行为”的观察脚本只保留在历史 artifacts 中，不作为本回归套件的通过标准。

YAML / Unity API 一致性由 `PropertyYamlParity.cs` 覆盖。启动器会在共享的
`src-tauri/target` 下编译小型 Rust driver，直接包含生产 `unity_asset_core`，
使用真实 Unity 导出的资产生成候选 YAML，再通过生产 `disk_apply` 协议导入。
另一份资产通过生产 Property Tree 主线程写入；比较精确整数、浮点、文本、
布尔、向量成员、空引用、数组顺序和未修改的共享/循环引用。同时验证批次前
不改动资产、每个文件只导入一次，以及准备之后发生 Unity 修改时拒绝过期写入。
这组测试不替代完整 `asset-api` CLI suite 中的 Rust 存储事务与 SDK 端到端验证。

共享语义层新增 YAML05–07：通过循环与别名逻辑路径修改已有托管对象、逻辑数组批次、
未开放类型创建的明确拒绝。driver 将 `properties` 请求交给生产
`unity_asset_core::semantic::SemanticAsset::lower_writes` 编译，测试不自行转换路径。

纯 Rust 地址/对象图/类型证据测试及完整服务入口测试：

```powershell
cargo test --manifest-path scripts/tests/unity-property/yaml-driver/Cargo.toml --target-dir src-tauri/target semantic::tests
cargo test --manifest-path src-tauri/Cargo.toml --lib unity_assets::property::tests --no-default-features
cargo test --manifest-path src-tauri/Cargo.toml --lib unity_serialized_property::property_tree::tests --no-default-features
```

SDK 模式选择、Tree 模式继承、本地预览、版本冲突、批次顺序和失败重放保护：

```powershell
bun run test src/__tests__/unityPropertyApi.test.ts src/__tests__/unityAssets.test.ts
```

P2–P4 累计测试在 `PropertyAuthoringParity.cs`：显式 managed 模板创建/替换、循环及别名、
嵌套 Variant 和 Scene Override/Revert/Apply、重复实例隔离、组件模板增删、曲线/渐变
与 live payload 的差分、来源 dirty/stale 防护，以及 1,000 请求单次导入的计时样本。
driver 直接调用生产 `authoring` / `prefab` / `semantic` 内核。服务级类型/assembly
验证、来源版本、跨文件日志、发现及 Agent 缓存的测试位于 `unity_assets/property/tests.rs`。

所有阶段累计运行，不替换早期用例。YAML07 仍要求拒绝隐式 `setType`；新增 YAML08–09
验证有显式初始化数据的 `createManaged`，二者不是相互矛盾的契约。

性能与正确性追加 YAML17–18：实际 Unity 生成的数字形状字符串覆盖、从小值来源到
Int64 最大值的 Override/Apply。YAML14 验证 Editor 返回预检/写盘/导入分段耗时。
完整 Rust 服务的 ignored 性能用例覆盖 Prefab 批次及 129 源码文件的类型校验场景：

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --lib batch_service_benchmark --no-default-features -- --ignored --nocapture
```

输出 `PROPERTY_BENCH` JSON。常规测试断言缓存重建计数、跨层命令顺序、源码版本冲突、
精确值、覆盖失败时的原图保留、Windows 路径别名及无变化事务的文件时间保留。

Review 回归与完整服务合成测试：

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --lib unity_assets::property::regression_tests --no-default-features
$env:LOCUS_PROPERTY_UNITY_EDITOR=$env:UNITY_EDITOR
cargo test --manifest-path src-tauri/Cargo.toml --lib review_unity_service_roundtrip --no-default-features -- --ignored --nocapture
# 将环境变量换成 Unity 2022 Editor，再执行同一个用例。
cargo test --manifest-path src-tauri/Cargo.toml --lib review_service_performance_matrix --no-default-features -- --ignored --nocapture
```

合成测试先让真实 Unity 生成资产并退出，再调用生产 `unity_assets::execute` 完成 schema、
逻辑批次和日志事务，最后启动 Unity 重载并用 SerializedObject 构造对照数据。覆盖嵌套 managed
类型与循环、嵌套列表 bool/float/string、500 请求批次、未知 schema 拒绝及原生模板父子拓扑。
隔离项目和 seed/verify 日志、服务 JSON 响应保留在输出的 `PROPERTY_SERVICE_EDITOR.root`；
测试只会在自身子进程超时时结束该进程，不触及其他 Unity 项目。

性能矩阵输出 `PROPERTY_SCALE`，包含字段数、写请求数、是否不同字段、完整服务耗时、
分阶段计时和响应字节数。常规回归检查一次编译、有限重建次数和顺序，不设置机器相关耗时阈值。

复杂 Prefab 的完整服务测试（10 组事务场景）：

```powershell
$env:LOCUS_PROPERTY_UNITY_EDITOR=$env:UNITY_EDITOR
cargo test --manifest-path src-tauri/Cargo.toml --lib complex_prefab_unity_roundtrip --no-default-features -- --ignored --nocapture
```

`PrefabMatrixComponent.cs` / `PropertyPrefabMatrix.cs` 覆盖继承数组、空自定义列表、
多层 Variant/Scene、已有 added/removed 拓扑、重复实例、managed 循环叶子、数组 Revert/Apply、
来源缩短后的旧索引记录。隔离目录由 `PROPERTY_PREFAB_EDITOR.root` 输出，保留请求清单、
服务响应与计时、Unity 重载报告和 Editor 日志。切换 Editor 环境变量可执行 Unity 2022。
能力边界和失败复现记录见 `docs/unity/property-tree-prefab-coverage-2026-09-18.md`。
