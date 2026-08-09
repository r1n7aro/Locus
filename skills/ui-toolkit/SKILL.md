---
summary: >-
  用于 Unity 6.3+ UI Toolkit 的结构、样式、布局和交互调试。Skill 激活时加载 `Locus.Skills.UIToolkitApi` 程序集；通过现有 `unity_execute` 编排一次或多步 C# 操作。覆盖 EditorWindow 与 Play Mode 中的 Runtime `UIDocument`。
tools:
  - unity_execute
  - unity_capture_viewport
  - read
  - edit
---

# UI Toolkit C# DevTools

## 使用约束

- 在 `unity_execute` 中加入 `using Locus.Skills;`，从 `UIToolkitApi.Open()` 开始。
- 输出通过 `print(UIToolkitApi.Json(value))` 返回。投影当前判断所需字段，避免返回完整对象。
- 元素 ID 属于当前 Unity 会话和当前面板结构。收到 `stale_element` 后重新定位。
- selector 支持 `#name`、`.class`、`Type`、`Type#name`、`Type.class` 与 `*`。
- `IMGUIContainer` 内部控件不属于 `VisualElement` 树。

## 常用 API

```csharp
var ui = UIToolkitApi.Open();
var panels = ui.Panels();
var panel = ui.FindPanel("Replay Timeline");
var root = panel.Root();
var button = panel.Find("#play-button");
var subtree = button.InspectSubtree(new UIQuery { Depth = 2, MaxElements = 30 });
print(UIToolkitApi.Json(new {
    panels = panels,
    target = button.Info,
    elements = subtree.Elements
}));
```

`UIPanel.Inspect(UIQuery)` 返回有界结构。默认 `Depth=2`、`MaxElements=80`；`InteractiveOnly=true` 只保留可交互元素。样式判断使用单元素查询：

```csharp
var element = panel.Find(".timeline-row", new UIQuery {
    Depth = 1,
    MaxElements = 1,
    IncludeComputedStyle = true,
    IncludeMatchedRules = true,
    StyleProperties = new [] { "display", "width", "height", "flexGrow" }
});
print(UIToolkitApi.Json(element.Info));
```

`UIElement` 提供 `Click`、`Focus`、`SetValue`、`Type`、`Toggle`、`Select`、`Scroll`、`Press`、`DragTo`。一个 `unity_execute` 可完成复合流程并只返回最终收据：

```csharp
var ui = UIToolkitApi.Open();
var panel = ui.FindPanel("Settings");
var search = panel.Find("#search");
search.Type("render pipeline");
await panel.WaitAsync(".search-result", UIWaitCondition.Visible, timeoutMs: 10000);
var result = panel.Find(".search-result");
result.Click();
print(UIToolkitApi.Json(new { search = search.Id, clicked = result.Id }));
```

等待条件包括 `Exists`、`Missing`、`Visible`、`Hidden`、`Enabled`、`Disabled`、`Text`、`Value`、`LayoutStable`。等待使用 Editor update 回调，不阻塞 Unity 主线程。

## 布局预览与持久修改

`SetStyles` 原子应用 inline 样式并返回 `UIStylePreview`；验证失败时调用 `Rollback()`。`ResetStyles` 清除指定 inline 属性。

```csharp
var preview = element.SetStyles(
    new UIStyleChange("width", "420px"),
    new UIStyleChange("flex-grow", "1")
);
await element.WaitAsync(UIWaitCondition.LayoutStable, timeoutMs: 5000);
var measured = element.Refresh(new UIQuery {
    IncludeComputedStyle = true,
    StyleProperties = new [] { "width", "flexGrow" }
});
preview.Rollback();
print(UIToolkitApi.Json(new { measured, preview.PreviewId }));
```

持久修改使用 `UIElementInfo.UxmlSource` 或 `MatchedRules[].Path` 定位 UXML/USS，再通过 `read` 与 `edit` 修改源文件。Unity 导入并重建面板后重新定位元素。

## 图像确认

`element.Highlight()` 或 `panel.HighlightInteractions()` 返回 `CaptureTarget`、`RequestEditorStatus` 与 `WindowTitle`。随后调用 `unity_capture_viewport`，截图完成后调用 `panel.ClearHighlight()`。Runtime UI 使用 Game View；Editor UI 使用对应 EditorWindow。

## 结果模型

- `UIPanelInfo`：`Id`、`Kind`、`Title`、`Owner`、`ElementCount`、`Documents`
- `UIElementInfo`：`Id`、`ParentId`、`Type`、`Selector`、`ChildCount`、`Rect`、非空文本/值、动作、状态、源文件、按需样式与 USS 规则
- `Rect` 顺序为 `[x, y, width, height]`
- Runtime `UIDocument` 只在 Play Mode 存活；Game View 暂停时仍可检查当前树

## Requirements

- Unity `6000.3` 或更高版本
- UI Toolkit Editor UI，或连接到该 Editor 的 Play Mode Runtime UI
