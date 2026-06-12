# Unity HotReload 兼容面扩展计划（B 组机制扩展 + C 组 DMD 迁移）

前置：`unity-hotreload-plan.md`（H0–H6 基础机制）、`unity-hotreload-expansion-plan.md`（H7 成员表面级扩展）。本计划在两者全部落地、三轮实机自检（36 正向 + 20 负向 + A 组补充）的基础上，继续扩大可热更的代码修改情形。

术语沿用：M1 批次共享绑定 / M2 垫片+注册表 / M3 调用点核查 / M4 字段 store / M5 删除语义 / M6 using 全文件重挂；`__LocusHotPatch_` 补丁程序集、`__LocusShims` 垫片类、`__LocusFields_*` store 持有类。

---

## 0. 决策结论：DMD 迁移是否降低总工作量？

**结论：不降低 B 组的工作量；它是一条独立的兼容面轨道，建议拆成两期并与 B 组并行排序，而不是作为 B 组的前置。**

理由（逐项对照）：

| 工作项 | 难点所在 | DMD 是否消解 |
|---|---|---|
| B1 泛型方法体 | Mono 泛型 **detour** 不可靠（按实例化 JIT） | 否——DMD 改变的是访问检查，不改变泛型方法无法整体 detour 的事实；B1 的解法（remove+add 垫片分解）与 DMD 正交 |
| B2 属性/索引器/事件新增 | **元数据表面**不存在 + 调用点重写（赋值/复合赋值/±=） | 否——这是 sidecar 语法/绑定层工作；DMD 只在 Unity 侧 apply 层 |
| B3 跨 asmdef | 验证性工作 | 否 |
| B4 unsafe 跟随 | 一行级编译选项 | 否 |
| B5 无域重载模式 | 验证性工作 | 否 |
| B6 partial 类型 | 兄弟 part 并批 + 布局归并 | 否 |

DMD 真正消解的是**运行时可见性墙**这一类（与 B 组不相交）：
- C1（部分迁移，仅 detour 成员）：kept 成员体引用其它程序集 internal 类型/成员——当前的盲区（PrepareMethod 只兜底垫片，kept 体的 internal 引用在首调时才炸）。真实项目里默认可见性（internal）的类满地都是，这是**当前最大的隐性冷源**。
- C2（全量迁移，垫片+store+调用图重映射）：删除「新增成员体只能引用 public 表面」约束（`FindShimAccessViolation` 整类移除）、删除 store/垫片必须 public 的约束。**只有 C2 反过来简化 B2**（属性垫片不再需要 public-only 雕花），但 C2 自身的调用图重映射复杂度 ≥ 它省掉的部分。

**推荐顺序**：B4（半天）→ A 组测例实机回归 → B1（解锁最大冷区）→ **C1**（独立高收益，验证 DMD 在 Unity Mono 上的现实可行性）→ B2（若 C1 顺利且决定推进 C2，可在 B2 设计里预留 relax 开关）→ B3/B5（验证矩阵)→ B6 → C2 决策点（凭 C1 实机数据决定）。

---

## B1. 泛型方法体修改转热（remove+add 垫片分解）

> **状态：已实现（2026-06-13，连同 B4）。** 实现与计划的差异/补充：
> 1. CallerScan 补了 MethodSpec→MemberRef/MethodDef 解析链（计划第 3 点的预判成立：此前泛型调用点确实漏报 fail-open），先行测例后放行分类。
> 2. 计划未明说的一个正确性缺口已补：**批内"未变更"调用方的强制重挂**。M3 只保证调用点文件在批内，但 kept 成员的补丁副本若不被 detour，改写后的垫片直调永远不生效（原方法体继续运行）。PatchRewriter 新增 ensure 通道：凡 kept 成员体内出现对"同签名重加成员"的引用，即把该成员并入 detour 集（含实例初始化器→全部实例 ctor）；围合成员本身不可重挂（泛型/Burst/显式接口/析构器/未变更运算符）→ 点名冷。
> 3. 同键墓碑抑制：同签名 remove+add 时墓碑注册被跳过，否则注册表 last-write-wins 会用 tombstone 覆盖 added 条目，破坏后续批次。
> 4. ClassifyAddedMember 放行泛型方法后新增一个冷形态：方法类型参数遮蔽声明链类型参数（CS0693 源码合法，但扁平化垫片会重复声明）→ 点名冷。
> 5. 泛型 Unity message 名（如 `Update<T>`/泛型类型内 `Update`）在分解前显式守卫为冷（引擎派发无法被垫片直调替代）。
> 6. 调用点显式类型实参仅在方法自身带类型参数时物化（链泛型维持推断，保持既有 golden 行为）；不可言说类型实参（匿名类型）回退推断。
> 7. 限制（沿计划+新增）：跨程序集调用点不可改写（M3 兜底）；每次再编辑该方法都要求其编译期调用点文件再次同批（M3 无状态地每批重验）；ref struct 接收者沿 added-member 既有按值语义。
>
> 测试：sidecar 183/183（基线 170 + 13 新增：MethodSpec×2、HotDiff×6、HotPatch E2E/冷×4+golden×1，B4×2 见下）；自检新增 P33a/b（泛型方法体+泛型类型方法体、调用方不动靠 ensure 重挂）、N01 改为「调用点不在批内」点名形态、N15 改为泛型类型 ctor 体改。实机一轮自检待跑。

**现状**：`generic method body changed` → cold（泛型方法与泛型类型内方法体一律冷，detour 不可靠是根因）。

**机制**：不碰 detour。把「泛型方法体修改」按 H7c 既有路径分解：
1. HotDiff `DiffMember` 方法体变更分支：`genericContext || 方法自身泛型` 时不再直接冷，改为产出 `RemovedMembers`（墓碑、不需要 stub）+ `ChangedMethods(added)`（同签名重加）+ `RequiresCallerCheck`（M3 扫描旧方法名）。
2. M2 垫片已支持泛型（`GenericShim`，类型参数随声明链展开）——新体编译成泛型静态垫片，批内调用点物化为 `Shims.Foo<T>(self, args)` 直调（直调泛型静态方法 = 普通泛型调用，不依赖 detour）。
3. M3 核查：旧方法的编译期调用点必须全在批内（IL 扫描已有 `MemberRef/MethodSpec` 路径——**注意**泛型调用走 `MethodSpec` token，`CallerScan.ScanIl` 需确认 MethodSpec→MemberRef/MethodDef 的解析链已覆盖，没有则补）。
4. 再编辑连续性：泛型垫片不做旧→新 detour（H7b 已显式跳过 GenericShim），靠每批重发直调重绑——语义已是现状，无新增工作。

**限制（写进工具描述与冷原因）**：跨程序集调用点不可改写（M3 fail-closed 已兜底）；反射/已捕获泛型委托仍指向旧体（盲区清单已有）；虚泛型方法不适用（虚成员删除已冷）。

**DoD**：sidecar 测试：泛型方法体改→hot 分类+垫片直调 E2E（CoreCLR 可执行验证）；泛型类型内非泛型方法体同路径；跨批内调用点不全→cold 点名文件。自检新增 P：corpus 泛型类 `Echo<T>` 体改+调用方同批→行为断言；负向保留「调用点不在批内」形态。

**规模**：sidecar 为主，~2-3 天级。风险：MethodSpec 扫描遗漏导致漏报（fail-open）——必须先写 CallerScanTests 的 MethodSpec 用例再放行分类。

## B2. 属性 / 索引器 / 事件新增转热

**现状**：`property added` / `member kind addition not hot-reloadable` → cold。

**机制**：新增属性 = `get_X`/`set_X` 方法对的 M2 垫片化；新增索引器 = `get_Item`/`set_Item`；新增事件（访问器式）= `add_X`/`remove_X`。Sidecar 工作集中在**调用点物化**：
1. `ClassifyAddedMember`：属性/索引器/事件不再冷；按访问器拆成多个 `HotDiffMethod(added)`（命名沿用访问器约定，与 M5 删除侧的 `AccessorName` 对称）。
2. PatchBatch `AddedMembers` 键从 IMethodSymbol 扩展到 IPropertySymbol/IEventSymbol → 多个 ShimTarget（每访问器一个）。
3. PatchRewriter 调用点物化矩阵：
   - 读 `obj.P` → `Shims.get_P(obj)`；
   - 写 `obj.P = v` → `Shims.set_P(obj, v)`；
   - 复合 `obj.P += v` → `Shims.set_P(obj, Shims.get_P(obj) + v)`（侧效应序保持：obj 表达式提升为临时变量——用 lambda 包装 `((Func<T,R>)(o => { ... }))(obj)` 或逐节点保守冷）；
   - `obj[i]` 同矩阵；事件 `obj.E += h` → `Shims.add_E(obj, h)`；
   - **保守清单**（命中即整文件冷，原因点名）：`ref obj.P`、`out`、`nameof` 之外的取址、属性作 LHS 的解构、`obj.P++`（可转复合，留二期）。
4. **自动属性新增**：访问器对垫片 + M4 背字段 store（`<P>k__BackingField` 命名规约）二件套；初始化器走既有 ctor 重定向。
5. 删除/再编辑侧 M5/注册表键已有访问器粒度（`get_X` 等），对称性免费。

**限制**：新增 virtual/abstract/显式接口属性仍冷（同方法）；`[SerializeField]` 自动属性的 Inspector 不可见（既有限制）。若 C2 落地，第 3 步物化矩阵里的「垫片体只能 public」约束自动消失，但矩阵本身仍必须做。

**DoD**：sidecar golden（属性垫片对的 verbatim）、复合赋值物化 E2E、自动属性+store E2E、保守清单逐项冷断言；自检 P：新增属性经 Probe 读写断言、新增事件订阅断言；N05 改为「新增 virtual 属性」形态保留冷断言。

**规模**：~3-5 天级（物化矩阵是大头）。

## B3. 跨 asmdef 批次验证（验证型）

**现状**：机制理论覆盖（CallerScan 扫 ScriptAssemblies 全体、补丁引用集含全部脚本程序集、原类型解析跨程序集按名），但自检语料全在 Assembly-CSharp，未实证。

**工作**：
1. 自检语料新增 `LocusSelfTestLib/`（含 `.asmdef`，无引用限制）+ Assembly-CSharp 侧调用方。
2. 用例：lib 内体改（热）；lib 方法签名改、调用方在 Assembly-CSharp（跨程序集 M3：核查必须点名调用方文件→同批热）；lib 内新增方法被 Assembly-CSharp 调（跨程序集垫片直调——**预期问题点**：垫片在补丁程序集、self 类型在 lib 程序集，原类型解析与 `original_assembly` 记录需是 lib 名）。
3. 已知风险位：`FindOriginalType` 按 metadataName 全引用集搜索——同名类型在两个 asmdef 时取first？需要按文件→程序集映射消歧（params 里有 per-file assembly 归属？没有则 sidecar 需从 CompileParams 的 source→assembly 映射推导，Unity 侧 `CompilationPipeline.GetAssemblies` 可给）。
4. `.asmdef` 文件本身的增删改保持冷（走 unity_recompile），写进描述。

**DoD**：上述三用例自检全绿 + 同名类型消歧的 sidecar 单测。**规模**：1-2 天（若消歧缺失则 +1 天）。

## B4. unsafe / stackalloc 体修改跟随项目设置

> **状态：已实现（2026-06-13）。** 链路：Unity `ComputeCompileAllowUnsafe()`（Editor+Player 程序集任一 `compilerOptions.AllowUnsafeCode`，随引用路径缓存同生命周期）→ payload `allow_unsafe` + fingerprint 掺入 → Rust `CompileParams.allow_unsafe`（serde default=false 向后兼容）→ sidecar `CompileParamsDto.AllowUnsafe` → `PatchBatch.Build` 与 hotPatch 发射 options 双处 `WithAllowUnsafe`。ProtocolVersion 未动。测试：allowUnsafe=true 下 unsafe 体改热（含补丁执行 E2E）、false 下确定性 CS0227。注意：需要 Unity 插件与 Locus 同时更新才生效（旧插件 payload 无该字段→默认 false，行为与现状一致）。

**现状**：`PatchBatch.Build` 绑定编译 `allowUnsafe: false` 写死；开启 unsafe 的项目体内含 unsafe 构造→绑定/发射期确定性失败（CompileError 而非冷）。

**工作**：CompileParams 透传项目 `allowUnsafeCode`（Unity `CompilationPipeline.GetAssemblies()[].compilerOptions.AllowUnsafeCode`，桥侧 CompileParams 已带或补一个字段+ProtocolVersion 不变向后兼容默认 false）→ `PatchBatch.Build` 与 hot-patch 发射 options 双处 `WithAllowUnsafe(params.AllowUnsafe)`。

**DoD**：sidecar 测试：unsafe 体改在 allowUnsafe=true 下热、false 下确定性诊断；自检不加（测试工程未必开 unsafe）。**规模**：半天。

## B5. 无域重载模式（Enter Play Mode Options）验证矩阵

**现状**：用户高频配置（Reload Domain 关闭）未验证。理论上对我们**更有利**：进 play 不重载域 → edit 态打的补丁存活、`domain_generation` 不变、注册表/镜像连续。

**工作**：
1. 梳理 `domain_generation` 的来源（桥侧生成时机）确认该模式下进/退 play 不变化；
2. 自检加环境探测：读 `EditorSettings.enterPlayModeOptionsEnabled`，在日志中标注当前矩阵分支；
3. 该模式下补一条断言：edit 态补丁进 play 后仍生效（与 E01 衔接：不 revert 直接进 play 验证 8118 仍在，再 revert）；
4. H6 收敛在该模式下 play-exit 触发——确认 `on_play_mode_exited` 的 transition 检测不依赖域重载信号。

**DoD**：两种模式自检全绿（用户两种配置各跑一轮）。**规模**：1 天 + 一轮实机。

## B6. partial 类型支持（兄弟 part 并批）

**现状**：`partial type in file` → 全冷。DOTS/Source Generator/UI Toolkit 工程 partial 普遍，是冷区第二大块（仅次于泛型体）。

**机制**：
1. Sidecar 收到批后，对批内文件中的 partial 类型：经请求新增的 `partialSiblings` 映射（桌面端用类型索引/Roslyn 工作区查同类型其余声明文件）把兄弟 part 文件以「未变更基线文本」身份并入批（oldText==newText，只参与绑定与布局归并，不产出补丁成员）。
2. `CollectTypes` 放行 partial；`DiffType` 对 partial 类型先做**跨 part 归并**（成员字典合并、`InstanceFieldEntries` 以 part 文件名+序为序——**注意**：partial 字段的真实 CIL 布局顺序依赖编译器对 part 的处理序，须用 M4 的构造性布局校验（对照原程序集符号）代替猜测，校验不一致即冷）。
3. PatchRewriter：改名作用于所有 part 的声明（批内每个 part 文件各自产出补丁树，类型在补丁程序集中重新成为多 part 的同名 partial → 合并编译 ✓）。
4. **Source-generator 生成的 part**（不在磁盘上）：从原程序集符号能看到成员但拿不到源 → 凡类型存在「无源 part 成员」（原程序集成员 ∉ 所有磁盘 part）→ 冷，原因点名 generator part。这把 DOTS codegen 档明确划出去，磁盘 partial（手写拆分）划进来。

**DoD**：sidecar：双 part 体改热、part 间字段布局校验、generator-part 检测冷；自检：语料加双 part 类型，体改+字段加各一例。**规模**：~1 周级，B 组最大。

---

## C. DMD 迁移（MonoMod DynamicMethodDefinition, skipVisibility）

目标：用 `DynamicMethodDefinition(patchMethod).Generate()`（底层 `DynamicMethod(restrictedSkipVisibility: true)`）替代「直接 detour 到补丁程序集方法」，使补丁代码**完全绕过 Mono 的 JIT 访问检查**。不含 record 支持（明确出计划）。

### C1. 部分迁移：仅 detour 成员（kept 体）

**改动面**：仅 `LocusBridge.HotReload.cs` 的 `CreateMethodDetour`：
```
original → DMD(patchMethod).Generate() → detour(original, generated)
```
1. 前置：确认 detour bundle（`scripts/build-locus-detour-bundle.mjs`）含 `MonoMod.Utils` + `Mono.Cecil`（DMD 的 Cecil 后端）；缺则扩 bundle（注意 bundle 体积与 TypeIndex 跳过名单不受影响——这些是编辑器程序集）。
2. Generate 失败（IL 边角：filter 异常块、localloc、tail.）→ **逐方法回退**直接 detour（现状路径），日志记数。失败不是错误，是覆盖率统计。
3. 收益立即生效：kept 体引用其它程序集 internal 类型/成员不再 JIT 炸——**当前自检测不出的盲区**（语料全 public）。自检补一例：corpus 加 `internal class LocusSelfTestInternal { public static int V() => …}`…注意 internal 类型在 Assembly-CSharp 内、kept 体也在 Assembly-CSharp 补丁里跨程序集引用它 → 现状首调炸 / DMD 后热。这一例就是 C1 的 DoD 行为断言。
4. `PrepareHotPatchShims` 保留（垫片仍是程序集代码）；`ValidateDetourSignature` 对 DynamicMethod 的参数（DMD 保真复制签名）不变。
5. 风险：DMD 生成的 DynamicMethod 的 `this` 参数化（实例方法复制为静态形态 + 显式 this？MonoMod DMD 保持实例签名）；Unity 旧 Mono 上 DynamicMethod + NativeDetour 组合的稳定性 → 自检全量回归就是验收。

**规模**：桥侧 1-2 天 + bundle 工作 + 一轮实机。**先做这期的另一个理由**：它是 C2 的可行性试金石，失败即止损。

### C2. 全量迁移：垫片/store/调用图重映射（决策点，待 C1 数据）

**目标**：删除「新增成员体只能引用 public 表面」（`FindShimAccessViolation` 移除或降级为提示）与 store/垫片 public 约束。

**难点（为什么不是 C1 的简单推广）**：垫片是**直调**目标（不被 detour）。kept 体的 DMD 副本里 `call Shims.Boost` 仍指向补丁程序集的垫片方法——执行垫片时访问检查照炸。两条路线：
- **R1 调用图重映射**：DMD 复制 kept 体 IL 时，把对本批垫片/store 的 call/ldsfld token 重映射到同批生成的 DynamicMethod/动态容器。需要拓扑序生成（垫片先、调用者后）、跨批引用（注册表垫片）也要换成 Delegate 槽位间接调用 → 等价于把 M2 的"直调物化"改成"委托槽物化"（sidecar 调用点改写为 `LocusShimTable.Get(key)(args)`），sidecar+桥双侧大改。
- **R2 sidecar 槽位化**：sidecar 直接把垫片调用物化为运行时槽表（`Locus.HotReload.Runtime` 加 `ShimTable`：`Func<object[],object>` 或强类型泛型槽），桥把每个垫片 DMD 化后填槽。开销：每次新增成员调用一次委托 + 装箱（强类型槽可免）。**R2 改动集中、跨批连续性天然（槽按 MemberKey 永久寻址，替代「旧垫片 detour 新垫片」机制）**，是更优路线，但等价于重做 M2 的物化层。

**决策输入**（C1 实机后收集）：DMD 在目标 Unity 版本的生成成功率、性能（DMD 生成耗时/调用开销）、实机稳定性；以及 N12 类需求的真实频度（用户多久撞一次 private 墙）。

**规模**：R2 路线 ~1.5-2 周级。明确**不在本计划内启动**，C1 验收后单独立项。

---

## 排期总览

| 期 | 项 | 规模 | 解锁 |
|---|---|---|---|
| 1 | B4 unsafe ✅ 已实现 | 0.5d | unsafe 工程体改 |
| 1 | A 组实机回归 | 0.5d | 基线确认 |
| 2 | B1 泛型体 ✅ 已实现（实机自检待跑） | 2-3d | 最大冷区 |
| 3 | C1 部分 DMD | 1-2d+实机 | internal 盲区 + C2 试金石 |
| 4 | B2 属性/事件/索引器新增 | 3-5d | 第二大新增需求 |
| 5 | B3 + B5 验证矩阵 | 2-3d | 真实工程形态 |
| 6 | B6 partial | ~1w | DOTS/SG 工程 |
| 7 | C2 决策点 | — | 凭 C1 数据 |
