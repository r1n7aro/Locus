---
summary: >-
  用于合并分支改动、把多个 commit 的修改应用到当前脏工作区，或手工解决 Git/Unity 资产冲突。按用户意图选择只应用修改、Git merge 或 cherry-pick。
tools:
  - bash
  - read
  - write
  - edit
  - python
  - knowledge_query
---

# Merge

把用户选定的改动整合到目标中，保留目标已有且未要求替换的工作。修改内容与 Git 历史处理是两个选择；按用户已有要求完成，不在每一步重新征求确认。

## 选择落点与 Git 方式

一次读取目标 checkout、当前分支、暂存/未暂存状态、正在进行的 Git 操作，以及所选来源的提交列表和变更路径。先看目录与差异摘要，只展开相关文件；本地 ref 足够时不额外 fetch。

| 用户意图 | 执行方式 |
| --- | --- |
| 拿来这些修改、挑选多 commit、整合到脏工作区，未要求改变历史 | 默认在当前 checkout 用 `locus.merges` 应用修改，保留 HEAD/index；不自动提交 |
| 明确要求 Git merge、保留分支合并关系 | 按指定目标和参数执行 Git merge |
| 明确要求 cherry-pick、逐提交保留来源 | 按指定顺序执行 cherry-pick |
| 已有 merge/cherry-pick 等操作停在冲突 | 继续处理当前操作，不另起一次集成 |
| 只比较或预览 | 读取差异或生成 preview，到此结束 |

目标默认是当前工作区，不为了合并先切换分支。把来源 ref 一次解析为 commit OID，明确选取范围与依赖顺序；非连续选择不把中间未选提交带入。遇到 merge commit 时指定正确的 mainline，细节按需查 SDK 帮助。上下文足以判断就执行；只有来源、目标、业务取舍或最终 Git 语义确实无法确定时，集中问清缺失项。

## 常用路径：在当前工作区应用修改

通过 `python` 工具调用已内置的 `locus.merges`，执行修改的 Python body 使用 `readonly=false`。参数不熟悉时，从 `python` 工具描述给出的 SDK 文档目录读取 `merges.md`；同一任务内复用已读契约。

1. 以当前脏工作区准备一个 job，通常一个请求包含本次所选的全部 commits。范围明确时在 `prepare(paths=[...])` 传入精确文件路径；完整替换场景、Prefab 或其他文件时使用 `mode="files"`，直接 `job.plan().files.take(path, version="source", commit=oid)`，无需预先生成字段目录。资产/meta 会成对进入可选范围，但仍分别显式选择。需要字段合并时使用默认 `mode="structural"`。`new_plan(default="keep_target")` 默认保留目标，然后按用户范围批量 include/exclude。用户要全部所选提交时可直接 `include(all=True)`，无需逐字段确认。
2. 范围明确时直接按路径或 commit 选择。需要了解内容时分页读 `job.changes()`，只返回文件、状态与相关 change ID；需要字段上下文时才调用 `job.inspect_asset(path, version=...)`。大场景无需完整展开对象树。
3. 一批选择完成后执行一次 `preview()`。直接处理返回的冲突和依赖问题；`preview`、`validate(level="static")`、`check_dependencies` 当前共享静态预览，不连续调用三遍。
4. `ready_to_apply` 后直接 `apply(expected_plan_hash=preview.plan_hash)`，只应用既定计划。没有新信息时不再要求用户确认同一范围。返回后核对应用路径与变更摘要。

```python
# 在 Python 工具的 async body 中运行；source_oids 已由用户范围解析得到。
job = await locus.merges.prepare(
    workspace_ref=workspace_ref,
    sources=[{"commits": source_oids}],
)
plan = await job.new_plan(default="keep_target")
await plan.include(all=True)  # 此例中用户要纳入这些 commits 的全部修改。
preview = await plan.preview()
if preview.ready_to_apply:
    applied = await plan.apply(expected_plan_hash=preview.plan_hash)
    print(applied)
else:
    print(job.id, preview.issues)  # 针对这些问题修正同一个 plan 后再预览。
```

准备后的目标若有新增修改，旧计划会返回 `stale`。保留那些修改，在新目标上重新准备并按新 catalog 重建相关选择；不要用 reset 或强制覆盖消除 stale。继续已有 job 用 `locus.merges.get(...)` 和 `job.plan()`，不反复 `new_plan()` 清空已经作出的选择。

## 手工解决内容冲突

“手工”是理解两边意图后明确组合结果，不是将所有冲突统一选成 ours/theirs。

- **Unity YAML**：简单字段、对象和文件差异直接用 Locus 自研合并 API。按需读取冻结的 base/target/source/result，再使用 `fields.take/set`、`objects.*` 或 `files.*` 表达结果。`include` 只表示纳入，不等于解决冲突；`plan.resolve(change_id, side=...)` 使用 catalog 中的 change ID。只有需要双方行为信息时才进一步查询 Editor。
- **代码和普通文本**：保留双方必要逻辑，用普通文件工具拼接。如果必须修改计划涉及的目标文件，先完成文本编辑，再 prepare，并 exclude 已解决路径；若已有计划，完成手工编辑后重新 prepare。不要编辑目标后继续 apply 旧 hash。
- **二进制/完整选边**：按用户目的明确选取完整版本，用 `files.take(path, version="source", commit=oid)` 等操作。不能把 `include` 当作二进制内容选择。
- **引用、类型或依赖冲突**：只检查问题涉及的对象与依赖；必要修改一起纳入，其余保持排除。不要为方便直接选择整张 Scene/Prefab，也不要通过通用文件工具手写 Unity YAML。对象 ID/rid 选择器用字符串；设置数值引用字段时用 Python 整数。

如果用户要求实际 Git merge/cherry-pick，或目标已有未合并索引，按需读取本包的 [Git 操作与现有冲突](git-conflicts.md)：通过 `knowledge_query` 定位该文档，再 `read` 返回的物理路径。`locus.merges.prepare` 不接管已有 Git 操作，不向带冲突标记的资产重复 prepare。

## 按需要使用 Editor 与 worktree

- 不默认创建 worktree、申请项目池或启动第二个 Editor。普通代码/文本与可由 YAML API 明确合并的资产，优先在当前 checkout 完成。
- 需要运行时对象、反序列化结果、编译后的类型或实际场景行为时，先复用目标已打开的 Editor。只有必须同时观察另一个版本、隔离执行，或用户指定新落点时，才用 `locus.worktrees`；参数按需读 `python` 的 `worktrees` 帮助。
- 额外来源槽位检出准确的 source OID。多个 Editor 的查询可用 `asyncio.gather`，每次调用显式传 `worktree=handle`；目标新建后使用 `job.workspace_ref`。不要通过切换 Agent 工作目录来轮流控制 Editor。
- Editor 观察用于帮助选择；冻结快照才是计划依据。首次导入、复杂运行时复现不是每次合并的固定步骤。

## 验证与完成

验证与本次变化相称：简单改动看计划、应用结果及局部 diff；代码改动用项目现有的相关检查；确有 Unity 导入/行为疑问时再验证受影响资产。仅应用 YAML 修改不要求必启 Editor。后端报告具体阻塞时处理该项，不额外设置“必须工作区干净”“必须全量扫描/编译/测试”的入口条件。

需要通过 `plan.commit` 提交 Unity 资产/代码时，遵循其已有 Unity 证据要求；通常 `plan.validate(level="unity")` 检查已应用目标。只有精确部分提交需要独立候选验证时才传 `paths=...`；容量已满时先释放不再需要的来源 Editor。不要为每次预览预先准备验证槽位。

按用户指定的完成层级结束：只应用就保留工作区修改；需要 stage/commit 时显式列出路径。同路径原有本地修改也要纳入提交时使用 `include_local_changes=True`，依据用户已有意图决定。选择性集成提交是单父提交，不冒充 Git merge/cherry-pick；真正的 Git 操作沿其原流程完成。保留无关暂存和本地修改，不默认 stash、reset、clean 或 `git add -A` 整理全区。

最后简述：来源与范围、采用的 Git 方式、解决的关键取舍、验证结果，以及当前停在“已应用/已暂存/已提交/仍有具体冲突”哪一步。仅清理本次拥有且已不再使用的临时资源；已有 Editor 保持原用途。
