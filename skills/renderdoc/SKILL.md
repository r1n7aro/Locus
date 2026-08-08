---
tools:
  - renderdoc_capture_frame
---

# RenderDoc Frame Capture

## L1
用于 Unity 6.3+ Windows x64 编辑器的 Game/Scene 视图图形截帧、渲染流程检查、中间纹理导出和 Python 分析。渲染管线可以是 Built-in、URP、HDRP 或自定义 SRP；结果以图形 API 的 action、resource 和 pipeline state 表达。

## Instructions

1. 调用 `renderdoc_capture_frame`，`target` 只取 `game` 或 `scene`。工具在编辑、播放、暂停状态均可运行，产物固定写入当前 Unity 项目的 `Library/Locus/RenderDoc/`。
2. 工具只负责截帧。使用返回值中的 `pythonExecutable` 运行 RenderDoc 内置 Python，使用 `inspectionScript` 生成帧结构 JSON。不要尝试向系统 Python 安装 `renderdoc`；该模块由 `qrenderdoc` 自带并与 replay runtime 同版本。
3. PowerShell 分析命令：

```powershell
$env:LOCUS_RENDERDOC_CAPTURE = '<capturePath>'
$env:LOCUS_RENDERDOC_REPORT = '<capturePath>.json'
& '<pythonExecutable>' --python '<inspectionScript>'
```

报告包含 API、action 树、marker/pass 层级、draw/dispatch/copy flags、输入输出 resource id、纹理描述和资源表。需要逐事件 pipeline 输出时，额外设置 `$env:LOCUS_RENDERDOC_PIPELINE_SNAPSHOTS = '1'`；大帧会明显增加 replay 时间。

4. 导出中间纹理时使用返回值中的 `textureExportScript`：

```powershell
$env:LOCUS_RENDERDOC_CAPTURE = '<capturePath>'
$env:LOCUS_RENDERDOC_EVENT = '1234'
$env:LOCUS_RENDERDOC_RESOURCE = 'color:0'
$env:LOCUS_RENDERDOC_TEXTURE_OUTPUT = '<absolute-output.png>'
& '<pythonExecutable>' --python '<textureExportScript>'
```

`LOCUS_RENDERDOC_RESOURCE` 支持 `color:<index>`、`depth`、报告中的 `ResourceId::N` 或精确资源名。输出扩展名支持 `.png`、`.exr`、`.dds`、`.hdr`、`.jpg`、`.bmp` 和 `.tga`。

5. 需要更细分析时，在 Skill 的 `scripts/` 目录脚本基础上编写一次性 Python 脚本，并保持以下生命周期：

```python
import renderdoc as rd

capture = rd.OpenCaptureFile()
controller = None
try:
    result = capture.OpenFile(capture_path, "", None)
    if result != rd.ResultCode.Succeeded:
        raise RuntimeError(str(result))
    result, controller = capture.OpenCapture(rd.ReplayOptions(), None)
    if result != rd.ResultCode.Succeeded:
        raise RuntimeError(str(result))

    actions = controller.GetRootActions()
    controller.SetFrameEvent(event_id, True)
    pipeline = controller.GetPipelineState()
finally:
    if controller is not None:
        controller.Shutdown()
    capture.Shutdown()
```

6. 将 action marker 视为 RenderGraph/SRP pass 的 GPU 可见边界；`GetPipelineState()` 表示选中 event 的真实绑定状态；`GetUsage(resourceId)` 用于追踪纹理从写入到采样的流转。CPU 侧未提交 GPU marker 的 C# pass 名称不会出现在 `.rdc` 中。
7. 完成分析后报告 `.rdc`、JSON 和导出纹理的绝对路径，并指出关键 event id、resource id、首次写入、后续读取与最终输出关系。

## Requirements

- Windows x64
- Unity `6000.3` 或更高版本
- Unity 图形 API 为 RenderDoc 支持的 D3D11、D3D12、Vulkan 或 OpenGL 系列
- Skill runtime 由 `bun run renderdoc:bundle` 生成；安装版已随 Skill 携带
