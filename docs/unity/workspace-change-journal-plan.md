# Workspace Change Journal 与 Unity 增量同步计划

状态：已实现并通过单项目、双项目 CLI 验证
日期：2026-08-28
范围：workspace 文件监听、AssetDb 引用图、Unity AssetDatabase 同步、脚本重编译、多项目并发

## 背景

Locus 当前已经为每个 workspace 启动 `AssetDbWatcher`，递归监听 `Assets`、`Packages` 与链接资源根目录，并将变化投影到 Locus 自己的引用图数据库。Unity 重编译链路另有一套 changed-path 队列，用于在 Auto Refresh 被 Locus 编辑会话抑制时显式导入脚本。

现状具备两类问题：

- 引用图 watcher 的 dirty queue 在 worker 取出路径后即消费完成，没有可供其他系统使用的持久事件序号与成功水位。
- Unity 重编译无法证明 watcher 从上次成功编译至今持续完整，因此只能使用全量 `AssetDatabase.Refresh` 兜底。

隔离 Unity 2022.3.47f1 实验已经确认：Directory Monitoring 开启时，`RequestScriptCompilation` 可以发现已被 AssetDatabase 识别的 `.cs` 内容修改；新增 `.cs`、删除 `.cs`、`.asmdef` 变化仍需显式 AssetDatabase 同步。

## 目标

- 一套 OS 文件监听基础设施同时服务引用图、Unity 编译同步及后续 LSP/预览缓存消费者。
- 每个消费者拥有独立成功水位，引用图入队与 Unity 编译成功互不冒充对方的完成状态。
- watcher 完整性可证明时，Unity 重编译优先使用定向批量 `ImportAsset(Default)`。
- watcher 发生空窗、丢事件、结构变化或删除时，Unity 自动回退到一次全量 `Refresh`。
- 支持多个 Unity 项目并发编辑、导入与重编译；一个项目的事件 burst、锁竞争或健康降级不影响其他项目。
- 保留现有 edit-session owner 语义、hot-reload pending paths、Unity Test discovery pending paths 与旧插件协议兼容。

## 非目标

- 本阶段不移除 Locus 引用图数据库现有的 mtime 纠偏与 10 分钟新 `.meta` 发现。
- 本阶段不使用 watcher 直接修改 Unity AssetDatabase；Unity API 仍只在 Unity 主线程执行。
- 本阶段不让 watcher 的普通资源事件触发自动编译。
- 本阶段不承诺跨 Locus 进程恢复健康状态；应用重启后的第一次重编译执行全量 Refresh，建立新的可信基线。
- 本阶段不将所有 workspace 文件内容写入 journal，也不在事件热路径计算文件 hash。

## 核心语义

### 共享监听与独立消费

```text
notify / Locus 文件工具 / 插件安装
                  │
                  ▼
         WorkspaceChangeHub
    路径归一化、seq、generation、health
                  │
        ┌─────────┴─────────┐
        ▼                   ▼
 RefGraphProjection   UnityCompileProjection
 DirtyQueue/DB        compile ack / sync snapshot
```

`WorkspaceChangeHub` 是通用 workspace 变化枢纽。它负责记录事实和完整性，不执行引用图解析或 Unity API。消费者按自己的过滤规则读取快照并推进自己的水位。

### 事件记录

```rust
WorkspaceChange {
    seq: u64,
    generation: u64,
    path: String,
    kind: Upsert | Delete,
    source: OsWatcher | LocusWrite | PluginInstall | Reconcile,
}
```

- `path` 使用 workspace 相对路径和正斜杠。
- 路径限定在 `Assets/`、`Packages/` 及映射后的链接资源路径。
- 所有可归一化事件先生成共享 envelope；引用图立即投影到自己的 DirtyQueue/DB。
- Unity projection 只持久合并编译输入，同一路径保留最新记录；`seq` 保留该项目的全局发生顺序。
- rename 归一化为旧路径 Delete 与新路径 Upsert；无法获得完整 rename 两端时降低 health。
- journal 记录不依赖 `.meta`，因此覆盖 Auto Refresh 关闭后的新脚本。

### 健康状态

```text
Unverified  当前进程尚未通过一次 Unity 全量同步建立基线
Healthy     watcher 连续在线，事件流没有已知空档
Suspect     watcher 空窗、Rescan、错误、结构变化或容量溢出
```

以下事件必须 fail closed，将状态置为 `Suspect` 并推进 generation：

- notify `Rescan` 标记或 watch error；
- watcher 停止、重启或 workspace 因资源策略进入 idle；
- 目录删除、目录整体 rename、无法完整配对的 rename；
- journal 容量达到上限；
- `Packages/manifest.json`、`Packages/packages-lock.json` 或包根结构变化；
- 路径无法可靠归一化到所属 workspace。

只有一次成功的 Unity 全量 Refresh 可以把 `Unverified` / `Suspect` 恢复为 `Healthy`。恢复时要求 watcher 仍在线，且快照 generation 与完成时 generation 相同。

## Unity 编译输入范围

UnityCompileProjection 至少关注：

- `.cs`
- `.asmdef`
- `.asmref`
- `.rsp`
- `.dll`
- `Packages/manifest.json`
- `Packages/packages-lock.json`
- Locus Unity 插件安装目录内的编译相关文件

普通 `.meta` 事件不进入 Unity 编译 pending set；新脚本自身的 Create/Modify 事件已经提供路径。目录与包结构事件通过 health 降级进入全量路径。

## Unity 同步决策

重编译开始时获取不可变快照：

```text
UnitySyncSnapshot {
    through_seq,
    generation,
    health,
    mode: None | Targeted | Full,
    paths,
    reason,
}
```

决策规则：

| 条件 | 同步模式 |
|---|---|
| hub 不存在、Unverified、Suspect、watcher 不在线 | Full |
| 包清单或包结构变化 | Full |
| 任意 Delete 或目录结构变化 | Full |
| Healthy 且只有已知新增/修改路径 | Targeted |
| Healthy 且没有编译输入变化 | None |

现有 hot-reload pending paths、Unity Test pending paths 与 agent 明确写入路径必须合并到快照。磁盘上已经不存在的明确路径自动把模式提升为 Full。

### Targeted

- Unity 主线程调用 `StartAssetEditing`。
- 对全部路径调用 `ImportAsset(path, ImportAssetOptions.Default)`。
- `finally` 中调用 `StopAssetEditing`。
- 任一导入失败时请求失败，journal 不推进 compile ack。
- 导入可能自行启动编译；目标 epoch 逻辑继续识别该次编译。

### Full

- Unity 主线程执行一次 `AssetDatabase.Refresh(ImportAssetOptions.Default)`。
- 同一请求不再执行逐文件 ImportAsset。
- 成功编译或 Unity 明确返回 `not_needed` 后，建立/恢复 Healthy 基线。

### None

- 释放 edit-session Auto Refresh 计数。
- 如果 Unity 插件内部仍有明确 queued paths，则执行一次定向批量导入。
- 其余情况直接调用 `RequestScriptCompilation`。

## 水位与并发编辑

- 快照记录 `through_seq`，编译成功后只确认 `seq <= through_seq` 的变化。
- 编译期间产生的新事件继续保留，下一次重编译处理。
- 编译失败、超时、Unity 崩溃或导入失败均不推进水位。
- 全量同步只有在 generation 未变化时恢复 Healthy；同步期间发生 Rescan/空窗会保留 Suspect。
- 同一路径重复事件按最新 kind 合并，Delete 后再次 Create 最终表现为 Upsert。
- journal ack 与现有 hot-reload coordinator、Unity Test pending source 清理使用相同的“成功后按快照序号清理”原则。
- 定向快照为不超过 4 MiB 的编译输入保存 BLAKE3 内容指纹；成功确认时只丢弃内容完全一致的延迟 OS 重复事件，同路径并发编辑继续保留。

## 多项目并发约束

- 每个 canonical project root 对应独立 `Arc<WorkspaceChangeHub>`。
- 每个 hub 拥有独立 mutex、seq、generation、pending map、health 和 watcher 生命周期。
- 全局 registry 只保存 project key 到 `Weak<WorkspaceChangeHub>` 的映射；事件热路径不持有全局 registry 锁。
- Unity 项目 A 的事件、full-refresh 降级、compile ack 与 journal 容量不影响项目 B。
- 继续使用现有 per-project Unity operation lock，保证同一项目内重编译串行；不同项目可以并行。
- project key 在 Windows 上大小写不敏感，并移除扩展路径前缀与尾部分隔符。
- workspace runtime 被资源策略停止 watcher 时，仅标记该项目 Suspect，不遍历或锁定其他项目。

## 性能约束

### 事件热路径

- 每个事件只做路径归一化、扩展名判断、HashMap 合并和 seq 递增。
- 禁止在 hub record 路径读取文件内容、计算 hash、访问 SQLite 或调用 Unity。
- 单次 mutex 临界区只更新当前项目的内存状态。
- notify receiver 不递归扫描目录；目录结构事件直接降级 Full，避免阻塞 receiver 导致 Windows buffer overflow。

### 内存与快照

- 每个项目 pending map 设置硬上限，初始为 16,384 个唯一路径。
- 达到上限后停止接纳新的 Unity pending 路径并标记 Suspect；正确性由下一次 Full 保证。
- Unity 快照只复制编译输入路径，不复制普通资产记录。
- 路径排序和去重只在重编译快照阶段执行一次。
- 文件存在性检查和小文件内容指纹只在 snapshot/ack 阶段执行，并在项目 mutex 外完成。
- 成功 ack 后立即移除已确认记录，避免活跃项目长期积累。

### Unity 主线程

- Targeted 模式只产生一个 Asset Pipeline import batch。
- Full 模式只产生一个 Refresh。
- 不使用 `ImportAssetOptions.ForceUpdate`。
- 不在 Unity 主线程等待 watcher、文件 hash 或 Rust journal 锁。

## 生命周期与故障恢复

- hub 由 `WorkspaceCoreServices` 持有，生命周期长于可启停的 AssetDbWatcher。
- watcher start/stop 显式通知 hub。
- watcher 初始化失败时 workspace 注册可以报告错误，同时 hub 保持 Suspect。
- AssetDb 引用图完整扫描期间 watcher 被停止；重新启动后 hub 保持 Suspect，下一次 Unity 重编译建立新基线。
- Locus 进程重启后 hub 从 Unverified 开始，第一次 Unity 重编译执行 Full。
- Unity 进程重启不自动清除 journal；成功 Full/compile ack 后再推进水位。

## 协议兼容

`request_recompile` 消息升级为 JSON：

```json
{
  "schema": 1,
  "syncMode": "targeted",
  "paths": ["Assets/Foo.cs"],
  "reason": "healthy_known_changes"
}
```

- 新 Unity 插件接受 JSON 与旧换行路径格式。
- 旧格式按 Full 语义处理，保持正确性。
- 新 Rust 对旧插件发送 JSON 时，旧插件无法识别路径但仍执行其既有全量 Refresh，保持正确性。
- `request_recompile` 的开始确认、compile epoch、domain reload 与 `not_needed` 协议保持不变。

## 可观测性

每个项目至少暴露：

- watcher active；
- health / health reason；
- generation / next seq / compile ack；
- pending 总数与编译输入 pending 数；
- 上次 Full / Targeted / None 决策及原因；
- overflow、Rescan、watch error 计数。

Asset Overview 不再硬编码 queue length 与 current file。Unity recompile 结构化结果增加 `asset_sync` 与 `asset_sync_reason`，便于定位性能回退。

## 实施顺序

1. 新增通用 `workspace_changes` 模块、per-project hub、registry、事件模型、健康状态和 Unity 快照。
2. `WorkspaceCoreServices` 持有 hub；AssetDbWatcher start/stop 与 notify receiver 接入 hub。
3. 保留现有 RefGraph DirtyQueue，将其改为消费 hub 归一化后的路径事件。
4. `unity_bridge::recompile_and_wait_inner` 获取 UnitySyncSnapshot，合并现有明确路径。
5. `request_recompile` 升级 JSON；Unity 插件实现 None/Targeted/Full。
6. 编译成功与 not-needed 路径推进 compile ack；所有错误路径保留 pending。
7. 修复 watcher overview 的真实 queue/current/health 可观测性。
8. 增加故障注入、并发项目与 Unity CLI 性能回归。

## 实现结果

- `WorkspaceChangeHub` 已作为每个 `WorkspaceCoreServices` 的长期数据平面；AssetDbWatcher 与 RefGraph 共用其归一化事件 envelope，Unity 编译使用独立 projection 水位。
- notify `Rescan`、watch error、watcher start/stop、真实目录结构事件、包控制文件、删除和容量溢出均 fail closed。
- Unity Directory Monitor 的 `~UnityDirMonSyncFile~...~` 哨兵文件与 Windows `Name(Any)` 父目录噪声已过滤，避免正常 Refresh 把新基线误降级。
- 60 秒 mtime 对账会补偿已有编译输入的漏改/漏删；10 分钟目录发现会补偿漏掉的新脚本，包括尚无 `.meta` 的 `.cs`。
- Unity 插件协议已支持 `None / Targeted / Full`；旧换行请求按 Full 兼容。
- Asset Overview 已暴露真实 watcher queue/current 与 journal health、generation、水位、pending 和降级计数。

2026-08-28，Unity 2022.3.47f1 隔离 CLI 实测：

| 场景 | 结果 | 耗时 |
|---|---|---:|
| 仅 RequestScriptCompilation，已有 `.cs` | 收敛 | 2.093 s |
| 仅 RequestScriptCompilation，新 `.cs` | 45 秒内未收敛 | 45.179 s |
| 仅 RequestScriptCompilation，删除 `.cs` | 45 秒内未收敛 | 45.215 s |
| 仅 RequestScriptCompilation，修改 `.asmdef` | 45 秒内未收敛 | 45.056 s |
| Locus，64 个新 `.cs` | Targeted，类型探针通过 | 3.377 s |
| Locus，修改 `.asmdef` | Targeted，程序集探针通过 | 3.122 s |
| Locus，删除 `.cs` | Full，删除类型探针通过 | 2.599 s |
| Locus，无变化 | None / up_to_date | 1.539 s |

双项目单进程 workspace CLI 同时连接两个独立 Unity 进程，得到 2 个 watcher、2 个 LSP 进程、不同 checkout/runtime/service generation 与不同 editor session id，8 项隔离检查全部通过。

## 验证矩阵

### Rust 单元测试

- 同一路径 Modify 合并，Create→Delete、Delete→Create 状态正确。
- rename Both 生成旧 Delete 与新 Upsert。
- Rescan/error/start-stop 空窗推进 generation 并进入 Suspect。
- snapshot/ack 只清理 through_seq 以前的记录。
- ack 期间新增事件保留。
- targeted snapshot 遇到磁盘缺失路径升级 Full。
- 两个 project root 的 seq、health、pending 与 ack 完全隔离。
- 16,384 路径容量触发 Suspect，其他项目不受影响。

### AssetDbWatcher 测试

- 新 `.cs` 无 `.meta` 时 hub 仍记录 Upsert。
- `.asmdef/.asmref/.rsp/.dll` 内容变化进入 Unity projection。
- watch error 与 Rescan 标记 hub Suspect。
- 引用图原有 22 个 watcher 测试继续通过。
- watcher stop/restart 不丢 hub，health 按设计降级。

### Unity CLI driver

- Healthy + 64 个新增 `.cs`：一次 Targeted batch，无全量 Refresh。
- Healthy + `.asmdef` 修改：Targeted 并正确编译。
- Healthy + 删除 `.cs`：一次 Full。
- Suspect/Rescan：一次 Full，成功后恢复 Healthy。
- Healthy + 无变化：None，快速返回 `up_to_date`。
- 编译期间追加新文件：第一次 ack 后仍保持 pending，第二次编译消费。
- 两个隔离 Unity 项目并发运行不同模式，事件和结果不串项目。

### 性能验收

- 64 个已知新增/修改文件只产生一个显式 Targeted import batch。
- 事件记录不执行文件内容读取或 SQLite 操作。
- 两项目并发事件写入无共享项目级锁竞争。
- journal snapshot 时间与编译输入变化数线性相关。
- Full 只在 fail-closed 条件触发，结构化日志明确给出原因。

## 完成标准

- 日常已知脚本与 asmdef 新增/修改稳定走 Targeted。
- 删除、包变化与监听完整性异常稳定走 Full。
- 无变化且 watcher Healthy 时稳定走 None。
- 所有成功水位按项目与快照序号推进，失败不丢变更。
- 多项目 CLI 并发回归通过。
- 现有引用图、hot reload、Unity Test discovery 和旧插件兼容测试保持通过。
