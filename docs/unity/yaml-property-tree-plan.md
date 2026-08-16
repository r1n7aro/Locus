# Unity YAML 渐进式 Property Tree 方案

## 目标

`unity_yaml_read`、`unity_yaml_search`、前端 Unity 数据查看与编辑统一使用现有
`UnitySerializedPropertySnapshot` / `UnitySerializedPropertyTarget` 基础设施。工具层只负责：

- 将 `Assets/Foo.asset/field/items/3` 解析成 Unity 对象 target 与 property path；
- 调用统一 Property Tree read/discover；
- 将结构化节点渲染成适合模型读取的紧凑树文本。

统一后端保留前端既有的编辑元数据、binding target、枚举、ObjectReference、
SerializeReference、数组和写回能力。YAML 磁盘解析与 Unity Editor 实时读取是同一节点协议的两个数据源。

## 对外 DSL

### Read

```json
{
  "path": "Assets/Actions/LightNormalAttack1.asset/hitTrack/clips/4",
  "depth": 2
}
```

- `path` 同时包含 Unity Asset path 与内部 Property Tree path。
- Unity 已连接时先读取 Editor 内存态；仅在实时协议不可用或目标无法实时解析时读取磁盘 YAML。
- 每次 Read 从请求深度开始渐进探测完整投影；紧凑树超过 4,000 字符后回到请求深度的大纲。
- 完整输出不超过 4,000 字符时直接返回全部非数组后代；该规则同样适用于显式传入 `depth` 的调用。
- 超过阈值后，`depth` 表示从目标节点继续展开的层数；目标节点为 0，直接子节点为 1。默认 `depth=2`，硬限制为 4。
- 所有分支中的数组单次最多展开 4 项，任意索引仍可通过完整 path 直接读取。

### Search

```json
{
  "path": "Assets/Actions/LightNormalAttack1.asset/hitTrack",
  "query": "AttackWindow",
  "match_fields": "path,field_name,field_value,type",
  "limit": 50
}
```

Search 与 Read 共用完整 path parser 和节点索引。每个搜索结果的首行都是可直接传给 Read 的完整 path。

`match_fields` 是逗号分隔的 `path,field_name,field_value,type` 子集，省略时搜索全部四类信息。

- `path` 命中按分支保留最浅节点；祖先 path 已命中后，后继节点因完整路径继承查询文本产生的命中会被抑制。
- `field_name`、`field_value` 与 `type` 继续扫描完整搜索范围，每个命中节点独立返回。
- 输出按 `Path matches`、`Field matches`、`Type matches` 分区。字段结果明确标注命中的字段名；字段值结果同时给出字段名与紧凑值；类型结果给出实际类型。
- `limit` 在浅层 path 去重后计数，额外探测一项以提供明确的截断提示。

### 路径分割

从请求 path 中选择最长、真实存在且扩展名受支持的 Unity Asset 文件前缀。剩余部分按 `/` 分段：

```text
Assets/Actions/LightNormalAttack1.asset  # asset path
hitTrack/clips/4                        # semantic suffix
```

内部字段名采用 JSON Pointer 转义：`~0` 表示 `~`，`~1` 表示 `/`。数组索引使用独立数字段。

Scene、Prefab 与 ScriptableObject 使用同一语法：

```text
Assets/Actions/Attack.asset/hitTrack/clips/4
Assets/Prefabs/Weapon.prefab/HitBoxAuthoring/data
Assets/Prefabs/Weapon.prefab/Blade/HitBoxAuthoring/data
Assets/Scenes/Arena.unity/Player/HitBoxAuthoring/data
```

- Prefab 路径已包含根 GameObject，根组件直接位于 asset 下；子 GameObject 继续作为子段。
- Scene 在 asset 后保留顶层 GameObject 与后续子 GameObject。
- 同名兄弟节点使用 `Name[2]`、`Name[3]`；组件同名时采用相同规则。
- `GameObject` 是 Prefab/Scene 对象的序列化字段节点，组件按组件类型命名。

## 输出格式

输出使用统一 Property Tree 紧凑树语法。根节点显示一次完整 path，子节点只显示相对名称：

```text
Assets/Actions/LightNormalAttack1.asset (PlayerAttackAction)
├─ hitTrack (HitTrack)
│  └─ clips [6] (List<HitBoxClip>)
│     └─ … +6
├─ bakedRootMotion [116]
│  └─ … +116
└─ swordWind (SwordWindData)
   └─ frames [86]
      └─ … +86

--- Subassets [3] ---
  HitTrack (HitTrack)
  ├─ HitBoxClip (HitBoxClip)
  └─ HitBoxClip[2] (HitBoxClip)
```

移除独立的版本、revision、document 数量、root 数量和数组限制元数据块。短树完整输出时不带折叠标记；大树到达 depth 边界的复合节点保留类型与计数。数组标题只显示总数与元素类型，省略量固定作为数组的最后一个子节点输出。

复合资产根节点在普通字段树之后输出 `Subassets` 分节。该分节是同文件 Unity Object 的可寻址归属树，
不参与普通属性的 `depth` 计数。归属从主对象出发，仅遍历 `SerializedProperty` / YAML 中实际存在的同文件
ObjectReference；目标第一次到达的位置成为父节点，数组保持索引顺序，后续共享引用和祖先回引不重新归属。
普通 CLR 字段、属性和运行时缓存不参与关系推导。每个树节点都是可追加到父路径后的相对段；重复名采用
`Name[2]`、`Name[3]`，空名称采用 C# 类型名。总节点超过 32 项时在分节末尾输出 `… +N`，Search 仍扫描全部对象。

Vector2/3/4、Quaternion、Color、Rect、Bounds、Curve、Gradient 等语义原子始终在节点行内显示，内部
`x/y/z/w` 或 keyframe 实现字段不消耗 `depth`。Hierarchy、Parent、World Position、World Rotation、
World Scale、Prefab 来源等派生信息没有可直接读取的 Property Tree path，因此只在根节点之后输出独立分节：

```text
Assets/Scenes/Arena.unity/Player/Transform (Transform)
├─ Local Rotation: 0, 0, 0
├─ Local Position: 1, 0, 2
└─ Local Scale: 1, 1, 1

--- Transform ---
  World Position: {x: 1, y: 0, z: 2}
  World Rotation: {x: 0, y: 0, z: 0}
  World Scale: {x: 1, y: 1, z: 1}
```

## 统一 Property Tree 模型

现有 `UnitySerializedPropertySnapshot` 继续作为唯一节点模型。增加可选语义字段：

```text
semanticPath       节点完整模型路径
nodeKind           asset/scene/hierarchy/object/component/property/array/item/reference
canonicalPath      共享或循环引用第一次出现的位置
referenceTarget    ObjectReference 指向的 UnitySerializedPropertyTarget
visibleChildCount  当前节点实际子项数量
childrenTruncated  当前快照是否折叠了子项
displaySections    无直接 path 的只读语义分节
subassets          同文件可寻址 Unity Object 的序列化归属树
```

字段全部可选，旧 Unity 插件和旧前端数据可以继续反序列化。前端编辑继续使用 `bindingTarget.propertyPath` 写回；
Agent 工具使用 `semanticPath` 定位和呈现。

实现分层如下：

```text
前端编辑器  src/services/propertyTree.ts
     │      UnitySerializedPropertySnapshot / Target
Rust 桥接    src-tauri/src/unity_serialized_property.rs
     ├─ 实时源 property_tree_read / discover / write / apply
     └─ 磁盘源 unity_serialized_property/property_tree.rs
Unity 端     locus_unity/Editor/LocusBridge.PropertyTree.cs
             locus_unity/Editor/LocusBridge.SerializedProperties.cs
```

前端查看、编辑、Agent read/search 和压缩后的工具结果恢复共享同一 Snapshot/Target 契约。旧
`view_binding_*` 入口仅作为协议兼容别名，内部转发到 `property_tree_*`。

`unity_execute` 同时公开相同格式化器：

```csharp
var go = GameObject.Find("Player");
print(ctx.PropertyTree(go, depth: 2, maxArrayItems: 4));

// 无 ExecuteCodeContext 时使用静态入口。
print(LocusPropertyTree.Format(go, depth: 2, maxArrayItems: 4));
```

SDK 直接读取传入的当前 Unity 对象，使用相同紧凑值、深度、数组限制和只读语义分节。

## 数据源

### Unity Editor 实时源

复用 `unity_serialized_property::read/discover` 与 `LocusBridge.SerializedProperties.cs`：

- `SerializedObject.GetIterator()` 提供真实 Property Tree 顺序；
- `SnapshotSerializedProperty` 提供值、类型、编辑状态和 children；
- ObjectReference 快照增加本地 fileID 与 reference target；
- Agent read 固定传入 `maxArrayItems=4`，前端继续使用自己的显示上限。
- Scene/Prefab 的精确路径先解析最长 GameObject 前缀，再进入组件与 SerializedProperty。
- Scene 根路径解析为当前已加载 `Scene`，直接生成可寻址 GameObject 层级；不会把 `SceneAsset` 当成空对象返回。
- Scene 根搜索在 Unity 端过滤实时层级，只回传命中节点及 asset-qualified path。
- GameObject 的 Hierarchy、World Transform、Prefab 来源等无直接 path 的派生信息通过 `displaySections` 单独输出。
- 主 `.asset` 快照从 `AssetDatabase.LoadAllAssetsAtPath` 枚举对象，再按完整 `SerializedObject` 引用顺序构建实时 Subasset 归属树；每项 target 保留本地 fileID，模型路径只暴露稳定语义段。
- 根级 discover 在 Unity 端按归属树逐个过滤主对象与 Subasset 的 `SerializedObject`，命中嵌套路径可直接交给 Read，避免传输完整复合资产。

### YAML 磁盘源

Unity Editor 不可用时，YAML adapter 产生相同 `UnitySerializedPropertySnapshot`：

- YAML mapping 对应 Generic property；
- YAML sequence 对应 Array property；
- scalar 对应现有 SerializedPropertyType；
- `{fileID}` / `{guid,fileID}` 对应 ObjectReference；
- managed-reference registry 对应 ManagedReference；
- `bindingTarget` 保留 asset、targetFileId 与 propertyPath。
- Scene 根通过 GameObject、Transform 与 `m_GameObject` 关系合成层级；Prefab 将唯一根对象展平。
- 非层级复合资产按 YAML 中实际序列化的同文件引用构建 `subassets` 归属树；没有序列化所有者的 document 成为额外根节点。

## C# 定义与父子归属

父子归属由 C# serialized field definition 和 YAML/SerializedObject 当前值共同确定。

1. `m_Script.guid` 定位 C# 类型。
2. 磁盘 adapter 使用 tree-sitter C# parser 解析 public field、`SerializeField`、`SerializeReference`、字段类型和声明顺序。
3. YAML 中已经固化的 SerializedProperty 顺序作为实例遍历顺序；C# 定义负责字段绑定、类型标注和 `FormerlySerializedAs` 语义名迁移。
4. mapping 与 inline serializable object 成为普通 property 子树，数组/List 成为带数字索引的子树。
5. 同文件 ObjectReference 的实际 fileID 在首次出现的序列化字段位置展开，字段声明类型用于节点类型标注。
6. 外部 GUID 保持外部资产引用；其显示值为外部 asset path 与 fileID。

实时源以 Unity 已编译类型和 `SerializedObject.GetIterator()` 为权威，因此自然覆盖继承字段、第三方 drawer
对应字段与当前未保存值。磁盘源保留 YAML 的完整实例顺序，并用 C# schema 补充语义，不丢失继承或第三方字段。

Property Tree 从主对象按 Unity property 顺序遍历，数组按索引升序。每个实例身份首次出现的位置登记为 canonical path：

```text
Unity Object identity       = (asset path, target fileID)
Managed reference identity = (owner target, managed reference id)
```

后续共享引用和循环引用输出 `→ canonical path`，停止递归。YAML document 物理顺序不参与父子归属。

磁盘源在第一次 read 前对完整实例树预索引 canonical path，并按 asset 修改时间缓存；它与本次请求 path、depth
和数组四项预览无关。实时源完全依据当前 Editor 对象图登记本次遍历中第一次出现的位置，读取前不会接触磁盘 YAML。

主对象优先采用 Unity 标准主 ScriptableObject fileID `11400000`，随后按同文件引用入度选择未被其他 document
持有的对象，最后采用第一个 document 作为稳定根。一次遍历中的第一个序列化定义位置固定为 canonical path，
循环不会改变该位置。非层级 `.asset` 的 Subasset 分节包含所有非主对象；从主对象按序列化顺序首次到达的
Group→Track→Clip 形成嵌套路径，没有所有者的对象作为额外根。Read 可从任意目录节点直接进入对象，
根级 Search 按相同嵌套路径逐个扫描主对象与全部 Subasset，因此覆盖文件中的全部 document，且不会沿对象引用重复递归。

## 工具兼容

- 对外工具仅保留 `unity_yaml_read` 与 `unity_yaml_search`；历史会话中的 list 调用结果只参与上下文恢复。
- `file_path + object_path` 转译为完整 `path`。
- `path_prefix` 转译为 search `path`。
- `max_field_depth` 转译为 `depth`。
- `max_array_items` 固定为 4；4,000 字符以内的完整分支也保留四项预览和末尾省略量。
- `detail=document` 退出公开协议；原始字段通过精确 Property Tree path 获取。

## 预算

- Read 的完整分支采用 4,000 字符硬阈值，投影过程中逐节点精确累计最终树文本；即将超限时立即终止并回到大纲分支。
- 完整分支不受 `depth` 限制，数组仍受四项硬限制；共享引用与循环引用在 canonical path 处停止。
- 大纲分支默认展开 2 层，硬上限 4 层，单个数组最多返回 4 项。
- 单次文本在完整节点边界截断。
- 同一模型工具轮次的 YAML read/search 结果使用总预算，公平分配给并行调用。
- 单次文本默认最多 16,000 字符；同轮多个 YAML 调用共享 48,000 字符预算。
- 磁盘 Property Tree 按绝对路径、修改时间与文件长度复用；最多保留 8 棵完整树，canonical path 轻量索引最多保留 64 份。

## 验证

- 多 document ScriptableObject 首次读取只返回有界 Property Tree。
- 单 document asset 的完整紧凑树不超过 4,000 字符时直接返回全部 Property Tree。
- C# `T`、`List<T>`、`T[]`、`SerializeReference` 与 `FormerlySerializedAs` 均能绑定。
- 共享引用与循环引用固定指向第一次出现的 canonical path。
- 数组第 5 项及后续项可按完整 path 直接读取；所有树输出最多预览前 4 项。
- search 返回的每个 path 均可原样交给 read。
- 主资产 Read 在独立 `Subassets` 分节按首次序列化引用列出全部同文件对象；循环、共享引用、同名、空名和 32 项以上目录保持稳定且有界。
- Scene/Prefab 使用同一 asset-qualified path，Prefab 根组件路径无需重复根 GameObject 名。
- 前端 Property Tree 编辑的 read/write/apply 契约保持兼容。

## 实际项目验收输出

以下片段由当前实现直接读取 Unity 6000.5.6f1 测试项目生成。

复合 `PlayerAction` 主资产在普通字段树之后列出 13 个实时 Subasset；序列化的 Group、Track、Clip
引用形成嵌套路径，空名 Clip 使用 C# 类型名，重复 `EffectTrack` 在同级使用稳定序号：

```text
--- Subassets [13] ---
  New Group (Group)
  ├─ AnimacerTrack (AnimacerTrack)
  │  └─ AnimacerClip (AnimacerClip)
  ├─ HitTrack (HitTrack)
  │  └─ HitBoxClip (HitBoxClip)
  ├─ EffectTrack (EffectTrack)
  │  └─ ParticleSystemClip (ParticleSystemClip)
  ├─ EntityTrack (EntityTrack)
  │  └─ EntityIgnoreEntityWallClip (EntityIgnoreEntityWallClip)
  ├─ ActionStageTrack (ActionStageTrack)
  │  └─ ActionStageClip (ActionStageClip)
  └─ EffectTrack[2] (EffectTrack)
     └─ CameraDirectionBlurClip (CameraDirectionBlurClip)
```

根级 Search 命中的目录路径可原样交给 Read：

```text
Assets/Assets/Entity/Player/Light/Skill/01_蓄力突刺/EloraSkill蓄力突刺-完美.asset/New Group/EntityTrack (EntityTrack)

Assets/Assets/Entity/Player/Light/Skill/01_蓄力突刺/EloraSkill蓄力突刺-完美.asset/New Group/EntityTrack (EntityTrack)
├─ Script: "EntityTrack (Assets/Scripts/Battle/Entity/Actions/Runtime/ProjectB/Tracks/Entity/EntityTrack.cs)"
├─ actionClips [1] (ActionClip)
│  └─ 0 (ActionClip) …
└─ color: #FFFFFFFF
```

短材质 `Assets/Art/Props/StoneWall/Stone_Wall_Material.mat` 的紧凑树低于 4,000 字符，首次 Read
直接返回全部非数组字段；`m_TexEnvs` 等数组显示前 4 项并在最后标记省略量。

`LiberationSans SDF.asset` 的完整投影在第 4,001 个字符前终止，随后直接生成默认两层大纲；最终大纲为
3,461 字符，其中 250 项 character table 保持四项预览：

多 document Addressables 模板首次读取：

```text
Assets/AddressableAssetsData/AssetGroupTemplates/Packed Assets.asset (AddressableAssetGroupTemplate)
├─ m_SchemaObjects [2] (AddressableAssetGroupSchema)
│  ├─ 0 (AddressableAssetGroupSchema) …
│  └─ 1 (AddressableAssetGroupSchema) …
├─ m_Description: "Pack assets into asset bundles."
└─ m_Settings
```

搜索子 document 字段并原样交给 read：

```text
Assets/AddressableAssetsData/AssetGroupTemplates/Packed Assets.asset/m_SchemaObjects/0/m_BundleMode: 0 (Integer)

Assets/AddressableAssetsData/AssetGroupTemplates/Packed Assets.asset/m_SchemaObjects/0/m_BundleMode: "Pack Together"
```

250 项 TMP character table 只展开四项，search 仍可命中第 5、6 项：

```text
Assets/Addons/TextMeshPro/Resources/Fonts & Materials/LiberationSans SDF.asset/m_CharacterTable [250] (TMP_Character)
├─ 0 (TMP_Character)
├─ 1 (TMP_Character)
├─ 2 (TMP_Character)
├─ 3 (TMP_Character)
└─ … +246

Assets/Addons/TextMeshPro/Resources/Fonts & Materials/LiberationSans SDF.asset/m_CharacterTable/4/m_Unicode: 36 (Integer)
Assets/Addons/TextMeshPro/Resources/Fonts & Materials/LiberationSans SDF.asset/m_CharacterTable/5/m_Unicode: 37 (Integer)
```

Scene 搜索返回完整可读路径：

```text
Assets/Assets/ECSPrototype/Scenes/EloraECSPrototype.unity (Scene)
├─ ECS Prototype (GameObject)
│  ├─ EloraECSPrototype (EloraEcsPrototypeBridge, EcsPrototypeInputBridge) [Inactive, Layer:Ground] …
│  ├─ Ground (MeshFilter, MeshCollider, MeshRenderer) [Layer:Ground]
│  ├─ Main Camera (Camera, CinemachineBrain) [Tag:MainCamera]
│  └─ KalanECSPrototype (EloraEcsPrototypeBridge, EcsPrototypeInputBridge) [Layer:Ground] …
├─ ECS Prototype Entities SubScene (SubScene)
└─ Virtual Camera (Camera) [Inactive]

--- Scene ---
  Active: true
  Loaded: true
  Dirty: false
  Root GameObjects: 3

Assets/Assets/ECSPrototype/Scenes/EloraECSPrototype.unity/ECS Prototype/EloraECSPrototype/TargetDummy (MeshFilter, MeshRenderer) [Layer:Friend]
Assets/Assets/ECSPrototype/Scenes/EloraECSPrototype.unity/ECS Prototype/KalanECSPrototype/TargetDummy (MeshFilter, MeshRenderer) [Layer:Friend]
```

Prefab 内部共享引用回写为 canonical path：

```text
Assets/Prefabs/Gameplay/CameraSystem.prefab (GameObject)
├─ CameraManager (CameraManager)
│  ├─ mainCamera (Camera) …
│  ├─ freeLookVCam (MonoBehaviour) …
│  └─ impulseSource (MonoBehaviour) …
├─ Main Camera (GameObject)
│  └─ Camera → Assets/Prefabs/Gameplay/CameraSystem.prefab/CameraManager/mainCamera
├─ FreeLook_VCam (GameObject)
│  └─ MonoBehaviour → Assets/Prefabs/Gameplay/CameraSystem.prefab/CameraManager/freeLookVCam
└─ ImpulseSource (GameObject)
   └─ MonoBehaviour → Assets/Prefabs/Gameplay/CameraSystem.prefab/CameraManager/impulseSource
```
