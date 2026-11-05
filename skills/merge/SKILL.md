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

一次读取目标 checkout、当前分支、暂存/未暂存状态、正在进行的 Git 操作，以及所选来源的提交列表与提交说明。变更路径、与目标的重叠、已应用项和冲突以 prepare 后的 catalog 为准，不再用 `git diff`/`comm` 另算一遍；只对 catalog 和 preview 标出的问题展开具体文件。本地 ref 足够时不额外 fetch。

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

1. 以当前脏工作区准备一个 job，通常一个请求包含本次所选的全部 commits。合并整条分支的最终结果时用 `sources=[{"branch_ref": ref, "squash": True}]`：只比较合并基点与分支末端，中间提交改了又改回的内容不产生冲突，不自建临时 squash 提交。只取其中部分文件时才在 `prepare(paths=[...])` 传入精确文件路径；合并整条分支或整组提交时不传 `paths`，也不手工计算路径清单。完整替换场景、Prefab 或其他文件时使用 `mode="files"`，直接 `job.plan().files.take(path, version="source", commit=oid)`，无需预先生成字段目录。资产/meta 会成对进入可选范围，但仍分别显式选择。需要字段合并时使用默认 `mode="structural"`。`new_plan(default="keep_target")` 默认保留目标，然后按用户范围批量 include/exclude。用户要全部所选提交时可直接 `include(all=True)`，无需逐字段确认。
2. 范围明确时直接按路径或 commit 选择。需要了解内容时先用 `job.change_summary()` 看每个文件的状态与类型计数和 `destination_changed`（为 false 时目标未改过该文件，纳入即得到来源版本），再用 `job.all_changes(status="conflict")`（可加 `paths=[...]`）只读需要决定的变更及其 `reason` 与 change ID；大场景可能有上万条 clean 变更，不要拉取全量 catalog 再本地过滤，clean 变更用 `include(all=True)` 或 `include(path=...)` 直接纳入。需要字段上下文时才调用 `job.inspect_asset(path, version=...)`。大场景无需完整展开对象树。
3. 一批选择完成后执行一次 `preview()`。每个 issue 带 path、相关 change ID 和处理方式（`detail`，LFS 缺失时还有 `fetch_command`），按它处理后再预览；`preview`、`validate(level="static")`、`check_dependencies` 当前共享静态预览，不连续调用三遍。不要为了让预览通过而排除来源的正当改动，也不要在计划外用 `git restore`/文件工具写目标文件；按 issue 的处理方式仍无法消除时，停下来向用户报告 job id 与 issue 原文，这通常是工具缺陷。
4. `ready_to_apply` 后直接 `apply(expected_plan_hash=preview.plan_hash)`，只应用既定计划。没有新信息时不再要求用户确认同一范围。保存并核对返回的路径、`editor_sync` 与 `warnings`；重复 apply 只返回幂等结果，不为重读结果再调用。

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

准备后的目标若有新增修改，旧计划会返回 `stale`。保留那些修改，用 `job = await job.replan()` 以同一组来源重新快照：目标字节未变的文件沿用原有决定，`job.replan_report` 的 `dropped` 和 `changed_paths` 列出需要重新选择的部分，补做后再预览；不要用 reset 或强制覆盖消除 stale。继续已有 job 用 `locus.merges.get(...)` 和 `job.plan()`，不反复 `new_plan()` 清空已经作出的选择。

## 手工解决内容冲突

“手工”是理解两边意图后明确组合结果，不是将所有冲突统一选成 ours/theirs。

- **Unity YAML**：简单字段、对象和文件差异直接用 Locus 自研合并 API。按需读取冻结的 base/target/source/result，再使用 `fields.take/set`、`objects.*` 或 `files.*` 表达结果。`include` 只表示纳入，不等于解决冲突；`plan.resolve(change_id, side=...)` 使用 catalog 中的 change ID，每个 `conflict` 都带 `reason` 说明分歧和可选的边。
- **Prefab 覆盖等身份序列**：`m_Modifications`、实例上新增/移除的对象（`m_Added*`、`m_Removed*`）、`RefIds`、`m_Component`、`m_Children`、场景 `m_Roots` 按元素合并，两侧对不同元素的增删改直接组合，冲突落在具体元素或其字段上。需要“取一侧整段再去掉某项”时，先 `resolve(序列 change_id, side="source")`，再对元素路径 `fields.delete`；缺失的元素用 `fields.take(side=...)` 原样复制。不要用 `fields.set` 重写整段来代替元素操作，也不要回到 Editor 里手工撤销覆盖。
- **没有元素身份的列表**（普通值/结构体列表、`[SerializeReference]` 的 `{rid: N}` 列表）：冲突的 `reason` 给出下一步。两侧都只追加时，用 `fields.set` 把该路径设为“目标元素 + 来源新增元素”；两侧改了不同元素时，先 `resolve` 一侧，再 `fields.take` 另一侧的 `<路径>/<序号>`。`fields.set` 落在冲突 change 的同一路径上即是该冲突的解决，不需要先 exclude。
- **SerializeReference 删除/修改**：一侧删掉、另一侧改了同一个托管引用时，冲突在 `RefIds/@rid=` 元素上，`reason` 列出指向它的字段。要保留来源的修改，`resolve(side="source")` 之外还要对这些字段 `fields.take(side="source")`，否则恢复的引用没有人指向，Unity 下次保存会丢弃它。
- **对象删除/修改**：来源删除的对象与目标的修改冲突时，同一次删除的其余部分（GameObject、组件、子 Transform 以及 `m_Component`/`m_Children`/`m_Roots` 中的引用）也会是冲突，用同一个 side 一起解决：`target` 保留并保持挂接，`source` 整体删除。目标删除了对象而来源修改了它时，要保留来源的修改就调用 `plan.objects.restore(path, object_id=...)`（`reason` 会给出），它一次带回所属的 GameObject、全部组件、子物体（含 Prefab 实例）直到目标仍保留的最近对象，并补上那里的 `m_Component`/`m_Children`/`m_Roots` 引用；不要逐个 `objects.add` 或手工 `fields.take`。
- **同一 fileID 的两个不同对象**：`reason` 为 “both sides added an object with this fileID” 时只能保留一侧，向用户说明另一侧的对象需要重新添加。同一个 Editor 进程里对同一场景基线编辑两次（例如在打开的 Editor 里切换分支）会出现这种情况。
- **代码和普通文本**：先 `include`。两侧改了不同行时，它直接做三方行合并并保留目标的换行；仅换行不同的工作区文件按未修改处理。对 `conflict`（带 `hunks`）用 `inspect_asset(path, version="result")` 读出带冲突标记的文本，理解两边意图后组合，再用 `plan.files.set_text(path, merged)` 写进计划：它随 plan hash 应用、可 abort，不需要改目标文件或重新 prepare。删除/新增分歧用 `resolve(change_id, side=...)`。不要用 `git merge-file` 或普通文件工具绕过计划写目标。
- **二进制/完整选边**：按用户目的明确选取完整版本，用 `files.take(path, version="source", commit=oid)` 等操作。不能把 `include` 当作二进制内容选择。
- **引用、类型或依赖冲突**：只检查问题涉及的对象与依赖；必要修改一起纳入，其余保持排除。不要为方便直接选择整张 Scene/Prefab，也不要通过通用文件工具手写 Unity YAML。对象 ID/rid 选择器用字符串；设置数值引用字段时用 Python 整数。

如果用户要求实际 Git merge/cherry-pick，或目标已有未合并索引，按需读取本包的 [Git 操作与现有冲突](git-conflicts.md)：通过 `knowledge_query` 定位该文档，再 `read` 返回的物理路径。`locus.merges.prepare` 不接管已有 Git 操作，不向带冲突标记的资产重复 prepare。

## 按需要使用 Editor 与 worktree

- 不默认创建 worktree、申请项目池或启动第二个 Editor。普通代码/文本与可由 YAML API 明确合并的资产，优先在当前 checkout 完成。
- 需要运行时对象、反序列化结果、编译后的类型或实际场景行为时，先复用目标已打开的 Editor。只有必须同时观察另一个版本、隔离执行，或用户指定新落点时，才用 `locus.worktrees`；参数按需读 `python` 的 `worktrees` 帮助。
- 额外来源槽位检出准确的 source OID。多个 Editor 的查询可用 `asyncio.gather`，每次调用显式传 `worktree=handle`；目标新建后使用 `job.workspace_ref`。不要通过切换 Agent 工作目录来轮流控制 Editor。
- Editor 观察用于帮助选择；冻结快照才是计划依据。首次导入、复杂运行时复现不是每次合并的固定步骤。

## 验证与完成

`apply` 的返回包含 `editor_sync` 和 `warnings`。计划里同时有代码、文本、meta 或新文件时，写盘后会让已连接的 Editor 重新导入资产，并从磁盘重载受影响的已打开场景和 Prefab Stage，无需手动重开场景。若 `warnings` 报告某资产在 Editor 中有未保存修改（stale），先向用户说明，再处理该资产，避免保存时覆盖合并结果。脚本改动在 Editor 下次刷新时编译，依赖新类型前先 `unity_recompile`。

验证与本次变化相称：简单改动看计划、应用结果及局部 diff；代码改动用项目现有的相关检查；应用了 Unity 资产或代码时，一次 `plan.validate(level="unity")` 覆盖编译、导入、缺失脚本与引用检查，不再逐项手查场景 dirty、类型是否存在或 git 状态计数；validate 本身报错时向用户报告，不用手工检查替代。仅应用 YAML 修改不要求必启 Editor。后端报告具体阻塞时处理该项，不额外设置“必须工作区干净”“必须全量扫描/编译/测试”的入口条件。

需要通过 `plan.commit` 提交 Unity 资产/代码时，遵循其已有 Unity 证据要求；通常 `plan.validate(level="unity")` 检查已应用目标。只有精确部分提交需要独立候选验证时才传 `paths=...`；容量已满时先释放不再需要的来源 Editor。不要为每次预览预先准备验证槽位。

按用户指定的完成层级结束：只应用就保留工作区修改；需要 stage/commit 时显式列出路径。同路径原有本地修改也要纳入提交时使用 `include_local_changes=True`，依据用户已有意图决定。选择性集成提交是单父提交，不冒充 Git merge/cherry-pick；真正的 Git 操作沿其原流程完成。保留无关暂存和本地修改，不默认 stash、reset、clean 或 `git add -A` 整理全区。

最后简述：来源与范围、采用的 Git 方式、解决的关键取舍、验证结果，以及当前停在“已应用/已暂存/已提交/仍有具体冲突”哪一步。仅清理本次拥有且已不再使用的临时资源；已有 Editor 保持原用途。
