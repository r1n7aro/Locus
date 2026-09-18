# 内置 Profile Skill 与 API Review

本轮范围为本地 Unity Editor / Play Mode。`unity_execute` 与 `unity_run_states` 共用 `ctx.Profiler`；原有状态机 profiler helper 保持兼容。

## 检查发现与修复

| 原问题 | 修复 |
| --- | --- |
| API 与状态机 session 绑定，execute 只能手写 Unity API | 抽出共享 ProfilerApi，接入 execute 的成功、异常、取消清理路径 |
| 指标名固定，缺少类别、单位和数据类型发现 | 可分页发现实际注册的 marker/counter，提供可组合领域指标和自定义 reader |
| recorder 无样本时也读取数值；只按整数读值 | 检查已完成样本，支持 Double；缺失值与合法零值区分，GPU 无效零耗时记为缺失 |
| 不同指标样本可能错列；CSV 限制小数精度 | 缺失指标保留同一行，CSV 使用完整双精度格式 |
| 全局 recording 被开启后未恢复；未确保 CPU 模块可用 | 引用计数管理 recording、CPU/GPU 模块及 Editor capture 状态；清理时恢复先前设置 |
| 尖峰使用调用时帧号；Top-N 替换后“最后帧”不一定最新 | 保存观测点元数据，去重，并按观测时间寻找最新保留尖峰 |
| 找不到指定线程时退回其他线程 | 返回明确错误，提供线程索引发现和精确选择 |
| 统计仅 avg/p95/max/last | 增加缺失率所需计数、median/p90/p99、标准差、首尾变化、预算分析和基线差异 |

## 实现入口

- [内置 skill](../../knowledge/skill/profiler.md)：两种工具的示例、指标选择、分析规则、能力边界和文件格式。
- [共享 API](../../locus_unity/Editor/LocusBridge.Profiler.cs)：会话所有权、兼容 helper、资源清理。
- [指标发现](../../locus_unity/Editor/LocusBridge.Profiler.Metrics.cs)：CPU/GPU/内存/渲染及动态 subsystem 指标。
- [采集与导出](../../locus_unity/Editor/LocusBridge.Profiler.Captures.cs)：线程/帧筛选、Frame Timing Manager、内存快照。
- [统计](../../locus_unity/Editor/LocusProfilerAnalysis.cs)及[对比分析](../../locus_unity/Editor/LocusBridge.Profiler.Analysis.cs)。

普通计数器采样默认不启用全局 Profiler。需要调用层级时显式启用 `CaptureHierarchy`，需要 GPU 模块时启用 `CaptureGpu`。Unity 的 `ProfilerRecorder.Reset()` 会停止采样，采样循环中不调用它。

采样是对“最近完成的 recorder 数值”的观测，不是无损的逐帧 trace。Edit Mode 的多个更新可能看到相同数值；Play Mode 默认按 Unity 帧号去重。Profiler buffer 帧号只作为邻近帧候选，GPU 延迟与跨时钟关系会写入采样策略，不能据此声称精确归因。

## Unity 6.5 核对

官方发布说明确认新增 2D Profiler 模块（sprite/atlas）、实验性 USS Stats Profiler，以及 Profiler 的 AI 入口。2D 指标以运行时注册结果为准；USS/atlas 详情没有被假定为通用 recorder API。GPU 计时仍受图形 API、驱动、平台和 Frame Timing Stats 配置限制。[Unity 6.5 文档](https://docs.unity3d.com/6000.5/Documentation/Manual/WhatsNewUnity65.html)

内存快照通过 `Unity.Profiling.Memory.MemoryProfiler.TakeSnapshot` 保存 `.snap`；对象保留关系分析仍交给 Memory Profiler。API 会跟随 Profiler 连接目标，因此帧采集与快照会先检查 Editor 目标，拒绝远程 Player。目标判断缓存绑定 Unity 内部的 `IsConnectionEditor`，接口不可用时返回明确错误。原生快照不可取消，取消只结束等待。[Unity Memory Profiler API](https://docs.unity3d.com/6000.5/Documentation/ScriptReference/Unity.Profiling.Memory.MemoryProfiler.html)

## 验证

- Unity 6.5.8f1 / Direct3D12 的隔离 Editor 与 Play Mode：51 项行为检查通过，包括连续 CPU/内存采样、Double counter、真实 `.snap`、帧数据、统计/CSV、状态机休眠、成功/异常/取消清理。
- 本机该测试场景没有返回有效 GPU 耗时，验证的是“不可用”的正确表达，不能据此证明 GPU 计时在所有平台可用。
- 相关 4 个 Vitest 文件、17 个测试通过；原生检查作为其中一个 opt-in 测试执行。
- Unity 2022.3 的兼容性通过真实程序集编译检查。

重复运行（PowerShell，需要本机 Unity 和 .NET SDK）：

```powershell
$env:LOCUS_PROFILER_UNITY_EDITOR = '<unity-install-root>/6000.5.8f1/Editor/Unity.exe'
$env:LOCUS_PROFILER_OUTPUT_ROOT = 'E:/LocusTemp'
bun run test src/__tests__/unityProfilerApi.test.ts src/__tests__/unityRunStatesPreview.test.ts src/__tests__/unityExecuteProgress.test.ts src/__tests__/skillMarkdownFormat.test.ts
```

[测试入口](../../scripts/locus-profiler-check.ps1)创建独立目录并输出 `LOCUS_PROFILER_RUNTIME_JSON`，保留日志、CSV/JSON 和快照供检查；不会安装到现有项目或结束共享 Unity 实例。不设置 Unity 环境变量时，只跳过原生集成项。
