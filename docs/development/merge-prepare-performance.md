# Merge prepare 性能优化

## 接口与执行范围

已知目标文件时传 `paths`；完整替换使用 `mode="files"`，字段/对象合并使用默认 `mode="structural"`。

```python
job = await locus.merges.prepare(
    workspace_ref=workspace_ref,
    sources=[{"commits": [source_oid]}],
    paths=["Assets/Scenes/Blockout.unity"],
    mode="files",
)
plan = job.plan()
await plan.files.take("Assets/Scenes/Blockout.unity", version="source", commit=source_oid)
preview = await plan.preview()
```

路径按仓库根目录解析，支持精确文件名；资产与 meta 成对进入可选范围，不自动纳入结果。路径范围外不能执行文件、字段或对象写入。移动文件时需同时声明原路径和目标路径。省略 `paths` 保留完整目标快照行为。

`prepare/get` 只返回 job 标识、状态、目标 HEAD、计数和性能信息。`job.snapshot_page(kind="target" | "dependencies", offset=0, limit=100)` 分页读取快照标识；`job.changes()` 分页读取差异。结构差异在查询或选择字段/对象时才生成；清空计划和完整文件选择不会提前生成字段目录。

## 快照与缓存

- 新 job 使用 journal v2。目标文件保存工作区原始字节，保留 CRLF 等差异。
- 范围外的干净依赖引用固定 Git blob。脏文件、未跟踪依赖、assume-unchanged/skip-worktree 文件，以及带 filter/working-tree-encoding 的依赖保存真实工作区字节。
- 固定版本通过 `refs/locus/merge-jobs/<job-id>/` 保持可达。Git 版本与工作区修改覆盖共同构成冻结的依赖输入，预览不读取之后变化的工作区来替代它们。
- 原始 blob 在仓库的 `.git/locus/merge-jobs/blobs-v2` 共享；linked worktree 使用其 common Git directory。发布采用临时文件和原子安装，重复 job 不再复制同一份内容。
- Git 依赖按对象大小分批读取，普通批次最多约 32 MiB、1,024 个对象；单个超大对象独立处理。批量 stdin/stdout 同时推进，避免管道互等。
- GUID 索引、引用摘要、脚本 schema 和差异目录按固定输入缓存。字段 schema 只解析相关脚本；来源树中的其他脚本不会在 prepare 阶段全量解析。
- v1 journal 不被隐式转换，仍使用原来的 job-local blobs，并可继续读取、规划和应用。v2 的依赖描述与共享存储不会交给旧版本读取器解释。

## 并行与一致性

文件快照、内容发布、哈希校验和差异目录计算共用一个有上限的 Rayon 线程池。默认按逻辑处理器数的两倍配置，限制在 8–32 路；`LOCUS_MERGE_WORKERS=1..32` 可用于部署调优或单线程基准。大文件快照读取使用约 256 MiB 的并发字节预算，单个超过预算的文件独占预算，解析缓存继续保留其独立上限。

校验改为流式哈希，不创建新 blob。job/事务 journal 使用 256 KiB JSON 写缓冲，保留原子替换与同步；可重建缓存只要求原子发布。

并行处理不会放宽写入边界：HEAD/index/工作区目标、脏依赖和影响 Git 比较的配置仍需通过一致性检查。丢失 GUID、删除对象后仍存在的反向引用继续阻止应用。包含范围外脏代码的已应用 Unity 验证不能直接证明较窄的候选提交，仍需验证确切提交树。

RPC 的工作区与 Git 文件系统协调 guard 由真正执行工作的 blocking worker 持有。等待者超时/断连不会提前释放 guard。

## 验证与基准

本次 merge 回归测试通过 86 项（另有 1 项手动基准默认忽略）；Python merge/worktree/assets 接口测试通过 23 项。覆盖冻结引用、旧 journal、CRLF、过滤器与配置变更、范围限制、共享 blob 并发发布、源文件缺失、取消后 guard 生命周期，以及较窄提交的验证隔离。

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --lib merge_jobs:: -- --test-threads=8
cargo test --manifest-path src-tauri/Cargo.toml --lib prepare_performance_baseline -- --ignored --nocapture
$env:PYTHONPATH = "python"
python -m unittest discover -s python/tests -p test_merges.py -v
python -m unittest discover -s python/tests -p test_worktrees.py -v
```

性能 fixture 在临时仓库中生成 1,200 个无关资产及配对 meta，加一个 32 MiB 的无关未跟踪 GLB，然后比较按路径准备和完整准备的首次/重复执行。输出 `MERGE_BENCH` JSON，包含实际调用耗时、阶段耗时、目标文件数、依赖数、worker 数、共享 blob 总字节数和响应大小。计时不作为单元测试的硬阈值，避免机器负载影响正确性测试。

`prepare_metrics.phases` 将来源 Git 操作、快照、目录构建和目标校验分开记录。`total_ms` 是 journal 写入前的累计时间；基准的 `prepare_ms` 覆盖整个调用，包括 journal 写入。

### 本机基准记录（2026-09-09）

Windows、32 个逻辑处理器，debug 测试构建。不同并行度分别在新临时仓库运行；数字是一次采样，不代表原业务项目的耗时或固定加速比。

| 场景 | 1 路 | 32 路 | 32 路目标路径数 | 32 路共享 blob 总量 | 返回体 |
| --- | ---: | ---: | ---: | ---: | ---: |
| 按路径首次准备 | 1,960 ms | 1,869 ms | 2 | 238 B | 624 B |
| 按路径重复准备 | 1,828 ms | 1,669 ms | 2 | 238 B | 623 B |
| 完整首次准备 | 3,792 ms | 2,666 ms | 2,402 | 33,747,960 B | 597 B |
| 完整重复准备 | 2,313 ms | 1,585 ms | 2,402 | 33,747,960 B | 596 B |

2 个目标路径包括该资产和不存在的配对 meta；2,400 个范围外干净依赖仍有冻结的 Git 身份。按路径准备没有复制无关 GLB。重复准备后共享 blob 总量不增长。完整首次准备的快照阶段从 2,172 ms 降到 1,436 ms；完整重复准备的快照阶段从 656 ms 降到 248 ms。按路径准备的剩余耗时主要是 Git 查询、私有版本固定和一致性检查。
