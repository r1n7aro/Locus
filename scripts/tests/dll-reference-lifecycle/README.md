# DLL 引用生命周期回归夹具

这是 opt-in 的集成回归夹具，直接验证当前生产实现。旧编译失效、扫描期间无法替换 DLL，或新版内容未被刷新后的任务识别，都会使测试失败。不生成候选实现，不修改生产源码，不启动或结束 Unity Editor，不访问业务项目。

## 运行

需要 Windows、Bun 和 .NET 10 SDK。夹具构建、DLL、索引和日志写入本次创建的 `%TEMP%/locus-dll-lifecycle-*`；同一测试入口还会将既有 C# 编译、扫描、索引和 scope 测试运行在 `%TEMP%/locus-dll-server-tests-*`。不使用共享编译服务的输出目录。

```powershell
$env:LOCUS_DLL_LIFETIME_PROBE = '1'
bun run test src/__tests__/dllReferenceLifecycleProbe.test.ts
```

可选：通过 JSON 数组传入已安装 Unity 的 `MonoBleedingEdge` 绝对目录，再运行同一命令：

```powershell
$env:LOCUS_DLL_PROBE_MONO_HOMES = '["E:/Unity/Editor/Data/MonoBleedingEdge","E:/Unity 6000.5.6f1/Editor/Data/MonoBleedingEdge"]'
```

这些是可替换的本机运行时路径，不是项目名单。Mono 用例加载仓库实际的 `Locus.Roslyn.dll`。独立进程不继承 Editor 的程序集解析器，因此夹具提供 `System.Memory 4.5.5` 及其依赖；这属于运行时组件验证，不能当成完整 Editor 集成验证。

直接运行 `bun scripts/tests/dll-reference-lifecycle/run.mjs` 也会输出 `LOCUS_DLL_PROBE_ROOT` 和 `LOCUS_DLL_PROBE_REPORT`。`report.json` 保存所有观察结果、输入源码 SHA-256 和运行时版本，其他 `.log` 保存构建与运行输出。夹具保留自身临时目录供检查。

## 测量方式

- 用 Roslyn 生成约 90 KB 的独立 DLL，旧版只含 `OldType`，新版只含 `NewType`，同时携带可区分的调用关系和嵌入式 PDB。
- 强引用旧编译；新任务覆盖写或原子替换 DLL、刷新引用并编译；最后恢复旧任务。分别覆盖未绑定和已完成一次编译的引用。替换阶段不触发 GC。
- 直接验证当前 `ReferenceCache` 的刷新和 `PruneExcept`，要求旧编译、新编译及两种文件替换均成功。
- 直接编译仓库 `CompileService` 和 registry 源码，验证指纹刷新及四个 scope 共用同一 DLL 的并发行为；各 scope 保持现有请求串行化。
- `CallerScan` 使用当前源码的临时副本，只改命名空间并在读取元数据/PDB 后、遍历 IL 前加入同步停顿点，文件读取逻辑保持原样。在停顿点替换 DLL，再检查旧结果与新结果，要求替换及两次扫描均成功。
- Mono 用例在每个运行时执行覆盖写/原子替换 × 冷/热引用，四个旧编译共享同一个引用；实际执行编译以验证快照可用性。
- 额外记录相同指纹、相同大小与时间戳的缓存行为，不把这些观测当作本修复的成功断言。仅在独立回收用例中执行 GC，保持缓存对象存活，验证淘汰的旧引用可以回收；不将此用例解释为原生内存压力测试。

扫描同步停顿点的源码锚点发生变化时夹具会报错，避免静默测试错误位置。缓存、编译服务和 registry 直接编译仓库源码；扫描不再应用任何候选替换逻辑。

## 2026-09-22 修复与验证

环境：Windows；.NET 10.0.11 / Roslyn 5.6；Unity 2021.1.23f1c1 和 Unity 6000.5.6f1 随附 Mono，使用仓库打包的 Roslyn 3.8。尚未覆盖 Unity 2022 或真实 Editor 内的端到端流程。

| 路径 | 修复前 | 修复后 / 边界 |
| --- | --- | --- |
| `MetadataReference.CreateFromFile` | 引用存活时可替换，旧、新编译均完成；两个 Mono 运行时同样通过 | 无需替换此 API |
| `ReferenceCache` 刷新/淘汰 | DLL 可替换；旧编译抛 `ObjectDisposedException`，新编译成功 | 淘汰仅移除缓存引用，旧、新编译均成功 |
| `CallerScan.BuildIndex` | 扫描停顿期间覆盖写和原子替换均失败；结束后可替换 | 预读完整 PE 后，两种替换均成功，旧、新调用关系正确 |
| `CompileService` 指纹更新 | 新任务识别新版 | 不需修改正常刷新路径 |
| 四个并发 scope | 按现有 gate 使用同一 DLL，刷新后各自识别新版 | 不需增加全局锁或取消现有 gate |
| 同指纹 / 同大小同时间戳 | 仍复用旧内容 | 属于缓存刷新契约，本修复保持原行为 |

生产改动限定为 `ReferenceCache.cs` 与 `CallerScan.cs`，同时修正前者关于 `MetadataReference.CreateFromFile` 长期映射文件的不准确注释。业务发布流程、进程管理、项目识别及缓存版本判定逻辑保持不变。
