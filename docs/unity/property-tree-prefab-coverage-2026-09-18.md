# 复杂 Prefab 覆盖扩展（2026-09-18）

本轮遵循“合成样例 → 确认失败 → 扩展实现 → Unity 重载验证”。`OnValidate`、
`ISerializationCallbackReceiver` 等业务回调不在实现范围内，由后续 View 自行实现相应逻辑。
模式仍由 Property API 实例选择，不新增前端控件或逐控件模式配置。

## 已复现的失败

| 样例 | 修复前行为 | 修复方式 |
| --- | --- | --- |
| Base → Variant 的数组尺寸覆盖 | `property.invalid_path: items.Array.size`，整棵树不可读 | 按层先恢复数组尺寸，再处理叶子覆盖；嵌套尺寸按父子顺序执行 |
| Variant 新增组件/子对象，同时删除继承对象 | `property.prefab_added_topology_unsupported` | 解析 added records，按源 GUID/对象和实例归属补齐组件/父子链接，保留 removed 投影 |
| 两个相同嵌套实例中的新增组件 | 同上 | 显式 stripped 身份优先，各实例独立映射源对象 ID |
| 继承 managed 对象的循环/别名字段 | 拒绝 `managedReferences[rid].amount` | 将逻辑别名路径和 Unity wire path 归一到宿主 registry 中的同一 rid |
| Base 中为空的自定义列表 | `property.prefab_array_template_required` | 从已验证的 C# schema 构造序列化零值结构，支持其中的嵌套空数组；不运行构造器 |
| 对整个数组 Revert | 返回成功但实际 `changedFiles=0`，子项覆盖仍在 | 删除目标对象、目标实例内该属性的完整覆盖子树 |
| 数组跨两层 Apply | `property.apply_scalar_required` | 数组降为 size/leaf 覆盖；到达实体来源时调用共享编辑器；清除跨过层的对应子树 |
| Base 数组变短但 Variant 留有旧索引覆盖 | `property.unsupported_prefab_path` | 暂不应用越界元素覆盖，保留原始记录；源数组恢复长度后再次生效 |
| Unity 6 SceneRoots 引用 PrefabInstance | 有效图校验报 dangling local reference | 仅在展开投影中将实例根句柄转换为有效根 Transform，保持磁盘格式不变 |

最初六项集成失败保留于 `<runtime-root>/property-prefab-<run-id>`。
空列表失败位于 `<runtime-root>/property-prefab-<run-id>`；
Revert/Apply 失败位于 `<runtime-root>/property-prefab-<run-id>`；
旧索引和 SceneRoots 失败位于 `<runtime-root>/property-prefab-<run-id>`。

## 合成测试

`PropertyPrefabMatrix.cs` 让真实 Unity 生成 Base、Variant、嵌套 Prefab 和 Scene。
Editor 完全退出后，Rust 测试调用生产 `unity_assets::execute`，包含类型证明、依赖版本、
候选图校验和日志事务；再启动 Unity 导入并检查最终对象。没有绕开存储层或使用测试专用写入口。

当前包含 10 组事务场景：

1. 继承数组 insert → move → delete → resize → 修改新元素；使用 SerializedObject 构造实时 API 对照。
2. 两层 Variant 中自定义对象列表内的嵌套列表编辑。
3. 新增组件、新增子对象/孙对象、删除组件和删除子对象混合的结构读取与属性修改。
4. 重复嵌套实例中，仅修改左侧实例的新增组件，右侧保持原值。
5. 继承 SerializeReference 的循环别名字段写入，保持对象身份和循环。
6. Base → Variant → Variant → Scene 的嵌套字段修改与来源传播。
7. 空自定义列表增长，布尔、浮点、数字外观字符串、嵌套列表插入/移动。
8. 整个数组 Revert，保留旁边另一个数组的覆盖。
9. 数组逐层 Apply 到中间 Variant 和实体 Base，并确认跨过层的数组覆盖已清除。
10. Base 缩短后保留失效的旧索引记录，其他字段仍可编辑。

重载还验证左右实例的本地对象引用各自指向本实例的子对象。
这是既有引用的继承投影验证，不代表已经支持任意跨层引用 Apply。

无需 Unity 的测试覆盖：乱序嵌套尺寸、覆盖合并与失效项清理、managed rid 宿主作用域、
错误拓扑作用域/缺失目标拒绝、类型保真、未知 schema 拒绝、过期依赖、失败不落盘，以及
插入后连续 1,000 次写入新元素的一次校验批次。耗时不作为跨机器的通过阈值。

## 性能处理

- 普通继承标量保留原有按来源分组校验和批量覆盖路径。
- 结构批次先在共享 `SemanticAsset` 中按请求顺序执行，并保留每个原请求做类型校验；
  最终有效对象只生成一次，避免为每个中间数组状态反复生成 YAML。
- 数组仅生成最终尺寸和叶子覆盖，清理该数组的旧记录，不扩大到其他属性或实例。
- 生成覆盖后仍预检有效图；`summary` 省去结果树构建，但不跳过该正确性检查。
- 空数组类型结构只在有 Prefab 继承的事务中准备，使用同一份已捕获源码及依赖版本。

## 明确边界

- 已支持复杂 **既有** added/removed 拓扑的读取和字段写入；在继承层直接创建/删除组件、
  创建/删除子对象的 `editObjects` 命令仍未开放。实体层模板创建能力保持原有范围。
- 继承 managed 对象的已存在字段可写；继承层创建/替换 managed 类型及 managed 数组结构编辑仍拒绝。
- 普通数组及内联自定义结构数组可编辑并整体 Revert/Apply；需要重映射非空本地对象引用或 rid
  的数组 Apply 明确拒绝。任意对象引用跨实例、跨层 Apply 尚未实现。
- 若要 Apply 的嵌套数组在目的层还没有对应元素，需选择目的层已存在的外层数组进行 Apply。
  不自动把叶子 Apply 扩大到任意祖先数组。
- 空列表元素需要可证明的字段 schema；未知、条件化、partial 或没有已知布局的原生类型
  不猜测默认结构。默认结构是序列化零值，不执行 C# 字段初始化器或构造器。
- 回调、Undo 和任意自定义 Inspector 业务行为不属于离线 YAML 语义保证。

## 运行

```powershell
cargo test --manifest-path scripts/tests/unity-property/yaml-driver/Cargo.toml --target-dir src-tauri/target --quiet
cargo test --manifest-path src-tauri/Cargo.toml --lib unity_assets:: --no-default-features
$env:LOCUS_PROPERTY_UNITY_EDITOR='<unity-install-root>/6000.5.8f1/Editor/Unity.exe'
cargo test --manifest-path src-tauri/Cargo.toml --lib complex_prefab_unity_roundtrip --no-default-features -- --ignored --nocapture
# 将环境变量改为 <unity-install-root>/2022.3.47f1/Editor/Unity.exe 后重复最后一条。
```

每次保留隔离目录 `E:/LocusTemp/property-prefab-<uuid>`，其中 `prefab-matrix.json`
是请求清单，`service-results.json` 是逐项生产 API 结果与计时，
`prefab-matrix-results.json` 是 Unity 重载结果，`Seed.log` / `Verify.log` 是 Editor 日志。

## 本轮验证结果

- Unity `6000.5.8f1`：10/10 事务成功，11 项重载断言全部为 true。
  证据：`<runtime-root>/property-prefab-<run-id>`。
- Unity `2022.3.47f1`：同一套 10/10 事务成功，11 项重载断言全部为 true。
  证据：`<runtime-root>/property-prefab-<run-id>`。
- 共享内核 83 tests、资产服务 47 tests、Property Tree 42 tests、前端 4 files / 48 tests 通过。
  资产服务默认忽略的 5 项中，本轮显式运行新 Prefab 合成测试及 2 个 Prefab 性能用例；
  原有完整服务 roundtrip 和通用规模矩阵没有重复执行。
- Prefab 性能回归保留一次来源校验、2 次有效图/树构建；本次 debug 完整响应采样，
  8 对象 × 33 字段的 1,000 请求约 459 ms，含 129 份源码的 typed 场景约 539 ms。
  采样时另有 Unity 导入和服务测试运行，因此不与之前不同负载下的计时作速度比承诺。
