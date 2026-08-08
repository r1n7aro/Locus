---
tools:
  - unity_ui_list_panels
  - unity_ui_inspect
  - unity_ui_style
  - unity_ui_action
  - unity_ui_wait
  - unity_ui_highlight
  - unity_capture_viewport
  - read
  - edit
---

# UI Toolkit DevTools

## L1

用于 Unity 6.3+ UI Toolkit 的结构、样式、布局和交互调试。覆盖 Unity EditorWindow UI 与 Play Mode 中的 Runtime `UIDocument`；工具直接在目标 Unity Editor 主线程读取和驱动实时 `VisualElement` 树。

## Instructions

1. 先调用 `unity_ui_list_panels`。`kind=editor_window` 对应编辑器 UI，`kind=runtime_uidocument` 对应 Runtime UI；Runtime 面板在 Play Mode 中保持实时，Game View 暂停时仍可检查当前树。
2. 使用返回的 `panelId` 调用 `unity_ui_inspect`。初次检查采用 `depth=4`、`maxElements=250`；需要定位时用 `#name`、`.class`、`Type`、`Type#name` 或 `Type.class`。元素 ID 只在当前 Unity 会话和当前文档结构中有效；收到 `stale_element` 后重新检查。
3. 结构检查优先读取 `text`、`value`、`actions`、`layout`、`worldBound`、`visible`、`enabled` 和 `hitTest`。需要解释布局时开启 `includeComputedStyle`；需要定位持久样式来源时开启 `includeMatchedRules`。
4. 使用 `unity_ui_style` 做实时 inline 预览。一次请求中的 edits 原子应用，并返回 `previewId`；结论不成立时立即 `rollback`。`reset` 清除指定 inline 属性。支持常见尺寸、边距、内边距、边框、flex、对齐、定位、显示、可见性、透明度、颜色、字号、空白和 overflow 属性。
5. 持久修改使用检查结果中的 `uxmlSource` 或 `matchedRules[].fullPath` 定位 UXML/USS，通过 `read` 和 `edit` 修改源文件。等待 Unity 导入与面板重建后重新列出/检查元素；旧 element ID 可能失效。
6. 使用 `unity_ui_action` 驱动交互。按钮采用 `click`，输入框采用 `type` 或 `setValue`，Toggle 采用 `toggle`，DropdownField 采用 `select`，ScrollView 采用 `scroll`。事件会在实时 Editor 或 Runtime panel 中派发，回调可能修改场景、资源或项目状态。
7. 异步 UI 使用 `unity_ui_wait`。动画和延迟布局优先等待 `layoutStable`，动态列表优先等待目标 selector 的 `exists`/`visible`，操作结果优先等待 `text` 或 `value`。
8. 需要图像确认时先调用 `unity_ui_highlight`，再按照 panel 返回的 `captureTarget`、`requestEditorStatus` 调用 `unity_capture_viewport`。高亮会聚焦对应 EditorWindow；同名窗口存在时省略 `window_title` 以捕获当前焦点窗口，名称唯一时可传 `window_title=windowTitle`。Runtime UI 使用 `target=game`。截图后调用 `clear`，避免诊断层残留。
9. `IMGUIContainer` 是 UI Toolkit 与 IMGUI 的边界。可以检查容器布局和可见性，内部 IMGUI 控件不属于 `VisualElement` 树。

## Result interpretation

- `documentRevision` 在检测到根或结构变化时增加，用于判断两次检查是否来自同一棵树。
- `hitTest=direct` 表示元素中心点直接命中；`descendant` 表示中心点由子元素接收；`blocked` 表示被其他元素遮挡；`outside` 表示当前布局不在 panel 可交互区域。
- `computedStyle` 是布局完成后的 resolved 值；inline `style`、UXML 属性和 USS 级联共同决定它。
- `matchedRules` 来自 Unity 6.3/6.5 编辑器内部诊断器。Unity 无法提供规则细节时返回 capability warning，结构、computed style 与交互工具仍可使用。

## Requirements

- Unity `6000.3` 或更高版本
- UI Toolkit Editor UI 或连接到该 Editor 的 Play Mode Runtime UI
