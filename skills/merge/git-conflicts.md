---
summary: >-
  在用户明确要求 Git merge/cherry-pick，或已有 Git 操作停在冲突时读取。区分 Git 索引冲突与 locus.merges 选择性集成，按文件类型解决并继续原操作。
tools:
  - bash
  - read
  - write
  - edit
  - python
  - unity_execute
---

# Git 操作与现有冲突

## 开始用户指定的 Git 操作

先利用当前已读取的状态判断目标分支与操作是否一致。用户已明确 Git merge 或 cherry-pick 时执行该方式及其指定参数；不要先用 SDK 应用同一批修改，然后再重复执行 Git。

- Git merge 按用户选择处理快进/合并提交以及是否停在提交前；不要擅自追加 `--no-ff`、`--squash`、`--autostash` 等改变语义的选项。
- cherry-pick 按所选提交的依赖顺序执行；只选指定 commits，不默认扩展为连续 range。遇到冲突时先完成当前一项，再继续序列。
- 脏工作区不是一律拒绝执行的理由。先检查与本次改动的实际重叠；Git 若因重叠修改拒绝操作，指出具体路径，再按用户意图选择保留工作区的 SDK 应用、先处理那些本地修改或隔离执行。不要为了“干净”自动搬走所有工作。

`locus.merges.prepare` 会拒绝已有 unmerged index 和正在进行的 merge/cherry-pick/rebase 等操作。遇到这种状态按下面流程解决，不循环重试，不自动 abort 后换一种 Git 语义。

## 定位和解决冲突

合并读取一次 `git status --short`、`git diff --name-only --diff-filter=U` 和 `git ls-files -u`，按实际路径处理。需要三方内容时用 `git show :1:<path>`、`:2:<path>`、`:3:<path>`，并结合当前操作解释它们：1 是共同基线，2/3 是此次索引的两侧；rebase 中不要把 ours/theirs 简单等同于用户口中的“我的/别人的分支”。新增/删除冲突可能没有某一 stage。

### 普通文本和代码

读冲突段与必要的调用上下文，保留双方需要的行为；使用文件编辑工具移除冲突标记并组合代码。新增/删除/重命名冲突按用户需要决定最终路径和内容。相同业务判断可批量处理，无需每个文件都询问；只有无法从代码和用户目标推断的业务选择才问。

已确认只取完整一侧的路径，可使用 `git checkout --ours -- <path>` 或 `git checkout --theirs -- <path>`；确认删除时用 `git rm -- <path>`。这些选择按路径执行，不把一个冲突的结论扩展到整仓库。

### Unity 资产

这里解决的是 Git 索引中的冲突，与普通 SDK 集成计划分开处理：

- **完整选边或二进制**：Git 的完整版本恢复适用；对资产与 `.meta` 保持正确的 GUID 配对。完整版本恢复不等于手写 YAML。
- **已经能使用 Locus Collab 语义冲突面板/接口**：用其自研 YAML 字段/对象合并完成现有索引冲突。当前公开 Python `locus.merges` 没有这些 `git_merge_semantic_*` RPC，不杜撰可调用入口，也不为此临时创建 View 或调试网页。
- **Agent 需要组合字段且没有可调用的索引语义接口**：选定完整版本作为工作文件基底，保留索引三方内容供对照；从中确定需要补入的字段，再复用当前 Editor，通过 `unity_execute` 的 Unity/SerializedObject API 修改并保存目标资产。先使工作文件成为合法的完整资产，再导入；不要让 Editor 导入冲突标记。目标 Editor 未运行时可按需启动，仍不需要新 worktree；编译或导入问题只处理实际涉及部分。只有 Editor 确实无法运行且结构化结果无法可靠确定时，才留下该资产的具体待决项，其余能完成的冲突继续处理。

普通、尚未进入 Git 操作的 YAML 修改优先走主文档的 `locus.merges` 快速路径，不因为这一节就为所有合并启动 Editor。复杂图或类型迁移确实需要另一个版本的运行时证据时，才按主文档申请额外 checkout/Editor；不要为了绕开 unmerged 检查修改索引或清除 Git 操作标记。

## 结束当前操作

完成一个文件后显式 `git add -- <resolved-path>`；内容已由 Collab 暂存时不重复处理。只暂存实际解决的路径，检查剩余 unmerged 项与本次结果 diff，运行与改动相称的检查。

没有剩余冲突后，按用户要求的完成层级执行原操作的 `git merge --continue`、`git cherry-pick --continue` 等。仅要求解决冲突、保留提交前状态时停在那里。`--skip` 会丢弃当前提交，`--abort` 会放弃当前操作，不能用来掩盖尚未解决的问题。不要在原操作之上调用 `plan.commit` 代替 Git 的历史语义。

报告完成的操作和实际 commit（若有），以及保留的本地修改或仍待决定的具体冲突；不生成额外审计文件或要求与任务无关的检查。
