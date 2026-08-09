---
summary: >-
  用于 Unity 6.3+ 图形与渲染问题诊断：通过内置 Frame Debugger 快速检查事件、Pass、渲染状态和 Render Target，或通过后台 RenderDoc 截帧分析 GPU action、Compute Buffer、资源生命周期、中间纹理和管线状态。根据问题深度选择通道，所有 Unity 操作通过 `unity_execute` / `unity_run_states` 编排。
tools:
  - unity_execute
  - unity_run_states
  - bash
  - read
---

# Unity Graphics Debugger

## 选择通道

- 默认从 `FrameDebuggerApi` 开始：定位渲染顺序、Draw/Dispatch、Shader/Pass、Mesh、Render Target 和常规渲染状态，启动快、结果直接来自 Unity。
- 需要实际 GPU replay、完整 action 树、Compute/UAV Buffer 字节、指定事件的管线快照、资源首次写入/后续读取关系或任意中间纹理时，使用 `RenderDocCaptureApi` 与 `locus_renderdoc`。
- 可以先用 Frame Debugger 缩小事件和相机范围，再关闭 Frame Debugger，以相同场景状态截取 RenderDoc 帧。
- 同一轮分析只保持一个捕获后端处于活动状态。切换前调用 `FrameDebuggerApi.Disable()`，或完成 RenderDoc `End()` / `Discard()`。

## 通用流程

1. 明确 Game 或 Scene 视图、相机、触发条件，以及需要回答的问题。
2. 选择最轻量的后端并获取有界事件列表；先过滤，再读取少量事件详情。
3. 导出纹理或 Buffer 时保留事件索引/EID、资源 ID、输出路径和选择器。
4. 在 Locus Python 中直接导入包模块，返回 `dict`、`list`、`bytes`、数字等 Python 类型进行分析。
5. 在成功、失败、超时和取消路径中清理捕获状态。

## Frame Debugger

### C# API

- `Status() -> FrameDebuggerStatus`
- `CaptureAsync(int timeoutMs = 15000, CancellationToken cancellationToken = default) -> Task<FrameDebuggerStatus>`
- `Events(FrameEventQuery) -> FrameEventListResult`
- `Event(int index, FrameEventOptions) -> FrameEventDetail`
- `Select(int index) -> FrameDebuggerStatus`
- `ExportRenderTargetAsync(int index, FrameTextureExportOptions, CancellationToken) -> Task<FrameTextureExportResult>`
- `Disable() -> FrameDebuggerStatus`
- `Json(object) -> string`

捕获 Editor 本地目标时 Play Mode 会暂停在当前帧。事件索引为零基；Unity Frame Debugger 的 event limit 等于 `index + 1`。

```csharp
using Locus.Skills;

var capture = await FrameDebuggerApi.CaptureAsync(timeoutMs: 15000);
var events = FrameDebuggerApi.Events(new FrameEventQuery {
    TypeContains = "Draw",
    MaxEvents = 80
});
print(FrameDebuggerApi.Json(new { capture, events.Count, events.Events }));
```

读取单个候选并导出当前 Render Target：

```csharp
var chosen = events.Events.Count > 0 ? events.Events[events.Events.Count - 1] : null;
if (chosen == null) throw new Exception("No matching frame event.");

var detail = FrameDebuggerApi.Event(chosen.Index, new FrameEventOptions {
    IncludeRenderState = true,
    IncludeShaderProperties = true,
    MaxShaderProperties = 32
});
var exported = await FrameDebuggerApi.ExportRenderTargetAsync(
    chosen.Index,
    new FrameTextureExportOptions { Format = FrameTextureFormat.Png });
print(FrameDebuggerApi.Json(new { chosen, detail, exported }));
```

`FrameTextureExportOptions` 支持：

- `Format`：`Png`、`Exr`、`Tga`
- `OutputDirectory`：绝对路径或项目相对路径，默认 `Library/Locus/FrameDebugger`
- `RenderTargetIndex`：MRT 从 `0` 开始；深度为 `-1`，Stencil 为 `-2`
- `Channels`：`RGBA`、`RGB`、`R`、`G`、`B`、`A`
- `BlackLevel`、`WhiteLevel`、`FlipY`、`WriteMetadata`

导出使用 Unity Frame Debugger 的 Render Target 可视化材质处理 backbuffer、MSAA、深度、Cube 与 Texture Array；可用时以 EXR 保留线性/HDR 数据。完成后始终调用 `FrameDebuggerApi.Disable()`。

### Python 纹理分析

```python
import locus_texture_analysis as lta

report = lta.analyze_texture(exported_texture_path)
comparison = lta.compare_textures(before_path, after_path)
```

`analyze_texture` 返回尺寸、格式、通道统计、分位数、亮度、透明度、截断比例、熵、边缘密度和颜色数量；`compare_textures` 返回 MAE、RMSE、PSNR、最大误差与变化像素比例。PNG/TGA 直接支持；EXR 取决于 Locus Python 图像运行时的 EXR codec。

## RenderDoc

### 初始化与后台约束

选中 Skill 时注入内容包含只读 `Root`。每个 Unity 进程先初始化一次：

```csharp
using Locus.Skills;

var init = await RenderDocCaptureApi.InitializeAsync(@"<Root>");
print(RenderDocCaptureApi.Json(init));
```

`InitializeAsync` 校验 Unity 版本、Windows x64、`renderdoc.dll`、`qrenderdoc.exe` 和 Python 模块。首次加载 runtime 会重建 Unity 图形设备。进程中已有其他路径的 `renderdoc.dll` 时返回 `renderdoc_module_conflict`；重启 Unity 后先初始化本 Skill。

捕获使用 RenderDoc in-app API、精确视图 HWND 和关闭的 overlay，不启动 qrenderdoc UI。不要调用 `UnityEditorInternal.RenderDoc.BeginCaptureRenderDoc`、`EndCaptureRenderDoc`、`LaunchReplayUI`，也不要直接启动 `qrenderdoc.exe`。

### C# API

- `InitializeAsync(string root) -> Task<RenderDocInitializeResult>`
- `CaptureOnceAsync(RenderDocBeginCaptureOptions, int maxEditorUpdates = 16) -> Task<RenderDocCaptureOnceResult>`
- `Status() -> RenderDocCaptureStatus`
- `Begin(RenderDocBeginCaptureOptions) -> RenderDocBeginCaptureResult`
- `Trigger() -> RenderDocTriggerCaptureResult`
- `RequestTargetRepaint() -> RenderDocRepaintResult`
- `LastCapture() -> RenderDocCaptureLookupResult`
- `End() -> RenderDocEndCaptureResult`
- `Discard() -> RenderDocDiscardCaptureResult`
- `RecoverStaleCaptures(int maxAttempts = 8) -> RenderDocRecoveryResult`
- `Json(object) -> string`

每个操作返回独立结果类型，包含 `Success`、`ErrorCode`、`Message` 与操作特有数据。`.rdc` 创建后的 `ReplayValidation` 初始为 `not_run`；嵌入式 Python `OpenCapture` 成功才表示可回放。

立即截取当前视图：

```csharp
var capture = await RenderDocCaptureApi.CaptureOnceAsync(
    new RenderDocBeginCaptureOptions {
        Target = RenderDocCaptureTarget.Game,
        CaptureName = "game_analysis"
    });
print(RenderDocCaptureApi.Json(capture));
```

Scene 视图使用 `RenderDocCaptureTarget.Scene`。默认输出目录为 `<UnityProject>/Library/Locus/RenderDoc`。

### 条件触发

跨帧条件通过 `unity_run_states` 编排，生命周期固定为：

`等待条件 → Begin → Trigger → LastCapture → End`

每个失败、超时和取消路径调用 `End` 或 `Discard`，state 的 `end` 再以 `Discard` 兜底。触发状态示例：

```csharp
bool trigger = GameObject.Find("CaptureMarker") != null;
if (!trigger) return;

var begin = RenderDocCaptureApi.Begin(new RenderDocBeginCaptureOptions {
    Target = RenderDocCaptureTarget.Game,
    CaptureName = "marker_hit",
    CaptureTitle = "CaptureMarker appeared"
});
ctx.Print(RenderDocCaptureApi.Json(begin));
if (!begin.Success) { ctx.Fail(begin.ErrorCode + ": " + begin.Message); return; }
ctx.Goto("capturing");
```

`capturing.start`：

```csharp
var trigger = RenderDocCaptureApi.Trigger();
ctx.Print(RenderDocCaptureApi.Json(trigger));
if (!trigger.Success) {
    RenderDocCaptureApi.Discard();
    ctx.Fail(trigger.ErrorCode + ": " + trigger.Message);
}
```

`capturing.update`：

```csharp
var lookup = RenderDocCaptureApi.LastCapture();
if (lookup.Success && lookup.NewSinceLastBegin) {
    var end = RenderDocCaptureApi.End();
    ctx.Print(RenderDocCaptureApi.Json(new { lookup, end }));
    if (!end.Success) { ctx.Fail(end.ErrorCode + ": " + end.Message); return; }
    ctx.Done(lookup.Capture.Path);
    return;
}
RenderDocCaptureApi.RequestTargetRepaint();
```

`capturing.end`：

```csharp
if (RenderDocCaptureApi.Status().TrackedCaptureActive)
    RenderDocCaptureApi.Discard();
```

只有 `Status().State == "untracked_capture_active"` 时调用 `RecoverStaleCaptures`。正常清理始终使用精确会话的 `End` / `Discard`。

### Python replay API

```python
import locus_renderdoc as lrd

report = lrd.inspect_capture(capture_path)
actions = report["actions"]
resources = report["resources"]
buffers = report["buffers"]
```

公开入口直接返回 Python 数据类型：

- `inspect_capture(path, include_pipeline=False) -> dict`
- `buffers(path) -> list[dict]`
- `compute_bindings(path, event_id) -> dict`
- `buffer_data(path, event_id, resource, offset=0, length=None) -> dict`
- `save_texture(path, event_id, resource, output_path, mip=0, slice_index=0) -> dict`
- `unpack_buffer(result_or_bytes, format_string, stride=None, max_elements=None) -> list[tuple]`
- `open_capture(path) -> Capture`

模块自动使用与捕获版本一致的后台 qrenderdoc replay worker；调用方无需 CLI 参数、环境变量或 JSON 中间报告。`inspect_capture` 的 action marker 表示 RenderGraph/SRP 的 GPU 可见边界；大帧谨慎使用 `include_pipeline=True`。

读取指定 Dispatch 的 Compute Buffer：

```python
bindings = lrd.compute_bindings(capture_path, event_id)
uav = bindings["readWrite"][0]
result = lrd.buffer_data(
    capture_path,
    event_id,
    uav["resourceId"],
    offset=0,
    length=None,
)
assert isinstance(result["data"], bytes)
rows = lrd.unpack_buffer(result, "<4I", stride=16, max_elements=128)
```

`resource` 支持 `ResourceId::N`、精确资源名、资源字典，以及 `compute-readonly:<index>` / `compute-readwrite:<index>`。读取前会对目标 EID 执行 `SetFrameEvent`；大型 Buffer 应分段读取。

导出指定事件的中间纹理：

```python
saved = lrd.save_texture(
    capture_path,
    event_id,
    "color:0",
    output_path,
)
```

纹理 selector 支持 `color:<index>`、`depth`、`ResourceId::N` 和精确资源名；输出支持 `.png`、`.exr`、`.dds`、`.hdr`、`.jpg`、`.bmp`、`.tga`。

## 支持边界

- Frame Debugger：Unity `6000.3`、`6000.5`，Editor 本地渲染及 Unity 支持的远程 Development Player。
- RenderDoc：Windows x64、Unity `6000.3+`，以及 RenderDoc 支持的 D3D11、D3D12、Vulkan、OpenGL 系列图形 API。
- SRP 事件名称、层级与 Render Target 数量由当前渲染管线决定；分析通道本身与具体渲染管线无关。
- RenderDoc runtime 由 `bun run renderdoc:bundle` 生成，安装版随 Skill 携带。
