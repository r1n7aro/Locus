# Unity 原生 Locus View

Windows x64、Unity 6.5 编辑器使用 `locus_unity_native.dll` 创建 WebView2 子窗口。一个 Unity 项目共享一个 Environment、一个隐藏的 Vue 主宿主和一个 `Library/Locus/WebView2/Shared` profile。网页直接显示在原生 HWND 中，不进入 Unity 纹理管线。

## 打开和嵌入

### 完整 Locus 工作台

`Window/Locus` 和 Locus 的“在 Unity 打开会话”使用直接创建的 WebView2 Controller。Controller 由 Locus 主线程管理，以 Unity 的 ContainerWindow HWND 为父窗口；不再创建、改写样式或注入 Tauri 顶层窗口，也没有浮层回退路径。

完整工作台复用 Locus 主窗口的 Environment、Vue/Pinia、编译缓存和现有 Tauri 命令/事件。关联子文档通过 Teleport 承载 `DevelopmentWorkbench`，每个窗口保留独立的窗口标识、工作区绑定和文档事件目标。输入、菜单、快捷键及 diff 弹层使用所属文档，嵌入主题继续使用现有 Unity 主题设置。

当前最多同时打开 4 个完整工作台，保留最多 2 个空闲 Controller。Unity 关闭父窗口前通过控制管道确认 WebView 已移到隐藏停放窗口，等待上限为 250 ms；普通关闭随即卸载 Vue、清理监听和面板绑定，确认完成后进入空闲池。再次打开创建新的业务作用域，复用原生文档及未变化的 CSS；空闲 30 秒后回收。前端释放确认另有 2 秒超时和数量上限，异常时强制关闭。

Domain Reload 保留浏览器文档；停靠和编辑器重启前的迁移同样使用 Controller 的 ParentWindow API。旧窗口实例迟到的控制消息会被丢弃。主 Locus 窗口仍在使用的 Vue/Environment 会继续运行，关闭嵌入窗口只回收新增的控件资源。

P3 补充：Unity 窗口显示连接、容量等待、加载超时和恢复状态，提供重新连接与折叠诊断；状态查询和正常生命周期不输出日志。第五个窗口等待名额，关闭已有窗口后自动继续。加载超过 30 秒显示重试；父 HWND 连续失效 5 秒后回收。renderer 崩溃自动恢复每分钟最多 3 次；完整工作台的共享 browser 进程退出时明确提示重启 Locus。Inspector 独立 Environment 的 renderer/browser 故障均可重建宿主，普通单个 View 重试只重建该控件。

共享工作台草稿增加版本化 sessionStorage 恢复缓存，最多 64 条、约 1 Mi 字符，以 200 ms 合并写入；用于 renderer 重载恢复。超过缓存预算的草稿仍可正常编辑，但仅保留内存副本；崩溃前尚未写入的最后 200 ms 和浏览器整体退出不承诺恢复。工作区布局继续使用现有存储。文件拖入通过 WebView2 原生 File 对象取得路径，校验当前文档和 generation 后进入已有工作台拖放流程，不接受网页 JSON 声明的文件路径。

控件记录所属窗口 Repaint 的 DPI，后台更新不再读取其他窗口的缩放比例。常见 Unity Object Selector、Color Picker 和独立 PopupWindow 与控件重叠时自动协调隐藏；同面板和第三方自定义浮层仍使用 `SetOverlayVisible`。完整 P3 自动化和人工检查步骤见 [`P3-ACCEPTANCE.md`](../scripts/tests/unity-native-view/P3-ACCEPTANCE.md)。

完整工作台的可重复验收见 [`EMBED-WORKBENCH.md`](../scripts/tests/unity-native-view/EMBED-WORKBENCH.md)。隔离 Unity 6.5 的最终 1,000 次循环为 1019/1019 通过；循环全程仅 2 次原生创建，打开耗时 P95 为 157 ms，空闲池自然回收后资源预算通过。物理 IME、混合 DPI 与完整系统拖放矩阵仍需 P3 验收。

完整工作台和下面的 Inspector 控件具有不同的进程归属：前者共享 **Locus 进程**中的主宿主，后者共享 **Unity 进程**中的 P2 宿主。二者都直接使用 WebView2 显示，不使用 Unity 纹理；不跨进程共享 COM 对象或 Vue 实例。

### 自定义 View 控件

在 Locus 对 View 执行“在 Unity 中打开”，或调用 SDK `views.native.open`，会把工作区绑定的连接送入目标 Unity，并打开可停靠的 `LocusNativeViewWindow`。再次打开同一个 View 复用窗口；选中对象通过 `binding.targets` 更新。

在同一 Unity 项目内，Inspector 可以取得该连接并复用控件：

```csharp
using Locus;
using UnityEditor;
using UnityEngine.UIElements;

public override VisualElement CreateInspectorGUI()
{
    if (!LocusViewConnection.TryGet("project/my-view", out var connection))
        return new Label("请先从 Locus 在 Unity 中打开 View。");

    // 每个 Inspector 窗口自动解析自己的 owner；key 在该窗口内必须唯一。
    var view = new LocusWebViewElement(
        "my-inspector-view", connection, LocusViewContext.FromObjects(target));
    view.style.height = 260;
    return view; // Detach 时自动释放逻辑绑定。
}
```

`EditorWindow` 可使用 `new LocusWebViewElement(this, key, connection, context)` 明确指定 owner。改变对象调用 `SetContext`，关闭自建窗口时调用 `Dispose`。连接只适用于 Locus 授权的当前工作区；新的 View 或过期连接应从 Locus 再次打开。不要把连接 URL 写入项目资源、日志或可分享文件。

IMGUI 使用 `LocusWebViewGUI(owner, key, connection, context)`，每次 `OnGUI` 调用 `Draw(rect, screenClip)`，在 `OnDisable` 调用 `Dispose`。`rect` 使用当前 GUI 坐标；`screenClip` 使用屏幕点坐标，必须包含调用者的全部滚动、分组和裁剪范围。UI Toolkit 自动处理 ScrollView 和计算后的矩形裁剪，也可通过 `ClipProvider` 补充面板坐标下的裁剪区。

原生子窗口有独立的窗口层级。显示同面板 Unity 浮层时调用 `SetOverlayVisible(true)`，收起后传 `false`。不支持旋转、透明叠加、任意圆角蒙版。Tab 焦点通过 WebView2 与包装控件交接；IMGUI 可订阅 `FocusLeaving` 完成自定义焦点布局。页面内部处理文本输入和剪贴板，Ctrl+S 转交 Unity。

## View 代码

```vue
<script setup>
import { onMounted, ref } from 'vue';
import { property, useViewContext } from '@locus/view-runtime';
const context = useViewContext();
const target = { ...context.binding.targets[0], propertyPath: 'm_Name' };
const snapshot = ref(null);
onMounted(async () => { snapshot.value = await property.readProperty(target); });
// property.write(target, value) 沿用现有 Unity 属性和撤销服务。
</script>
```

`binding` 包含 `targets`、`data`、`instanceId` 和 `generation`；`data` 来自 C# 的 `LocusViewContext.dataJson`。对象切换会卸载旧执行作用域再建立新作用域。View 编译缓存、Vue 和组件库由主宿主共享，业务状态、计时器、观察器、事件和自定义 window 属性按绑定隔离。通过 View API 发起的请求跟随绑定取消，已提交到 Unity 的操作不会因取消 HTTP 等待而自动回滚。

当前通道支持 View 读取/热更新/脚本、工作区文件、View storage/日志、Unity 属性/资源和工作区事件。直接调用 Tauri API、桌面窗口管理、Chat/Session/LLM 类 SDK 命令不在此原生通道的 P2 范围内，会明确返回不支持；不要在 View 内假设 `__TAURI_INTERNALS__` 存在。

## 生命周期和诊断

- 同时最多 4 个可见绑定、2 个空闲控制器；其余控件等待空位。
- 普通关闭先暂停更新，并留出 250 ms 的树重建交接期；未被同一控件接回则卸载业务作用域。空闲控制器暂停并在 30 秒后回收；最后一个绑定关闭后，主宿主和 Environment 也在 30 秒后整组退出。
- UI 重建留有短暂的身份交接时间，程序集重载以原 owner/key 和 generation 恢复。隐藏、对象切换与真正关闭的处理不同。
- 在 Unity 主线程调用 `LocusNativeViewAutomation.Diagnostics()` 可获取运行时版本、浏览器 PID/进程组、原生父子窗口、bounds 和队列计数；`BindingDiagnostics()` 提供不含连接令牌的绑定状态。
- 缺少 DLL 时提示更新 Unity 插件；缺少 WebView2 Runtime 时提供官方安装入口。使用 Evergreen Runtime，更新后关闭整个浏览器组或重启 Unity 生效，遵循 [微软分发说明](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/distribution)。
- Locus 重启后，从 Locus 再次打开 View 更新连接；浏览器故障可用控件的“重试”重建共享宿主。

原生 DLL 随现有 `bun run unity:bundle-native` / `bun run unity:bundle` 分发；无需额外拷贝独立测试 DLL。产品安装链路仍使用 Locus 的 Unity 插件更新流程。

## 验证

隔离验收入口及前置构建见 [`scripts/tests/unity-native-view/README.md`](../scripts/tests/unity-native-view/README.md)。P2 自动化覆盖真实属性、Undo、Prefab、绑定切换、控件数量、整组回收、程序集重载和锁定 Inspector。物理 IME 候选窗、混合 DPI/多显示器、所有停靠与第三方浮层组合、1,000 次以上长时间 CPU/GPU 稳定性属于 P3 发布验收。
