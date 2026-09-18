# 多 worktree、自研资产合并与 Unity 项目池：实施与验收记录

更新：2026-09-07。范围为阶段 1–3，Windows、Unity 6.5。此文件随实际 CLI 验收结果更新；“代码已实现”与“真实 Editor 已通过”分开记录。原始设计与其他平台/CoW 决策见 [方案](./worktree-parallelism-and-asset-merge-plan.md)。

## 实现范围

| 阶段 | 实现 | 主要代码 |
| --- | --- | --- |
| 1 | 同一逻辑项目的受管/外部 Git worktree、创建/导入/移除、包含本地修改的独立快照、session checkout 绑定、可调 session 与 Unity Editor 并发 | `workspace_service/worktrees.rs`、`commands/worktrees.rs`、`resource_policy.rs`、`WorktreeManager.vue` |
| 2A | 自研 Unity YAML 解析/索引/合并，保留原始字节与未知字段，字段/对象/文件 API，SerializeReference 图、64 位标识、结构化冲突与引用验证，Collab 接入同一内核 | `unity_asset_core/`、`merge/core_adapter.rs`、`MergeInspectorFieldTree.vue` |
| 2B | 多 commit 的显式选择计划、脏目标快照、默认保持目标、二进制完整选边、预览/apply/stage/commit 分离、可恢复日志、Python SDK、真实 Unity 与精确提交候选验证 | `merge_jobs/`、`commands/merge_jobs.rs`、`python/locus/_merges.py`、`sdk.rs` |
| 3 | 固定目录槽位、精确 Editor 版本匹配、复用私有 Library、租约与 epoch、旧 session/句柄失效、异常槽位隔离 | `workspace_service/pool.rs`、`scope.rs`、`runtime.rs`、`session/store.rs` |

前端沿用工作台 checkout 树、列表和上下文菜单；管理弹窗复用 BaseButton/BaseCheckbox、现有中性 surface/border/text token，设置页沿用数值控件。没有新增状态胶囊或装饰卡片。操作失败信息只在当前操作区域展示。

## 已固定的行为

- 所选 commit 使用各自的 parent delta，非连续选择不会把中间未选提交顺带合入。merge commit 必须指定 mainline。
- 每个新计划默认 `keep_target`。无冲突也不意味着自动纳入。Agent 显式 include/exclude/defer，并用字段、对象或文件操作确定结果；重叠操作必须消歧。
- YAML 由 Rust 内核处理，Python 是结构化客户端。FBX 等不透明文件必须指定完整版本；LFS 只使用大小和 SHA-256 核验通过的真实对象，缺对象返回诊断。
- `job.inspect_asset(...)` 可按 target/source/base/result 查看冻结资产的对象、字段、引用、稳定路径和解析诊断，支持分页；查询不依赖当前来源分支的可变文件，也不开放原始 YAML 写接口。
- 选择器严格校验参数名和类型，拼写错误不会扩大选择范围。对象 ID 与 rid 查询值使用字符串；设置数值引用字段时，Python 传递整数，以保留 64 位精度和 YAML 数值类型。
- 内容选择保留目标文件模式；来源 chmod 是独立决策。删除资产内对象时，检查冻结目标中对其 `(GUID, fileID)` 的反向引用，依赖修改仍需显式纳入。
- apply 只写计划内的 W，保留 HEAD 与 index。后续目标修改触发 stale；重试可幂等恢复，abort 只恢复仍匹配本任务输出的文件。
- 默认新 worktree 会创建并校验独立容器，拒绝已有文件或链接占用；Windows Git 采用命令级长路径选项，原子资产/日志写入使用完整 Windows 路径，不修改用户 Git 配置。
- stage/commit 必须显式列路径。路径本身包含旧本地修改时，须显式 `include_local_changes=True`；其他已暂存内容不被提交吸收。
- 部分集成生成单父提交，记录来源和选择 manifest，避免把未选择的来源历史标成已经合并。
- `validate(level="unity")` 验证已应用目标；`validate(level="unity", paths=[...])` 在独立池槽验证 HEAD 加选定路径的精确 tree。证据绑定 tree、parent、plan、paths 和本地修改纳入范围，提交前再次核对。
- 资产验证限于实际应用路径与 Unity 依赖；选择代码或导入设置时扩展检查范围。共享验证模板报告遍历进度，检查 Scene/Prefab、原生对象引用及 ScriptedImporter 元数据；managed type API 仅用于 Unity 支持的宿主类型。正常导入进度窗口通过可见进度条与中断按钮分类，保存、错误、安全模式等真实弹窗仍会阻断。
- Python 外层写租约可委托给同一运行的嵌套 SDK；授权绑定项目、仓库、checkout、epoch 和 run。跨 checkout 竞争返回可重试忙错误，避免 A→B/B→A 相互等待。
- 池槽只有在无运行租约、Editor 已关闭、源码/index 干净且版本匹配时才可复用。Library 不跨活动项目共享，也不建立可写硬链接。
- 目录复用递增持久化 `materialization_epoch`，独立于进程内 runtime generation。旧 session、窗口绑定与已打开编辑内容不能静默接受新代次。
- 会话 schema v44→v45 显式迁移；历史没有 epoch 的导出写 `empty`。

## 测试项目与编译基线

用户原始项目为 `C:\Projects\UnitySample`，声明 Unity 2022.3.47f1，且含已有暂存/未暂存/未跟踪修改。原项目的源码、HEAD 和 index 均作为需保留的输入。

初次 worktree 准备暴露两项环境问题：未跟踪的嵌套 Git 仓库不能被普通快照伪装成常规文件；可选 Asset Store 上传工具包有 67 个缺失的 LFS 对象。测试在私有 index 中构建移除该可选工具包的基线，未修改原项目 index，也未把 LFS pointer 写成资产。

按照用户“单独 agent 修复编译基线后再开始 worktree”的要求，修复仅发生在已创建的隔离目录：

`<worktree-test-root>\<run-id>\a`

独立 agent 处理 Unity 6.5 包版本兼容、EntityId API、Polybrush 内嵌补丁和实际被引用的 URP RenderGraph pass。冻结基线为 `<compile-baseline-commit>`，工作目录 clean。已跟踪安装好的 Locus 包全部 142 个文件，包括 Runtime/Editor fixture 与 native plugin，不依赖外部绝对包路径。完整修复清单见 `artifacts/worktrees-stage1/source-compatibility.md`。

已观测到真实 Unity 6000.5.8f1 batch7：退出码 0、0 条 C# 编译错误、101 个 Editor 程序集；共享/循环引用、null、`9007199254740993` managed-reference ID、Prefab 引用均正确；在未保存 Untitled 场景中生成资产前后，active scene、dirty 与 root count 不变。

随后使用现有 Locus CLI driver 完成 connect：native_broker `ready/editing`，`passed=1, failed=0, finished.ok=true`。验证后仅关闭该隔离目录的 Editor，为阶段 1 两个新 Editor 留出并发额度。

后续两个全新目录的首次交互式导入又暴露 URP Material Upgrade 前置窗口。用户已手动确认两处，材质升级完成。原 batch 验收未覆盖依赖首个渲染帧的 URP 全量材质升级，因此 `<compile-baseline-commit>` 只是编译基线。完整材质基线已冻结为 `<material-baseline-commit>`，目录 `<worktree-test-root>\<run-id>\a`：236 个材质、ToonRendererData、URPAsset v13、ShaderGraphSettings 和 URPProjectSettings（材质版本 10），共 240 个真实升级文件。测试 marker 和 Directory.Build.props 保留为未提交修改，未纳入该提交；后续新 worktree 从 HEAD 创建。CLI 同时增加真实主线程 `get_reload_state` 的 compiling/updating 检查，避免 native broker 已连通而主线程仍不可执行时过早启动探针。

阶段 2 的全新候选目录首次导入又生成了被仓库忽略的 IET/ProBuilder 默认项目设置。生产验证保留对新增/忽略源码文件的严格拒绝；测试将 Unity 实际生成的两个 `ProjectSettings/Packages/*/Settings.json` 显式冻结为基线 `<settings-baseline-commit>`，父提交为 `<material-baseline-commit>`。只更新阶段 1 专用 A checkout 的基线与这两个 index 记录，其他 index/flags、原有 dirty diff、设置文件工作字节以及原始项目 HEAD/index 均保持不变。证据：`artifacts/worktrees-stage1/package-settings-baseline-verified.json`。

## 复杂资产验收设计

测试资产由 Unity C# API 生成并保存快照，不由模型手写 YAML。Runtime 测试程序集和 Editor 生成/检查入口在 `locus_unity/*/MergeTesting/`。

| 样例 | 必须验证的结果 |
| --- | --- |
| 同一个 SerializeReference 对象被两次引用、循环回指、null、超过 JavaScript 安全整数的 rid | 值合并后仍保持对象身份、环与精确 ID，缺失类型为 false |
| 来源 speed、目标 health/local 独立修改，显式排除来源 incoming | 只纳入所选字段，保留目标字段和 excluded 值 |
| 同字段冲突 | include 不等于冲突解决；明确取 source 后再导入检查 |
| 同 rid 的多态类型替换与另一侧旧类型 data 修改 | 整组返回冲突，明确取类型/数据后真实 Unity 可加载 |
| 含重复共享元素的竞争列表重排 | 不猜数组身份，显式选择后顺序和共享引用都正确 |
| 嵌套 Prefab、组件引用、两侧 Transform 不同坐标修改 | 文档 fileID 稳定，合并坐标正确，Prefab/组件引用有效 |
| 旧字段代码/source commit 与目标 FormerlySerializedAs 改名及脏代码 | 只采信可靠别名；真实重新编译与反序列化后值进入新字段 |
| 非连续 commit、二进制完整选边、脏 index/W | 未选中间提交不出现；HEAD/index/无关内容保持不变 |
| preview 后额外写入、重复 apply、abort | stale 不覆盖新改动，重试和恢复符合日志记录 |
| Python 写工具内部调用 SDK | 使用真实委托租约，完成规划/应用/恢复，无死锁 |
| 精确部分提交候选 | 独立 Unity 编译选定 tree；提交不夹带无关脏/已暂存内容 |
| 槽位 A→新 commit | 同路径、同 Library 文件身份/字节、epoch 递增，旧 session 拒绝，新 Editor 导入新资产 |

## 已完成的非集成验证

以下测试按对应修改后的编译快照执行，实际 Unity 验收另列：

- 自研内核 30 项；Git 2.55 下最终合并事务、验证范围与候选验证 63 项，worktree 14 项、pool 8 项，共 85/85 全部通过。覆盖精确提交、崩溃恢复、选择器、反向引用、长路径写回、受限换行证明和 ignored 源文件隔离。
- scope 3 项、context 7 项、event 5 项、workspace lock 9 项、resource policy 7 项通过。runtime 20 项通过后，修正测试对 Windows 开机时长的假设，第 21 项重新编译后单独通过。
- 会话 store 全部 93 项通过，覆盖新库、历史迁移和导出。
- 共享 Unity 验证模板的 12 项行为检查及 Unity 6.5 两个 C# 编译探针通过；Windows 弹窗模块完整 Cargo 测试 6 项通过、1 项交互窗口测试未运行。
- 实际 Windows 共享文件句柄的 marker 清理回归 3/3 通过，覆盖延迟释放、重试中新 Editor 出现时拒绝、异常文件类型诊断。
- 前端全量 403 个文件、2,420 项通过；后续 epoch/窗口定点回归 62 项通过。WorktreeManager 14 项与资源设置 3 项通过；typecheck/typecheck:test 通过。Python SDK 32 项通过。
- 实际原项目 YAML corpus：5,574 个文件、40,392 个文档、293,106,053 字节，0 个解析错误；40 个非 YAML 签名文件单独跳过。独立优化构建含 I/O 30.7 秒，debug 48.2 秒。尚无同口径旧内核性能对照，不能据此声称倍数提升。

证据位于仓库 `artifacts/`：`unity-core-final-rebuilt-tests.log`、`merge-validation-repair/final-git255-*-tests.log`、`pool-finalize-runtime-rebuilt.log`、`worktree-final-session-migration-tests.log`、`merge-audit-python-tests.log`、`worktree-frontend-regression.log`、`unity-asset-core-corpus-report*.json`。全量前端结果记录的是该次快照，不包含共享仓库中其他任务此后新增的功能。

后续定点性能验收选取 12 个真实资产（65.08 MB、572 个文档）及两组 Unity 原生三方快照，14/14 字节一致且 ready。基于此发现，单次 prepare 对相同原始输入共享 AST，超出全局缓存预算的文件也不再重复解析三次。只重测 4 个相同输入：大场景 prepare 4056→2194 ms，大 Prefab 2640→1533 ms，4/4 正确性检查通过。全局缓存仍为 128 MiB。这是本次优化的单次桌面观测，未测得相对旧解析器的性能倍数；方法、采样限制与复现材料见 `artifacts/unity-asset-core-performance-comparison.md`。

## 分阶段 CLI 结果

阶段 1–3 的实际 CLI 验收全部通过。初次阶段 1 停在旧包兼容问题，第二次在首次导入/URP 前置窗口阻塞主线程时超时，均保留失败记录；最终通过来自补全材质基线后创建的全新目录。

- 通过日志：`artifacts/worktrees-stage1-upgraded/driver.log`，`workspace: passed=8 failed=0`、`worktrees: passed=6 failed=0`、`finished.ok=true`。
- 目录：`<worktree-test-root>\<run-id>\a` 与 `b`，同一 ProjectId、不同 CheckoutId，真实 Editor PID 为 58520 与 49844。
- 两个 session 的实际工具路由、事件路由、并行编译 scope、mock agent run、LSP 进程与资源指标通过；创建时转移 seed 本地修改后，两边写入互不影响。源目录 HEAD/index/dirty 保持不变。

阶段 2：`artifacts/worktrees-stage2-asset-merge-r9/driver.log` 与同目录 `acceptance.json`，`asset-merge: passed=29 failed=0`、精确提交子验收 12 项通过、`finished.ok=true`。

- 真实结果：health=75、speed=2.5、local=99、被排除的 incoming=20；共享/循环/null 引用与 `9007199254740993` ID 保留，Prefab 坐标 x=3/y=4 与组件引用正确。
- 同字段冲突、多态类型替换、重复共享元素重排、非连续 commit、二进制选边、stale 拒绝、256 文件原始字节往返、Python 嵌套写入全部完成；FormerlySerializedAs 实际编译迁移后 currentValue=20、localValue=99。
- 独立候选编译导入后提交 `<validated-commit>`，tree 为 `<validated-tree>`，单父关系及 tree 完全一致，无关 staged/W 与源 checkout 保留。
- 此轮候选验证成功，但已证明的换行变化仍使 Git 状态缓存报告 dirty，槽位回收记录了 cleanup warning。已增加受限影子索引刷新，3 项真实 Git 回归验证 index 记录/flags、HEAD/tree、工作字节不变且状态可恢复；本轮原始 CLI 日志仍如实保留 warning。验证未放宽到新增文件或真实内容变化。

阶段 3：`artifacts/worktrees-stage3-project-pool-r2/driver.log`，`project-pool: passed=25 failed=0`、`finished.ok=true`。完整报告位于 `<pool-test-root>\<run-id>\acceptance.json`。

- 槽位 `<pool-test-root>\<run-id>\unity-<slot-id>` 完成首次导入、关闭、commit 切换及重新打开。物理路径和 CheckoutId 不变，`materializationEpoch` 从 1 变为 2。
- 缓存 sentinel 文件 ID `<sentinel-file-id>` 保留；实际 `Library/ArtifactDB` 文件 ID `<artifact-db-file-id>` 与切换前的 67,108,864 字节长度保持，排除了重新复制或重建文件的假象。首次打开到检查约 4 分钟，第二次打开到检查约 27 秒；这是本次桌面单次观测，不是性能保证。
- 重新打开后 incomingValue=88、speed=2.5、alternateLabel="source alternate"，共享/循环/null 引用和精确 64 位 ID 均保留；旧 session、旧 WorkspaceRef、缺 epoch 的旧引用和旧窗口恢复均拒绝。历史 run 仍记 epoch=1，新 session/run 为 epoch=2。
- 两次关闭都记录了主 Editor 和导入子进程退出；正常关闭超时后对本次拥有的进程执行了强制结束。初次失败源于退出后短暂的 marker 共享占用，重跑通过有限重试清理。源码随后在专用槽位显式封存并释放；source checkout 的 HEAD/index 保持不变。

```powershell
$env:LOCUS_WORKTREE_TEST_ROOT = 'F:/LocusWTTests'
bun run locus:test:unity -- --project <冻结的6.5基线> --suite worktrees --install-plugin --connect-timeout-ms 1200000 --timeout-ms 1200000
bun run locus:test:unity -- --project <阶段1受管checkout> --suite asset-merge --install-plugin --connect-timeout-ms 1200000 --no-progress-timeout-ms 600000 --timeout-ms 1800000
$env:LOCUS_UNITY_POOL_TEST_ROOT = 'F:/LocusPoolTests'
bun run locus:test:unity -- --project <阶段1受管checkout> --suite project-pool --install-plugin --connect-timeout-ms 1200000 --no-progress-timeout-ms 600000 --timeout-ms 1800000
```

## 本轮边界

阶段 4 的 ReFS block clone、差分 VHDX、macOS/Linux CoW provider 未实现。当前项目池复用已有私有 Library，首次创建仍需检出源资产和导入。普通无稳定身份的序列、复杂 C# 类型迁移、无法证明的别名或依赖必须显式解决或补足验证，不做静默猜测。解析成功、静态引用检查和 Unity 实际导入是不同层次的证据。

按用户要求，本机 Git 已通过 winget 更新至 `2.55.0.windows.3`，凭据管理器 `2.9.0` 可用，`core.autocrlf=true` 保持；不维护旧 Git 兼容分支。安装证据为 `artifacts/git-upgrade.log`，[官方发行说明](https://github.com/git-for-windows/git/releases/tag/v2.55.0.windows.3)。新版 Git 实测仍无法把超过约 310 字符的极深目录直接作为 `-C` 工作根，隔离 global 配置也未解决；相关操作明确报告路径错误。本轮已验证短仓库根内的长资产/索引路径、原子写回和长路径隔离门控，不将这些证据扩展为所有超深仓库根均受支持。
