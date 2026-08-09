using System;
using System.Collections;
using System.Collections.Generic;
using System.Globalization;
using System.Reflection;
using System.Runtime.CompilerServices;
using System.Threading.Tasks;

using UnityEditor;
using UnityEngine;
using UnityEngine.UIElements;

namespace Locus.Skills
{
    // Internal implementation used by the public UIToolkitApi facade.  Keeping
    // the Unity-version-specific reflection and event plumbing out of the
    // public surface lets agents compose larger operations with normal C#
    // handles instead of constructing tool-shaped request envelopes.
    internal static class UIToolkitDevTools
    {
        private const int MinimumUnityMajor = 6000;
        private const int MinimumUnityMinor = 3;
        private const int CompactTextLimit = 160;
        private const string OverlayName = "__locus_ui_toolkit_overlay";

        public sealed class SkillContext
        {
            public string skillPackageRoot;
            public string workingDirectory;
        }

        public sealed class ListPanelsRequest
        {
            public bool includeEmpty;
            public SkillContext __locus;
        }

        public sealed class InspectRequest
        {
            public string panelId;
            public int elementId;
            public string selector;
            public int depth;
            public int maxElements;
            public bool includeHidden;
            public bool interactiveOnly;
            public bool includeComputedStyle;
            public bool includeMatchedRules;
            public List<string> styleProperties;
            public SkillContext __locus;
        }

        public sealed class StyleEdit
        {
            public string property;
            public string value;
        }

        public sealed class StyleRequest
        {
            public string operation;
            public string panelId;
            public int elementId;
            public string previewId;
            public List<StyleEdit> edits;
            public SkillContext __locus;
        }

        public sealed class ActionRequest
        {
            public string panelId;
            public int elementId;
            public string operation;
            public object value;
            public string text;
            public bool append;
            public string key;
            public float x;
            public float y;
            public int targetElementId;
            public SkillContext __locus;
        }

        public sealed class WaitRequest
        {
            public string panelId;
            public int elementId;
            public string selector;
            public string condition;
            public string expected;
            public int timeoutMs;
            public int stableFrames;
            public SkillContext __locus;
        }

        public sealed class HighlightRequest
        {
            public string panelId;
            public string operation;
            public int elementId;
            public int maxElements;
            public SkillContext __locus;
        }

        private sealed class PanelRecord
        {
            public string id;
            public IPanel panel;
            public VisualElement root;
            public EditorWindow editorWindow;
            public readonly List<UIDocument> documents = new List<UIDocument>();
            public string kind;
            public string title;
            public string windowTitle;
            public string captureTarget;
            public int revision = 1;
            public int fingerprint;
            public int nextElementId = 1;
            public readonly Dictionary<VisualElement, int> ids =
                new Dictionary<VisualElement, int>(ReferenceComparer<VisualElement>.Instance);
            public readonly Dictionary<int, VisualElement> elements =
                new Dictionary<int, VisualElement>();
            public VisualElement overlay;
        }

        private sealed class PreviewRecord
        {
            public string id;
            public string panelId;
            public VisualElement element;
            public readonly Dictionary<PropertyInfo, object> originals =
                new Dictionary<PropertyInfo, object>();
        }

        private sealed class ReferenceComparer<T> : IEqualityComparer<T> where T : class
        {
            public static readonly ReferenceComparer<T> Instance = new ReferenceComparer<T>();

            public bool Equals(T left, T right)
            {
                return ReferenceEquals(left, right);
            }

            public int GetHashCode(T value)
            {
                return value == null ? 0 : RuntimeHelpers.GetHashCode(value);
            }
        }

        private static readonly Dictionary<string, PanelRecord> Panels =
            new Dictionary<string, PanelRecord>(StringComparer.Ordinal);
        private static readonly Dictionary<IPanel, PanelRecord> PanelsByObject =
            new Dictionary<IPanel, PanelRecord>(ReferenceComparer<IPanel>.Instance);
        private static readonly Dictionary<string, PreviewRecord> Previews =
            new Dictionary<string, PreviewRecord>(StringComparer.Ordinal);
        private static int _nextPanelId = 1;
        private static int _nextPreviewId = 1;

        private static readonly HashSet<string> SupportedStyleProperties =
            new HashSet<string>(StringComparer.Ordinal)
            {
                "width", "height", "minWidth", "minHeight", "maxWidth", "maxHeight",
                "left", "right", "top", "bottom",
                "marginLeft", "marginRight", "marginTop", "marginBottom",
                "paddingLeft", "paddingRight", "paddingTop", "paddingBottom",
                "borderLeftWidth", "borderRightWidth", "borderTopWidth", "borderBottomWidth",
                "borderTopLeftRadius", "borderTopRightRadius", "borderBottomLeftRadius", "borderBottomRightRadius",
                "flexBasis", "flexGrow", "flexShrink", "flexDirection", "flexWrap",
                "alignContent", "alignItems", "alignSelf", "justifyContent",
                "display", "position", "visibility", "opacity", "overflow", "whiteSpace",
                "color", "backgroundColor",
                "borderLeftColor", "borderRightColor", "borderTopColor", "borderBottomColor",
                "fontSize", "unityFontStyleAndWeight", "unityTextAlign"
            };

        private static readonly HashSet<long> InteractiveEventTypeIds =
            new HashSet<long>
            {
                ClickEvent.TypeId(),
                MouseDownEvent.TypeId(),
                MouseUpEvent.TypeId(),
                PointerDownEvent.TypeId(),
                PointerUpEvent.TypeId(),
                KeyDownEvent.TypeId(),
                NavigationSubmitEvent.TypeId()
            };

        public static Dictionary<string, object> ListPanels(ListPanelsRequest request)
        {
            ValidateUnityVersion();
            List<PanelRecord> records = DiscoverPanels();
            List<object> output = new List<object>();
            bool includeEmpty = request != null && request.includeEmpty;
            for (int i = 0; i < records.Count; i++)
            {
                PanelRecord record = records[i];
                int count = CountElements(record.root);
                if (!includeEmpty && count <= 1)
                    continue;
                output.Add(PanelSummary(record, count));
            }

            return new Dictionary<string, object>
            {
                { "editorStatus", CurrentEditorStatus() },
                { "panels", output }
            };
        }

        public static Dictionary<string, object> Inspect(InspectRequest request)
        {
            ValidateUnityVersion();
            if (request == null)
                throw new InvalidOperationException("UI Toolkit inspect request is empty.");

            PanelRecord record = RequirePanel(request.panelId);
            VisualElement scope = ResolveElement(record, request.elementId, request.selector, true);
            int maxDepth = request.depth <= 0 ? 2 : Math.Min(20, request.depth);
            int maxElements = request.maxElements <= 0 ? 80 : Math.Min(2000, request.maxElements);
            List<object> elements = new List<object>();
            List<string> warnings = new List<string>();
            bool truncated = false;

            Walk(
                scope,
                0,
                maxDepth,
                delegate(VisualElement element, int depth)
                {
                    if (elements.Count >= maxElements)
                    {
                        truncated = true;
                        return false;
                    }
                    if (!request.includeHidden && !IsVisible(element))
                        return false;
                    if (!request.interactiveOnly || IsInteractive(element))
                    {
                        elements.Add(ElementSnapshot(
                            record,
                            element,
                            request.includeComputedStyle && ReferenceEquals(element, scope),
                            request.includeMatchedRules && ReferenceEquals(element, scope),
                            request.styleProperties,
                            warnings));
                    }
                    return true;
                });

            Dictionary<string, object> result = new Dictionary<string, object>
            {
                { "panelId", record.id },
                { "documentRevision", record.revision },
                { "scopeElementId", ElementId(record, scope) },
                { "elements", elements }
            };
            if (truncated)
                result["truncated"] = true;
            if (warnings.Count > 0)
                result["warnings"] = warnings;
            return result;
        }

        public static Dictionary<string, object> Style(StyleRequest request)
        {
            ValidateUnityVersion();
            if (request == null)
                throw new InvalidOperationException("UI Toolkit style request is empty.");
            string operation = (request.operation ?? "").Trim().ToLowerInvariant();
            if (operation == "rollback")
                return RollbackStyle(request.previewId);
            if (operation != "set" && operation != "reset")
                throw new InvalidOperationException("Style operation must be set, reset, or rollback.");

            PanelRecord record = RequirePanel(request.panelId);
            VisualElement element = RequireElement(record, request.elementId);
            if (request.edits == null || request.edits.Count == 0)
                throw new InvalidOperationException("Style set/reset requires at least one edit.");

            PreviewRecord preview = new PreviewRecord
            {
                id = "preview-" + _nextPreviewId++,
                panelId = record.id,
                element = element
            };
            IStyle style = element.style;
            List<KeyValuePair<PropertyInfo, object>> parsed = new List<KeyValuePair<PropertyInfo, object>>();
            for (int i = 0; i < request.edits.Count; i++)
            {
                StyleEdit edit = request.edits[i];
                string propertyName = NormalizeStyleProperty(edit == null ? null : edit.property);
                if (!SupportedStyleProperties.Contains(propertyName))
                    throw new InvalidOperationException("Unsupported inline style property: " + propertyName + ".");
                PropertyInfo property = typeof(IStyle).GetProperty(propertyName, BindingFlags.Instance | BindingFlags.Public);
                if (property == null || !property.CanRead || !property.CanWrite)
                    throw new InvalidOperationException("Unity does not expose IStyle." + propertyName + " in this version.");
                object next = operation == "reset"
                    ? StyleKeywordValue(property.PropertyType, StyleKeyword.Null)
                    : ParseStyleValue(property.PropertyType, edit.value);
                preview.originals[property] = property.GetValue(style, null);
                parsed.Add(new KeyValuePair<PropertyInfo, object>(property, next));
            }

            try
            {
                for (int i = 0; i < parsed.Count; i++)
                    parsed[i].Key.SetValue(style, parsed[i].Value, null);
            }
            catch (Exception exception)
            {
                RestoreStyle(preview);
                throw new InvalidOperationException("Unity rejected the inline style preview: " + RootMessage(exception), exception);
            }

            Previews[preview.id] = preview;
            element.MarkDirtyRepaint();
            return new Dictionary<string, object>
            {
                { "previewId", preview.id },
                { "elementId", ElementId(record, element) },
                { "documentRevision", record.revision }
            };
        }

        public static Dictionary<string, object> Action(ActionRequest request)
        {
            ValidateUnityVersion();
            if (request == null)
                throw new InvalidOperationException("UI Toolkit action request is empty.");
            PanelRecord record = RequirePanel(request.panelId);
            VisualElement element = RequireElement(record, request.elementId);
            string operation = (request.operation ?? "").Trim().ToLowerInvariant();

            switch (operation)
            {
                case "click":
                    Click(element);
                    break;
                case "focus":
                    element.Focus();
                    break;
                case "setvalue":
                case "select":
                    SetControlValue(element, request.value);
                    break;
                case "type":
                    TypeText(element, request.text ?? "", request.append);
                    break;
                case "toggle":
                    object current = ReadControlValue(element);
                    bool next = request.value == null
                        ? !(current is bool && (bool)current)
                        : Convert.ToBoolean(request.value, CultureInfo.InvariantCulture);
                    SetControlValue(element, next);
                    break;
                case "scroll":
                    ScrollView scroll = element as ScrollView;
                    if (scroll == null)
                        throw new InvalidOperationException("scroll requires a ScrollView element.");
                    scroll.scrollOffset = new Vector2(request.x, request.y);
                    break;
                case "press":
                    PressKey(element, request.key);
                    break;
                case "drag":
                    VisualElement destination = RequireElement(record, request.targetElementId);
                    Drag(element, destination);
                    break;
                default:
                    throw new InvalidOperationException(
                        "Action operation must be click, focus, setValue, type, toggle, select, scroll, press, or drag.");
            }

            element.MarkDirtyRepaint();
            RefreshDocumentRevision(record);
            Dictionary<string, object> result = new Dictionary<string, object>
            {
                { "elementId", request.elementId },
                { "documentRevision", record.revision }
            };
            if (operation == "setvalue" || operation == "select" || operation == "toggle" || operation == "type")
                result["value"] = CompactValue(JsonSafeValue(ReadControlValue(element)), CompactTextLimit);
            else if (operation == "focus")
                result["focused"] = record.panel.focusController != null
                    && ReferenceEquals(record.panel.focusController.focusedElement, element);
            else if (operation == "scroll")
            {
                Vector2 offset = ((ScrollView)element).scrollOffset;
                result["scrollOffset"] = new float[] { Round(offset.x), Round(offset.y) };
            }
            else if (operation == "drag")
                result["targetElementId"] = request.targetElementId;
            return result;
        }

        public static Task<Dictionary<string, object>> Wait(WaitRequest request)
        {
            ValidateUnityVersion();
            if (request == null)
                throw new InvalidOperationException("UI Toolkit wait request is empty.");
            PanelRecord initialRecord = RequirePanel(request.panelId);
            string condition = (request.condition ?? "").Trim().ToLowerInvariant();
            HashSet<string> allowed = new HashSet<string>(StringComparer.Ordinal)
            {
                "exists", "missing", "visible", "hidden", "enabled", "disabled", "text", "value", "layoutstable"
            };
            if (!allowed.Contains(condition))
                throw new InvalidOperationException("Unsupported UI wait condition: " + condition + ".");

            int timeoutMs = request.timeoutMs <= 0 ? 10000 : Math.Min(60000, request.timeoutMs);
            int requiredStableFrames = request.stableFrames <= 0 ? 3 : Math.Min(120, request.stableFrames);
            double started = EditorApplication.timeSinceStartup;
            double deadline = started + timeoutMs / 1000.0;
            TaskCompletionSource<Dictionary<string, object>> source =
                new TaskCompletionSource<Dictionary<string, object>>();
            Rect lastRect = default(Rect);
            bool hasLastRect = false;
            int stable = 0;
            EditorApplication.CallbackFunction callback = null;
            callback = delegate
            {
                try
                {
                    DiscoverPanels();
                    PanelRecord record;
                    if (!Panels.TryGetValue(initialRecord.id, out record))
                    {
                        if (condition == "missing")
                        {
                            CompleteWait(source, callback, request, null, null, started, 1);
                            return;
                        }
                    }
                    VisualElement element = record == null
                        ? null
                        : ResolveElement(record, request.elementId, request.selector, false);
                    bool matched = EvaluateWait(
                        condition,
                        request.expected,
                        element,
                        ref lastRect,
                        ref hasLastRect,
                        ref stable,
                        requiredStableFrames);
                    if (matched)
                    {
                        CompleteWait(source, callback, request, record, element, started, stable);
                        return;
                    }
                    if (EditorApplication.timeSinceStartup >= deadline)
                    {
                        EditorApplication.update -= callback;
                        source.TrySetException(new TimeoutException(
                            "Timed out waiting for UI condition '" + request.condition + "' after " + timeoutMs + " ms."));
                    }
                }
                catch (Exception exception)
                {
                    EditorApplication.update -= callback;
                    source.TrySetException(exception);
                }
            };
            EditorApplication.update += callback;
            callback();
            return source.Task;
        }

        public static Dictionary<string, object> Highlight(HighlightRequest request)
        {
            ValidateUnityVersion();
            if (request == null)
                throw new InvalidOperationException("UI Toolkit highlight request is empty.");
            PanelRecord record = RequirePanel(request.panelId);
            RemoveOverlay(record);
            string operation = (request.operation ?? "").Trim().ToLowerInvariant();
            if (operation == "clear")
            {
                return new Dictionary<string, object>
                {
                    { "cleared", true }
                };
            }

            List<VisualElement> targets = new List<VisualElement>();
            if (operation == "element")
            {
                targets.Add(RequireElement(record, request.elementId));
            }
            else if (operation == "interactions")
            {
                int maximum = request.maxElements <= 0 ? 40 : Math.Min(200, request.maxElements);
                Walk(
                    record.root,
                    0,
                    100,
                    delegate(VisualElement element, int depth)
                    {
                        if (targets.Count >= maximum)
                            return false;
                        if (IsVisible(element) && IsInteractive(element))
                            targets.Add(element);
                        return true;
                    });
            }
            else
            {
                throw new InvalidOperationException("Highlight operation must be element, interactions, or clear.");
            }

            VisualElement overlay = CreateOverlay(record, targets, operation == "interactions");
            record.overlay = overlay;
            record.root.Add(overlay);
            overlay.BringToFront();
            record.root.MarkDirtyRepaint();
            if (record.editorWindow != null)
            {
                record.editorWindow.Focus();
                record.editorWindow.Repaint();
            }
            return new Dictionary<string, object>
            {
                { "highlighted", targets.Count },
                { "captureTarget", record.captureTarget },
                { "requestEditorStatus", CurrentEditorStatus() },
                { "windowTitle", record.windowTitle ?? "" }
            };
        }

        private static Dictionary<string, object> RollbackStyle(string previewId)
        {
            PreviewRecord preview;
            if (string.IsNullOrWhiteSpace(previewId) || !Previews.TryGetValue(previewId, out preview))
                throw new InvalidOperationException("Unknown or expired style preview: " + (previewId ?? "") + ".");
            PanelRecord record = RequirePanel(preview.panelId);
            if (preview.element == null || !BelongsTo(record, preview.element))
                throw new InvalidOperationException("stale_element: the preview target was rebuilt before rollback.");
            RestoreStyle(preview);
            Previews.Remove(preview.id);
            preview.element.MarkDirtyRepaint();
            return new Dictionary<string, object>
            {
                { "rolledBack", preview.id },
                { "elementId", ElementId(record, preview.element) },
                { "documentRevision", record.revision }
            };
        }

        private static void RestoreStyle(PreviewRecord preview)
        {
            if (preview == null || preview.element == null)
                return;
            IStyle style = preview.element.style;
            foreach (KeyValuePair<PropertyInfo, object> entry in preview.originals)
                entry.Key.SetValue(style, entry.Value, null);
        }

        private static List<PanelRecord> DiscoverPanels()
        {
            HashSet<IPanel> live = new HashSet<IPanel>(ReferenceComparer<IPanel>.Instance);
            EditorWindow[] windows = Resources.FindObjectsOfTypeAll<EditorWindow>();
            for (int i = 0; i < windows.Length; i++)
            {
                EditorWindow window = windows[i];
                if (window == null || window.rootVisualElement == null || window.rootVisualElement.panel == null)
                    continue;
                IPanel panel = window.rootVisualElement.panel;
                PanelRecord record = GetOrCreatePanel(panel);
                record.root = window.rootVisualElement;
                record.editorWindow = window;
                record.documents.Clear();
                record.kind = "editor_window";
                record.title = WindowTitle(window);
                record.windowTitle = record.title;
                record.captureTarget = "editor_window";
                live.Add(panel);
            }

            UIDocument[] documents = Resources.FindObjectsOfTypeAll<UIDocument>();
            for (int i = 0; i < documents.Length; i++)
            {
                UIDocument document = documents[i];
                if (document == null || document.rootVisualElement == null || document.rootVisualElement.panel == null)
                    continue;
                IPanel panel = document.rootVisualElement.panel;
                PanelRecord record = GetOrCreatePanel(panel);
                if (record.kind != "runtime_uidocument")
                {
                    record.documents.Clear();
                    record.editorWindow = null;
                }
                record.kind = "runtime_uidocument";
                record.root = panel.visualTree;
                if (!record.documents.Contains(document))
                    record.documents.Add(document);
                record.title = RuntimePanelTitle(record.documents);
                record.windowTitle = "Game";
                record.captureTarget = "game";
                live.Add(panel);
            }

            List<string> removed = new List<string>();
            foreach (KeyValuePair<string, PanelRecord> entry in Panels)
            {
                if (!live.Contains(entry.Value.panel))
                    removed.Add(entry.Key);
            }
            for (int i = 0; i < removed.Count; i++)
            {
                PanelRecord record = Panels[removed[i]];
                RemoveOverlay(record);
                Panels.Remove(removed[i]);
                PanelsByObject.Remove(record.panel);
            }

            List<PanelRecord> records = new List<PanelRecord>();
            foreach (PanelRecord record in Panels.Values)
            {
                if (record.root == null)
                    continue;
                RefreshDocumentRevision(record);
                records.Add(record);
            }
            records.Sort(delegate(PanelRecord left, PanelRecord right)
            {
                int kind = string.Compare(left.kind, right.kind, StringComparison.Ordinal);
                return kind != 0 ? kind : string.Compare(left.title, right.title, StringComparison.OrdinalIgnoreCase);
            });
            return records;
        }

        private static PanelRecord GetOrCreatePanel(IPanel panel)
        {
            PanelRecord record;
            if (PanelsByObject.TryGetValue(panel, out record))
                return record;
            record = new PanelRecord
            {
                id = "panel-" + _nextPanelId++,
                panel = panel
            };
            Panels[record.id] = record;
            PanelsByObject[panel] = record;
            return record;
        }

        private static PanelRecord RequirePanel(string panelId)
        {
            DiscoverPanels();
            PanelRecord record;
            if (string.IsNullOrWhiteSpace(panelId) || !Panels.TryGetValue(panelId, out record))
                throw new InvalidOperationException(
                    "panel_not_found: refresh UIToolkitApi.Open().Panels().");
            return record;
        }

        private static VisualElement ResolveElement(
            PanelRecord record,
            int elementId,
            string selector,
            bool throwWhenMissing)
        {
            VisualElement element = null;
            if (elementId > 0)
            {
                record.elements.TryGetValue(elementId, out element);
                if (element != null && !BelongsTo(record, element))
                    element = null;
            }
            else if (!string.IsNullOrWhiteSpace(selector))
            {
                element = FindBySelector(record.root, selector);
            }
            else
            {
                element = record.root;
            }
            if (element == null && throwWhenMissing)
            {
                string reason = elementId > 0 ? "stale_element" : "element_not_found";
                throw new InvalidOperationException(reason + ": re-inspect the current panel tree.");
            }
            return element;
        }

        private static VisualElement RequireElement(PanelRecord record, int elementId)
        {
            if (elementId <= 0)
                throw new InvalidOperationException("A positive element ID from UIPanel inspection is required.");
            return ResolveElement(record, elementId, null, true);
        }

        private static bool BelongsTo(PanelRecord record, VisualElement element)
        {
            if (record == null || record.root == null || element == null || element.panel != record.panel)
                return false;
            VisualElement cursor = element;
            while (cursor != null)
            {
                if (ReferenceEquals(cursor, record.root))
                    return true;
                cursor = cursor.parent;
            }
            return false;
        }

        private static int ElementId(PanelRecord record, VisualElement element)
        {
            int id;
            if (record.ids.TryGetValue(element, out id))
                return id;
            id = record.nextElementId++;
            record.ids[element] = id;
            record.elements[id] = element;
            return id;
        }

        private static void RefreshDocumentRevision(PanelRecord record)
        {
            if (record == null || record.root == null)
                return;
            int fingerprint = StructureFingerprint(record.root);
            if (record.fingerprint != 0 && record.fingerprint != fingerprint)
                record.revision++;
            record.fingerprint = fingerprint;
        }

        private static Dictionary<string, object> PanelSummary(PanelRecord record, int elementCount)
        {
            string ownerName = record.editorWindow == null
                ? "UIDocument"
                : record.editorWindow.GetType().Name;
            Dictionary<string, object> result = new Dictionary<string, object>
            {
                { "panelId", record.id },
                { "kind", record.kind ?? "" },
                { "title", record.title ?? "" },
                { "owner", ownerName },
                { "elementCount", elementCount }
            };

            if (record.documents.Count > 0)
            {
                List<string> documentNames = new List<string>();
                for (int i = 0; i < record.documents.Count; i++)
                {
                    UIDocument document = record.documents[i];
                    string name = document == null || document.gameObject == null
                        ? ""
                        : document.gameObject.name;
                    if (!string.IsNullOrEmpty(name))
                        documentNames.Add(name);
                }
                if (documentNames.Count > 0)
                    result["documents"] = documentNames;
            }

            return result;
        }

        private static Dictionary<string, object> ElementSnapshot(
            PanelRecord record,
            VisualElement element,
            bool includeComputedStyle,
            bool includeMatchedRules,
            List<string> styleProperties,
            List<string> warnings)
        {
            List<string> classes = new List<string>();
            foreach (string className in element.GetClasses())
                classes.Add(className);
            VisualElement parent = element.parent;
            int childCount = 0;
            for (int i = 0; i < element.hierarchy.childCount; i++)
            {
                VisualElement child = element.hierarchy[i];
                if (IsOverlay(child))
                    continue;
                childCount++;
            }

            string text = ReadText(element);
            object value = JsonSafeValue(ReadControlValue(element));
            List<string> actions = ActionsFor(element);
            string source = UxmlSource(element);
            bool visible = IsVisible(element);
            bool focused = record.panel.focusController != null
                && ReferenceEquals(record.panel.focusController.focusedElement, element);
            string typeName = element.GetType().Name;
            string selectorHint = SelectorHint(element, classes);
            Dictionary<string, object> result = new Dictionary<string, object>
            {
                { "elementId", ElementId(record, element) },
                { "type", typeName },
                { "rect", RectArrayValue(element.worldBound) }
            };
            if (!string.Equals(selectorHint, typeName, StringComparison.Ordinal))
                result["selectorHint"] = selectorHint;
            if (parent != null && BelongsTo(record, parent))
                result["parentId"] = ElementId(record, parent);
            if (childCount > 0)
                result["childCount"] = childCount;
            if (!string.IsNullOrEmpty(text))
                result["text"] = LimitText(text, CompactTextLimit);
            if (value != null)
                result["value"] = CompactValue(value, CompactTextLimit);
            if (actions.Count > 0)
                result["actions"] = actions;
            if (!visible)
                result["visible"] = false;
            if (!element.enabledInHierarchy)
                result["enabled"] = false;
            if (focused)
                result["focused"] = true;
            if (!string.IsNullOrEmpty(source))
                result["uxmlSource"] = source;

            if (includeComputedStyle)
                result["computedStyle"] = ComputedStyle(element, styleProperties);
            if (includeMatchedRules)
            {
                List<object> rules = MatchedRules(element, warnings);
                if (rules.Count > 0)
                    result["matchedRules"] = rules;
            }
            return result;
        }

        private static Dictionary<string, string> ComputedStyle(
            VisualElement element,
            List<string> requestedProperties)
        {
            IResolvedStyle style = element.resolvedStyle;
            Dictionary<string, string> output = new Dictionary<string, string>(StringComparer.Ordinal)
            {
                { "display", style.display.ToString() },
                { "visibility", style.visibility.ToString() },
                { "opacity", Invariant(style.opacity) },
                { "position", style.position.ToString() },
                { "left", Invariant(style.left) },
                { "right", Invariant(style.right) },
                { "top", Invariant(style.top) },
                { "bottom", Invariant(style.bottom) },
                { "width", Invariant(style.width) },
                { "height", Invariant(style.height) },
                { "minWidth", Convert.ToString(style.minWidth, CultureInfo.InvariantCulture) },
                { "minHeight", Convert.ToString(style.minHeight, CultureInfo.InvariantCulture) },
                { "maxWidth", Convert.ToString(style.maxWidth, CultureInfo.InvariantCulture) },
                { "maxHeight", Convert.ToString(style.maxHeight, CultureInfo.InvariantCulture) },
                { "marginLeft", Invariant(style.marginLeft) },
                { "marginRight", Invariant(style.marginRight) },
                { "marginTop", Invariant(style.marginTop) },
                { "marginBottom", Invariant(style.marginBottom) },
                { "paddingLeft", Invariant(style.paddingLeft) },
                { "paddingRight", Invariant(style.paddingRight) },
                { "paddingTop", Invariant(style.paddingTop) },
                { "paddingBottom", Invariant(style.paddingBottom) },
                { "borderLeftWidth", Invariant(style.borderLeftWidth) },
                { "borderRightWidth", Invariant(style.borderRightWidth) },
                { "borderTopWidth", Invariant(style.borderTopWidth) },
                { "borderBottomWidth", Invariant(style.borderBottomWidth) },
                { "flexBasis", Convert.ToString(style.flexBasis, CultureInfo.InvariantCulture) },
                { "flexGrow", Invariant(style.flexGrow) },
                { "flexShrink", Invariant(style.flexShrink) },
                { "flexDirection", style.flexDirection.ToString() },
                { "flexWrap", style.flexWrap.ToString() },
                { "alignContent", style.alignContent.ToString() },
                { "alignItems", style.alignItems.ToString() },
                { "alignSelf", style.alignSelf.ToString() },
                { "justifyContent", style.justifyContent.ToString() },
                { "overflow", Convert.ToString(MemberValue(style, "overflow"), CultureInfo.InvariantCulture) ?? "" },
                { "color", ColorValue(style.color) },
                { "backgroundColor", ColorValue(style.backgroundColor) },
                { "fontSize", Invariant(style.fontSize) },
                { "whiteSpace", style.whiteSpace.ToString() }
            };
            string[] defaults =
            {
                "position", "flexDirection", "flexGrow", "flexShrink",
                "alignItems", "justifyContent", "color", "backgroundColor", "fontSize"
            };
            IEnumerable<string> selected = requestedProperties != null && requestedProperties.Count > 0
                ? (IEnumerable<string>)requestedProperties
                : defaults;
            Dictionary<string, string> filtered = new Dictionary<string, string>(StringComparer.Ordinal);
            foreach (string requested in selected)
            {
                string property = NormalizeStyleProperty(requested);
                string value;
                if (output.TryGetValue(property, out value))
                    filtered[property] = value;
            }
            return filtered;
        }

        private static List<object> MatchedRules(VisualElement element, List<string> warnings)
        {
            List<object> output = new List<object>();
            try
            {
                Type extractorType = typeof(VisualElement).Assembly.GetType(
                    "UnityEngine.UIElements.MatchedRulesExtractor",
                    false);
                if (extractorType == null)
                    throw new MissingMemberException("MatchedRulesExtractor is unavailable.");
                Func<StyleSheet, string> pathResolver = delegate(StyleSheet sheet)
                {
                    return sheet == null ? "" : AssetDatabase.GetAssetPath(sheet);
                };
                object extractor = Activator.CreateInstance(
                    extractorType,
                    BindingFlags.Instance | BindingFlags.Public | BindingFlags.NonPublic,
                    null,
                    new object[] { pathResolver },
                    CultureInfo.InvariantCulture);
                MethodInfo find = extractorType.GetMethod(
                    "FindMatchingRules",
                    BindingFlags.Instance | BindingFlags.Public | BindingFlags.NonPublic);
                MethodInfo get = extractorType.GetMethod(
                    "GetMatchedRules",
                    BindingFlags.Instance | BindingFlags.Public | BindingFlags.NonPublic);
                if (find == null || get == null)
                    throw new MissingMethodException("Unity matched-rule methods are unavailable.");
                find.Invoke(extractor, new object[] { element });
                IEnumerable rules = get.Invoke(extractor, null) as IEnumerable;
                if (rules == null)
                    return output;
                foreach (object rule in rules)
                {
                    if (rule == null || output.Count >= 64)
                        break;
                    Type ruleType = rule.GetType();
                    object matchRecord = FieldValue(ruleType, rule, "matchRecord");
                    object selector = MemberValue(matchRecord, "complexSelector");
                    object specificity = MemberValue(selector, "specificity");
                    string path = Convert.ToString(FieldValue(ruleType, rule, "fullPath")) ?? "";
                    if (string.IsNullOrEmpty(path))
                        path = Convert.ToString(FieldValue(ruleType, rule, "displayPath")) ?? "";
                    string selectorText = SelectorDescription(selector);
                    if (string.IsNullOrEmpty(path) && string.IsNullOrEmpty(selectorText))
                        continue;
                    Dictionary<string, object> item = new Dictionary<string, object>();
                    if (!string.IsNullOrEmpty(path))
                        item["path"] = path;
                    int line = Convert.ToInt32(FieldValue(ruleType, rule, "lineNumber"), CultureInfo.InvariantCulture);
                    if (line > 0)
                        item["line"] = line;
                    if (specificity != null)
                        item["specificity"] = Convert.ToInt32(specificity, CultureInfo.InvariantCulture);
                    if (!string.IsNullOrEmpty(selectorText))
                        item["selector"] = selectorText;
                    output.Add(item);
                }
            }
            catch (Exception exception)
            {
                string message = "Matched USS rules unavailable: " + RootMessage(exception);
                if (!warnings.Contains(message))
                    warnings.Add(message);
            }
            return output;
        }

        private static object FieldValue(Type type, object instance, string name)
        {
            FieldInfo field = type == null
                ? null
                : type.GetField(name, BindingFlags.Instance | BindingFlags.Public | BindingFlags.NonPublic);
            return field == null ? null : field.GetValue(instance);
        }

        private static object MemberValue(object instance, string name)
        {
            if (instance == null)
                return null;
            Type type = instance.GetType();
            FieldInfo field = type.GetField(name, BindingFlags.Instance | BindingFlags.Public | BindingFlags.NonPublic);
            if (field != null)
                return field.GetValue(instance);
            PropertyInfo property = type.GetProperty(name, BindingFlags.Instance | BindingFlags.Public | BindingFlags.NonPublic);
            return property == null ? null : property.GetValue(instance, null);
        }

        private static string SelectorDescription(object selector)
        {
            if (selector == null)
                return "";
            string text = Convert.ToString(selector, CultureInfo.InvariantCulture) ?? "";
            return text == selector.GetType().FullName || text == selector.GetType().Name ? "" : text;
        }

        private static VisualElement FindBySelector(VisualElement root, string selector)
        {
            VisualElement found = null;
            Walk(
                root,
                0,
                1000,
                delegate(VisualElement element, int depth)
                {
                    if (MatchesSelector(element, selector))
                    {
                        found = element;
                        return false;
                    }
                    return found == null;
                });
            return found;
        }

        private static bool MatchesSelector(VisualElement element, string selector)
        {
            selector = (selector ?? "").Trim();
            if (selector == "*")
                return true;
            if (selector.Length == 0)
                return false;
            string type = "";
            string name = "";
            string className = "";
            int hash = selector.IndexOf('#');
            int dot = selector.IndexOf('.');
            if (hash >= 0)
            {
                type = selector.Substring(0, hash);
                name = selector.Substring(hash + 1);
            }
            else if (dot >= 0)
            {
                type = selector.Substring(0, dot);
                className = selector.Substring(dot + 1);
            }
            else if (selector[0] == '#')
            {
                name = selector.Substring(1);
            }
            else if (selector[0] == '.')
            {
                className = selector.Substring(1);
            }
            else
            {
                type = selector;
            }
            bool typeMatches = type.Length == 0
                || string.Equals(element.GetType().Name, type, StringComparison.OrdinalIgnoreCase)
                || string.Equals(element.GetType().FullName, type, StringComparison.Ordinal);
            bool nameMatches = name.Length == 0 || string.Equals(element.name, name, StringComparison.Ordinal);
            bool classMatches = className.Length == 0 || element.ClassListContains(className);
            return typeMatches && nameMatches && classMatches;
        }

        private static string SelectorHint(VisualElement element, List<string> classes)
        {
            if (element == null)
                return "";
            if (!string.IsNullOrEmpty(element.name))
                return "#" + element.name;
            if (classes != null && classes.Count > 0)
                return element.GetType().Name + "." + classes[0];
            return element.GetType().Name;
        }

        private static void Walk(
            VisualElement root,
            int depth,
            int maxDepth,
            Func<VisualElement, int, bool> visitor)
        {
            if (root == null || IsOverlay(root))
                return;
            bool descend = visitor(root, depth);
            if (!descend || depth >= maxDepth)
                return;
            for (int i = 0; i < root.hierarchy.childCount; i++)
            {
                VisualElement child = root.hierarchy[i];
                if (!IsOverlay(child))
                    Walk(child, depth + 1, maxDepth, visitor);
            }
        }

        private static bool IsOverlay(VisualElement element)
        {
            return element != null && string.Equals(element.name, OverlayName, StringComparison.Ordinal);
        }

        private static int CountElements(VisualElement root)
        {
            int count = 0;
            Walk(root, 0, 10000, delegate(VisualElement element, int depth)
            {
                count++;
                return true;
            });
            return count;
        }

        private static int StructureFingerprint(VisualElement root)
        {
            unchecked
            {
                int fingerprint = 17;
                Walk(root, 0, 10000, delegate(VisualElement element, int depth)
                {
                    fingerprint = fingerprint * 31 + RuntimeHelpers.GetHashCode(element);
                    fingerprint = fingerprint * 31 + element.hierarchy.childCount;
                    return true;
                });
                return fingerprint;
            }
        }

        private static bool IsVisible(VisualElement element)
        {
            return element != null
                && element.resolvedStyle.display != DisplayStyle.None
                && element.resolvedStyle.visibility == Visibility.Visible
                && element.resolvedStyle.opacity > 0.001f
                && element.worldBound.width > 0.001f
                && element.worldBound.height > 0.001f;
        }

        private static bool IsInteractive(VisualElement element)
        {
            return element is Button
                || element is Toggle
                || element is TextField
                || element is DropdownField
                || element is ScrollView
                || HasWritableValue(element)
                || element.focusable
                || HasInteractiveCallbacks(element);
        }

        private static List<string> ActionsFor(VisualElement element)
        {
            List<string> actions = new List<string>();
            if (element is Button)
                actions.Add("click");
            if (element.focusable)
                actions.Add("focus");
            if (element is TextField)
            {
                actions.Add("type");
                actions.Add("setValue");
                actions.Add("press");
            }
            else if (element is Toggle)
            {
                actions.Add("toggle");
                actions.Add("setValue");
            }
            else if (element is DropdownField)
            {
                actions.Add("select");
                actions.Add("setValue");
            }
            else if (HasWritableValue(element))
            {
                actions.Add("setValue");
            }
            if (element is ScrollView)
                actions.Add("scroll");
            if (HasInteractiveCallbacks(element) && !actions.Contains("click"))
                actions.Add("click");
            if (HasInteractiveCallbacks(element))
                actions.Add("drag");
            return actions;
        }

        private static bool HasInteractiveCallbacks(VisualElement element)
        {
            if (element == null)
                return false;
            try
            {
                FieldInfo registryField = typeof(VisualElement).GetField(
                    "m_CallbackRegistry",
                    BindingFlags.Instance | BindingFlags.Public | BindingFlags.NonPublic);
                object registry = registryField == null ? null : registryField.GetValue(element);
                if (registry == null)
                    return false;
                Type registryType = registry.GetType();
                return CallbackListHasInteractiveType(registryType, registry, "m_TrickleDownCallbacks")
                    || CallbackListHasInteractiveType(registryType, registry, "m_BubbleUpCallbacks");
            }
            catch
            {
                return false;
            }
        }

        private static bool CallbackListHasInteractiveType(
            Type registryType,
            object registry,
            string fieldName)
        {
            object dynamicList = FieldValue(registryType, registry, fieldName);
            object callbackList = MemberValue(dynamicList, "m_Callbacks");
            Array callbacks = MemberValue(callbackList, "m_Array") as Array;
            object rawCount = MemberValue(callbackList, "m_Count");
            int count = rawCount == null ? 0 : Convert.ToInt32(rawCount, CultureInfo.InvariantCulture);
            if (callbacks == null || count <= 0)
                return false;
            count = Math.Min(count, callbacks.Length);
            for (int i = 0; i < count; i++)
            {
                object callback = callbacks.GetValue(i);
                object rawId = MemberValue(callback, "eventTypeId");
                if (rawId != null
                    && InteractiveEventTypeIds.Contains(Convert.ToInt64(rawId, CultureInfo.InvariantCulture)))
                    return true;
            }
            return false;
        }

        private static string ReadText(VisualElement element)
        {
            TextElement text = element as TextElement;
            if (text != null)
                return text.text ?? "";
            PropertyInfo label = element.GetType().GetProperty("label", BindingFlags.Instance | BindingFlags.Public);
            object value = label == null ? null : label.GetValue(element, null);
            return value == null ? "" : Convert.ToString(value, CultureInfo.InvariantCulture);
        }

        private static object ReadControlValue(VisualElement element)
        {
            PropertyInfo property = ValueProperty(element);
            if (property == null || !property.CanRead)
                return null;
            try
            {
                return property.GetValue(element, null);
            }
            catch
            {
                return null;
            }
        }

        private static bool HasWritableValue(VisualElement element)
        {
            PropertyInfo property = ValueProperty(element);
            return property != null && property.CanRead && property.CanWrite;
        }

        private static PropertyInfo ValueProperty(VisualElement element)
        {
            return element == null
                ? null
                : element.GetType().GetProperty("value", BindingFlags.Instance | BindingFlags.Public);
        }

        private static void SetControlValue(VisualElement element, object rawValue)
        {
            PropertyInfo property = ValueProperty(element);
            if (property == null || !property.CanWrite)
                throw new InvalidOperationException(element.GetType().Name + " does not expose a writable value property.");
            object converted = ConvertValue(rawValue, property.PropertyType);
            try
            {
                property.SetValue(element, converted, null);
            }
            catch (Exception exception)
            {
                throw new InvalidOperationException(
                    "Unable to set " + element.GetType().Name + ".value: " + RootMessage(exception),
                    exception);
            }
        }

        private static object ConvertValue(object value, Type targetType)
        {
            Type nullable = Nullable.GetUnderlyingType(targetType);
            Type effective = nullable ?? targetType;
            if (value == null)
            {
                if (!effective.IsValueType || nullable != null)
                    return null;
                return Activator.CreateInstance(effective);
            }
            if (effective.IsInstanceOfType(value))
                return value;
            string text = Convert.ToString(value, CultureInfo.InvariantCulture);
            if (effective == typeof(string))
                return text;
            if (effective.IsEnum)
                return Enum.Parse(effective, NormalizeEnum(text), true);
            return Convert.ChangeType(value, effective, CultureInfo.InvariantCulture);
        }

        private static void TypeText(VisualElement element, string text, bool append)
        {
            PropertyInfo property = ValueProperty(element);
            if (property == null || property.PropertyType != typeof(string) || !property.CanWrite)
                throw new InvalidOperationException("type requires a text control with a writable string value.");
            string current = append ? Convert.ToString(property.GetValue(element, null)) ?? "" : "";
            element.Focus();
            property.SetValue(element, current + text, null);
        }

        private static void Click(VisualElement element)
        {
            Vector2 position = element.worldBound.center;
            Button button = element as Button;
            if (button != null && button.clickable != null)
            {
                using (MouseUpEvent clickEvent = MouseUpEvent.GetPooled(new Event
                {
                    type = EventType.MouseUp,
                    mousePosition = position,
                    button = 0,
                    clickCount = 1
                }))
                {
                    clickEvent.target = button;
                    MethodInfo simulate = typeof(Clickable).GetMethod(
                        "SimulateSingleClick",
                        BindingFlags.Instance | BindingFlags.Public | BindingFlags.NonPublic);
                    if (simulate != null)
                    {
                        simulate.Invoke(button.clickable, new object[] { clickEvent, 0 });
                        SendClickEvent(button);
                        return;
                    }
                    MethodInfo invoke = typeof(Clickable).GetMethod(
                        "Invoke",
                        BindingFlags.Instance | BindingFlags.Public | BindingFlags.NonPublic);
                    if (invoke != null)
                    {
                        invoke.Invoke(button.clickable, new object[] { clickEvent });
                        SendClickEvent(button);
                        return;
                    }
                }
            }
            SendMouse(element, EventType.MouseDown, position, 0);
            SendMouse(element, EventType.MouseUp, position, 0);
            SendClickEvent(element);
        }

        private static void SendClickEvent(VisualElement target)
        {
            using (ClickEvent click = ClickEvent.GetPooled())
            {
                click.target = target;
                target.SendEvent(click);
            }
        }

        private static void Drag(VisualElement source, VisualElement destination)
        {
            Vector2 from = source.worldBound.center;
            Vector2 to = destination.worldBound.center;
            SendMouse(source, EventType.MouseDown, from, 0);
            for (int i = 1; i <= 4; i++)
            {
                Vector2 point = Vector2.Lerp(from, to, i / 4f);
                SendMouse(source, EventType.MouseMove, point, 0);
            }
            SendMouse(destination, EventType.MouseUp, to, 0);
        }

        private static void SendMouse(VisualElement target, EventType type, Vector2 position, int button)
        {
            Event systemEvent = new Event
            {
                type = type,
                mousePosition = position,
                button = button,
                clickCount = 1
            };
            if (type == EventType.MouseDown)
            {
                using (MouseDownEvent evt = MouseDownEvent.GetPooled(systemEvent))
                {
                    evt.target = target;
                    target.SendEvent(evt);
                }
            }
            else if (type == EventType.MouseUp)
            {
                using (MouseUpEvent evt = MouseUpEvent.GetPooled(systemEvent))
                {
                    evt.target = target;
                    target.SendEvent(evt);
                }
            }
            else
            {
                using (MouseMoveEvent evt = MouseMoveEvent.GetPooled(systemEvent))
                {
                    evt.target = target;
                    target.SendEvent(evt);
                }
            }
        }

        private static void PressKey(VisualElement element, string key)
        {
            KeyCode keyCode;
            if (string.IsNullOrWhiteSpace(key)
                || !Enum.TryParse(key.Replace("-", ""), true, out keyCode))
            {
                throw new InvalidOperationException("Unknown Unity KeyCode: " + (key ?? "") + ".");
            }
            element.Focus();
            using (KeyDownEvent down = KeyDownEvent.GetPooled('\0', keyCode, EventModifiers.None))
            {
                down.target = element;
                element.SendEvent(down);
            }
            using (KeyUpEvent up = KeyUpEvent.GetPooled('\0', keyCode, EventModifiers.None))
            {
                up.target = element;
                element.SendEvent(up);
            }
        }

        private static bool EvaluateWait(
            string condition,
            string expected,
            VisualElement element,
            ref Rect lastRect,
            ref bool hasLastRect,
            ref int stable,
            int requiredStableFrames)
        {
            switch (condition)
            {
                case "exists": return element != null;
                case "missing": return element == null;
                case "visible": return IsVisible(element);
                case "hidden": return element != null && !IsVisible(element);
                case "enabled": return element != null && element.enabledInHierarchy;
                case "disabled": return element != null && !element.enabledInHierarchy;
                case "text": return element != null && string.Equals(ReadText(element), expected ?? "", StringComparison.Ordinal);
                case "value":
                    return element != null && string.Equals(
                        Convert.ToString(ReadControlValue(element), CultureInfo.InvariantCulture) ?? "",
                        expected ?? "",
                        StringComparison.Ordinal);
                case "layoutstable":
                    if (element == null)
                        return false;
                    Rect current = element.worldBound;
                    if (hasLastRect && Approximately(lastRect, current))
                        stable++;
                    else
                        stable = 1;
                    lastRect = current;
                    hasLastRect = true;
                    return stable >= requiredStableFrames;
                default:
                    return false;
            }
        }

        private static void CompleteWait(
            TaskCompletionSource<Dictionary<string, object>> source,
            EditorApplication.CallbackFunction callback,
            WaitRequest request,
            PanelRecord record,
            VisualElement element,
            double started,
            int stableFrames)
        {
            EditorApplication.update -= callback;
            string condition = (request.condition ?? "").Trim().ToLowerInvariant();
            Dictionary<string, object> result = new Dictionary<string, object>
            {
                { "elapsedMs", (int)Math.Round((EditorApplication.timeSinceStartup - started) * 1000.0) }
            };
            if (element != null && record != null)
                result["elementId"] = ElementId(record, element);
            if (condition == "text" && element != null)
                result["text"] = LimitText(ReadText(element), CompactTextLimit);
            else if (condition == "value" && element != null)
                result["value"] = CompactValue(JsonSafeValue(ReadControlValue(element)), CompactTextLimit);
            else if ((condition == "visible" || condition == "hidden") && element != null)
                result["visible"] = IsVisible(element);
            else if ((condition == "enabled" || condition == "disabled") && element != null)
                result["enabled"] = element.enabledInHierarchy;
            else if (condition == "layoutstable" && element != null)
            {
                result["stableFrames"] = stableFrames;
                result["rect"] = RectArrayValue(element.worldBound);
            }
            source.TrySetResult(result);
        }

        private static bool Approximately(Rect left, Rect right)
        {
            return Mathf.Abs(left.x - right.x) < 0.1f
                && Mathf.Abs(left.y - right.y) < 0.1f
                && Mathf.Abs(left.width - right.width) < 0.1f
                && Mathf.Abs(left.height - right.height) < 0.1f;
        }

        private static VisualElement CreateOverlay(
            PanelRecord record,
            List<VisualElement> targets,
            bool numbered)
        {
            VisualElement overlay = new VisualElement
            {
                name = OverlayName,
                pickingMode = PickingMode.Ignore
            };
            overlay.style.position = Position.Absolute;
            overlay.style.left = 0;
            overlay.style.top = 0;
            overlay.style.right = 0;
            overlay.style.bottom = 0;
            Color accent = new Color(0.18f, 0.64f, 1f, 1f);
            for (int i = 0; i < targets.Count; i++)
            {
                VisualElement target = targets[i];
                Rect world = target.worldBound;
                Vector2 local = record.root.WorldToLocal(world.position);
                VisualElement box = new VisualElement { pickingMode = PickingMode.Ignore };
                box.style.position = Position.Absolute;
                box.style.left = local.x;
                box.style.top = local.y;
                box.style.width = world.width;
                box.style.height = world.height;
                box.style.borderLeftWidth = 2;
                box.style.borderRightWidth = 2;
                box.style.borderTopWidth = 2;
                box.style.borderBottomWidth = 2;
                box.style.borderLeftColor = accent;
                box.style.borderRightColor = accent;
                box.style.borderTopColor = accent;
                box.style.borderBottomColor = accent;
                if (numbered)
                {
                    Label label = new Label((i + 1).ToString(CultureInfo.InvariantCulture))
                    {
                        pickingMode = PickingMode.Ignore
                    };
                    label.style.position = Position.Absolute;
                    label.style.left = -2;
                    label.style.top = -18;
                    label.style.height = 18;
                    label.style.minWidth = 18;
                    label.style.paddingLeft = 4;
                    label.style.paddingRight = 4;
                    label.style.backgroundColor = accent;
                    label.style.color = Color.black;
                    label.style.unityTextAlign = TextAnchor.MiddleCenter;
                    box.Add(label);
                }
                overlay.Add(box);
            }
            return overlay;
        }

        private static void RemoveOverlay(PanelRecord record)
        {
            if (record == null || record.overlay == null)
                return;
            record.overlay.RemoveFromHierarchy();
            record.overlay = null;
        }

        private static object ParseStyleValue(Type styleType, string value)
        {
            value = (value ?? "").Trim();
            if (styleType == typeof(StyleLength))
            {
                StyleKeyword keyword;
                if (TryStyleKeyword(value, out keyword))
                    return new StyleLength(keyword);
                return new StyleLength(ParseLength(value));
            }
            if (styleType == typeof(StyleFloat))
            {
                StyleKeyword keyword;
                if (TryStyleKeyword(value, out keyword))
                    return new StyleFloat(keyword);
                float number;
                if (!float.TryParse(value, NumberStyles.Float, CultureInfo.InvariantCulture, out number))
                    throw new FormatException("Expected a number, received '" + value + "'.");
                return new StyleFloat(number);
            }
            if (styleType == typeof(StyleInt))
            {
                StyleKeyword keyword;
                if (TryStyleKeyword(value, out keyword))
                    return new StyleInt(keyword);
                int number;
                if (!int.TryParse(value, NumberStyles.Integer, CultureInfo.InvariantCulture, out number))
                    throw new FormatException("Expected an integer, received '" + value + "'.");
                return new StyleInt(number);
            }
            if (styleType == typeof(StyleColor))
            {
                StyleKeyword keyword;
                if (TryStyleKeyword(value, out keyword))
                    return new StyleColor(keyword);
                return new StyleColor(ParseColor(value));
            }
            if (styleType.IsGenericType
                && styleType.GetGenericTypeDefinition() == typeof(StyleEnum<>))
            {
                Type enumType = styleType.GetGenericArguments()[0];
                string normalized = NormalizeEnum(value);
                string[] names = Enum.GetNames(enumType);
                for (int i = 0; i < names.Length; i++)
                {
                    if (string.Equals(NormalizeEnum(names[i]), normalized, StringComparison.OrdinalIgnoreCase))
                    {
                        object enumValue = Enum.Parse(enumType, names[i], true);
                        return Activator.CreateInstance(styleType, new object[] { enumValue });
                    }
                }
                StyleKeyword keyword;
                if (TryStyleKeyword(value, out keyword))
                    return StyleKeywordValue(styleType, keyword);
                throw new FormatException("Unknown " + enumType.Name + " value '" + value + "'.");
            }
            throw new InvalidOperationException("Unsupported Unity style value type: " + styleType.FullName + ".");
        }

        private static object StyleKeywordValue(Type styleType, StyleKeyword keyword)
        {
            try
            {
                return Activator.CreateInstance(styleType, new object[] { keyword });
            }
            catch (Exception exception)
            {
                throw new InvalidOperationException(
                    styleType.Name + " does not accept the style keyword " + keyword + ".",
                    exception);
            }
        }

        private static bool TryStyleKeyword(string value, out StyleKeyword keyword)
        {
            string normalized = (value ?? "").Trim().ToLowerInvariant();
            switch (normalized)
            {
                case "auto": keyword = StyleKeyword.Auto; return true;
                case "none": keyword = StyleKeyword.None; return true;
                case "initial": keyword = StyleKeyword.Initial; return true;
                case "null":
                case "unset": keyword = StyleKeyword.Null; return true;
                default: keyword = StyleKeyword.Undefined; return false;
            }
        }

        private static Length ParseLength(string value)
        {
            string text = value.Trim().ToLowerInvariant();
            LengthUnit unit = LengthUnit.Pixel;
            if (text.EndsWith("px", StringComparison.Ordinal))
                text = text.Substring(0, text.Length - 2).Trim();
            else if (text.EndsWith("%", StringComparison.Ordinal))
            {
                unit = LengthUnit.Percent;
                text = text.Substring(0, text.Length - 1).Trim();
            }
            float number;
            if (!float.TryParse(text, NumberStyles.Float, CultureInfo.InvariantCulture, out number))
                throw new FormatException("Expected a px or % length, received '" + value + "'.");
            return new Length(number, unit);
        }

        private static Color ParseColor(string value)
        {
            string normalized = value.Trim().ToLowerInvariant();
            if (normalized == "transparent")
                return Color.clear;
            if (normalized == "black")
                return Color.black;
            if (normalized == "white")
                return Color.white;
            if (normalized == "red")
                return Color.red;
            if (normalized == "green")
                return Color.green;
            if (normalized == "blue")
                return Color.blue;
            if (normalized == "yellow")
                return Color.yellow;
            if (normalized == "gray" || normalized == "grey")
                return Color.gray;
            Color color;
            if (ColorUtility.TryParseHtmlString(value, out color))
                return color;
            throw new FormatException("Expected an HTML color, received '" + value + "'.");
        }

        private static string NormalizeStyleProperty(string property)
        {
            property = (property ?? "").Trim();
            if (property.StartsWith("--", StringComparison.Ordinal))
                throw new InvalidOperationException("Custom USS variables require a persistent USS source edit.");
            if (property.StartsWith("-unity-", StringComparison.OrdinalIgnoreCase))
                property = "unity-" + property.Substring(7);
            if (property.IndexOf('-') < 0)
                return property;
            string[] parts = property.Split('-');
            string output = parts[0];
            for (int i = 1; i < parts.Length; i++)
            {
                if (parts[i].Length == 0)
                    continue;
                output += char.ToUpperInvariant(parts[i][0]) + parts[i].Substring(1);
            }
            return output;
        }

        private static string NormalizeEnum(string value)
        {
            return (value ?? "").Replace("-", "").Replace("_", "").Replace(" ", "");
        }

        private static string UxmlSource(VisualElement element)
        {
            try
            {
                PropertyInfo property = typeof(VisualElement).GetProperty(
                    "visualTreeAssetSource",
                    BindingFlags.Instance | BindingFlags.Public | BindingFlags.NonPublic);
                UnityEngine.Object asset = property == null
                    ? null
                    : property.GetValue(element, null) as UnityEngine.Object;
                return asset == null ? "" : AssetDatabase.GetAssetPath(asset);
            }
            catch
            {
                return "";
            }
        }

        private static float[] RectArrayValue(Rect rect)
        {
            return new float[]
            {
                Round(rect.x),
                Round(rect.y),
                Round(rect.width),
                Round(rect.height)
            };
        }

        private static string LimitText(string value, int maximum)
        {
            value = value ?? "";
            if (maximum <= 3 || value.Length <= maximum)
                return value;
            return value.Substring(0, maximum - 3) + "...";
        }

        private static object CompactValue(object value, int maximumStringLength)
        {
            string text = value as string;
            return text == null ? value : (object)LimitText(text, maximumStringLength);
        }

        private static object JsonSafeValue(object value)
        {
            if (value == null || value is string || value is bool
                || value is byte || value is sbyte || value is short || value is ushort
                || value is int || value is uint || value is long || value is ulong
                || value is float || value is double || value is decimal)
            {
                return value;
            }
            if (value.GetType().IsEnum)
                return value.ToString();
            return Convert.ToString(value, CultureInfo.InvariantCulture);
        }

        private static string WindowTitle(EditorWindow window)
        {
            if (window == null)
                return "";
            if (window.titleContent != null && !string.IsNullOrWhiteSpace(window.titleContent.text))
                return window.titleContent.text;
            return window.GetType().Name;
        }

        private static string CurrentEditorStatus()
        {
            if (!EditorApplication.isPlaying)
                return "editing";
            return EditorApplication.isPaused ? "playing_paused" : "playing";
        }

        private static string RuntimePanelTitle(List<UIDocument> documents)
        {
            if (documents == null || documents.Count == 0)
                return "Runtime UI";
            UIDocument first = documents[0];
            string name = first == null || first.gameObject == null ? "Runtime UI" : first.gameObject.name;
            return documents.Count == 1
                ? name
                : name + " +" + (documents.Count - 1).ToString(CultureInfo.InvariantCulture);
        }

        private static float Round(float value)
        {
            if (float.IsNaN(value) || float.IsInfinity(value))
                return value;
            return (float)Math.Round(value, 2);
        }

        private static string Invariant(float value)
        {
            return Round(value).ToString("0.##", CultureInfo.InvariantCulture);
        }

        private static string ColorValue(Color color)
        {
            return "#" + ColorUtility.ToHtmlStringRGBA(color);
        }

        private static string RootMessage(Exception exception)
        {
            while (exception is TargetInvocationException && exception.InnerException != null)
                exception = exception.InnerException;
            return exception == null ? "Unknown error." : exception.Message;
        }

        private static void ValidateUnityVersion()
        {
            string[] parts = (Application.unityVersion ?? "").Split('.');
            int major;
            int minor;
            if (parts.Length < 2
                || !int.TryParse(parts[0], out major)
                || !int.TryParse(parts[1], out minor)
                || major < MinimumUnityMajor
                || (major == MinimumUnityMajor && minor < MinimumUnityMinor))
            {
                throw new InvalidOperationException(
                    "UI Toolkit DevTools requires Unity 6000.3 or newer; current version is "
                    + Application.unityVersion + ".");
            }
        }
    }
}
