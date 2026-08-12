using System;
using System.Collections;
using System.Collections.Generic;
using System.Globalization;
using System.Reflection;
using System.Text;
using System.Threading.Tasks;

using UnityEngine;

namespace Locus.Skills
{
    [AttributeUsage(AttributeTargets.Property)]
    internal sealed class UIJsonOmitDefaultAttribute : Attribute { }

    /// <summary>
    /// Public, composable UI Toolkit debugging API loaded with the Skill.
    /// Call it from the built-in unity_execute tool; the Skill registers no
    /// package-specific tools.
    /// </summary>
    public static class UIToolkitApi
    {
        public static UIToolkitSession Open()
        {
            return new UIToolkitSession();
        }

        /// <summary>Serialize API results and anonymous projections as compact JSON.</summary>
        public static string Json(object value)
        {
            return UIToolkitJson.Serialize(value);
        }
    }

    public enum UIWaitCondition
    {
        Exists,
        Missing,
        Visible,
        Hidden,
        Enabled,
        Disabled,
        Text,
        Value,
        LayoutStable
    }

    public sealed class UIQuery
    {
        public int Depth { get; set; } = 2;
        public int MaxElements { get; set; } = 80;
        public bool IncludeHidden { get; set; }
        public bool InteractiveOnly { get; set; }
        public bool IncludeComputedStyle { get; set; }
        public bool IncludeMatchedRules { get; set; }
        public IList<string> StyleProperties { get; set; }

        public static UIQuery One()
        {
            return new UIQuery { Depth = 1, MaxElements = 1, IncludeHidden = true };
        }

        internal UIToolkitDevTools.InspectRequest ToRequest(string panelId, int elementId, string selector)
        {
            return new UIToolkitDevTools.InspectRequest
            {
                panelId = panelId,
                elementId = elementId,
                selector = selector,
                depth = Math.Max(1, Math.Min(20, Depth)),
                maxElements = Math.Max(1, Math.Min(2000, MaxElements)),
                includeHidden = IncludeHidden,
                interactiveOnly = InteractiveOnly,
                includeComputedStyle = IncludeComputedStyle,
                includeMatchedRules = IncludeMatchedRules,
                styleProperties = StyleProperties == null
                    ? null
                    : new List<string>(StyleProperties)
            };
        }
    }

    public sealed class UIStyleChange
    {
        public string Property { get; private set; }
        public string Value { get; private set; }

        public UIStyleChange(string property, string value)
        {
            if (string.IsNullOrWhiteSpace(property))
                throw new ArgumentException("A style property is required.", "property");
            Property = property;
            Value = value;
        }
    }

    public sealed class UIPanelInfo
    {
        public string Id { get; internal set; }
        public string Kind { get; internal set; }
        public string Title { get; internal set; }
        public string Owner { get; internal set; }
        public int ElementCount { get; internal set; }
        public IList<string> Documents { get; internal set; }
    }

    public sealed class UIMatchedRule
    {
        public string Path { get; internal set; }
        [UIJsonOmitDefault]
        public int Line { get; internal set; }
        [UIJsonOmitDefault]
        public int Specificity { get; internal set; }
        public string Selector { get; internal set; }
    }

    public sealed class UIElementInfo
    {
        public int Id { get; internal set; }
        public int? ParentId { get; internal set; }
        public string Type { get; internal set; }
        public string Selector { get; internal set; }
        [UIJsonOmitDefault]
        public int ChildCount { get; internal set; }
        public float[] Rect { get; internal set; }
        public string Text { get; internal set; }
        public object Value { get; internal set; }
        public IList<string> Actions { get; internal set; }
        /// <summary>Only populated when the element is hidden.</summary>
        public bool? Visible { get; internal set; }
        /// <summary>Only populated when the element is disabled.</summary>
        public bool? Enabled { get; internal set; }
        /// <summary>Only populated when the element owns focus.</summary>
        public bool? Focused { get; internal set; }
        public string UxmlSource { get; internal set; }
        public IDictionary<string, string> ComputedStyle { get; internal set; }
        public IList<UIMatchedRule> MatchedRules { get; internal set; }
    }

    public sealed class UIInspection
    {
        public string PanelId { get; internal set; }
        public int Revision { get; internal set; }
        public int ScopeElementId { get; internal set; }
        [UIJsonOmitDefault]
        public bool Truncated { get; internal set; }
        public IList<UIElementInfo> Elements { get; internal set; }
        public IList<string> Warnings { get; internal set; }
    }

    public sealed class UIActionResult
    {
        public int ElementId { get; internal set; }
        public int Revision { get; internal set; }
        public object Value { get; internal set; }
        public bool? Focused { get; internal set; }
        public float[] ScrollOffset { get; internal set; }
        public int? TargetElementId { get; internal set; }
    }

    public sealed class UIWaitResult
    {
        public int ElapsedMs { get; internal set; }
        public int? ElementId { get; internal set; }
        public string Text { get; internal set; }
        public object Value { get; internal set; }
        public bool? Visible { get; internal set; }
        public bool? Enabled { get; internal set; }
        public int? StableFrames { get; internal set; }
        public float[] Rect { get; internal set; }
    }

    public sealed class UIHighlightResult
    {
        public bool Cleared { get; internal set; }
        public int Highlighted { get; internal set; }
        public string CaptureTarget { get; internal set; }
        public string RequestEditorStatus { get; internal set; }
        public string WindowTitle { get; internal set; }
    }

    public sealed class UIToolkitSession
    {
        private readonly object _panelSnapshotLock = new object();
        private List<UIPanel> _panelSnapshot;

        public IList<UIPanel> Panels(bool includeEmpty = false)
        {
            List<UIPanel> snapshot = PanelSnapshot();
            List<UIPanel> output = new List<UIPanel>();
            for (int i = 0; i < snapshot.Count; i++)
            {
                if (includeEmpty || snapshot[i].ElementCount > 1)
                    output.Add(snapshot[i]);
            }
            return output;
        }

        public IList<UIPanel> RefreshPanels(bool includeEmpty = false)
        {
            lock (_panelSnapshotLock)
                _panelSnapshot = null;
            return Panels(includeEmpty);
        }

        private List<UIPanel> PanelSnapshot()
        {
            lock (_panelSnapshotLock)
            {
                if (_panelSnapshot != null)
                    return _panelSnapshot;

                Dictionary<string, object> raw = UIToolkitDevTools.ListPanels(
                    new UIToolkitDevTools.ListPanelsRequest { includeEmpty = true });
                List<UIPanel> output = new List<UIPanel>();
                IList values = UIConvert.List(raw, "panels");
                for (int i = 0; i < values.Count; i++)
                {
                    IDictionary<string, object> item = UIConvert.Map(values[i]);
                    if (item != null)
                        output.Add(new UIPanel(this, UIConvert.PanelInfo(item)));
                }
                _panelSnapshot = output;
                return _panelSnapshot;
            }
        }

        public UIPanel Panel(string panelId)
        {
            IList<UIPanel> panels = Panels(true);
            for (int i = 0; i < panels.Count; i++)
            {
                if (string.Equals(panels[i].Id, panelId, StringComparison.Ordinal))
                    return panels[i];
            }
            throw new InvalidOperationException("panel_not_found: call RefreshPanels() and select the panel again.");
        }

        public UIPanel FindPanel(string titleOrOwner)
        {
            if (string.IsNullOrWhiteSpace(titleOrOwner))
                throw new ArgumentException("A panel title, owner, document name, or ID is required.", "titleOrOwner");
            string query = titleOrOwner.Trim();
            IList<UIPanel> panels = Panels(true);
            UIPanel partial = null;
            for (int i = 0; i < panels.Count; i++)
            {
                UIPanel panel = panels[i];
                if (string.Equals(panel.Id, query, StringComparison.OrdinalIgnoreCase)
                    || string.Equals(panel.Title, query, StringComparison.OrdinalIgnoreCase)
                    || string.Equals(panel.Owner, query, StringComparison.OrdinalIgnoreCase)
                    || Contains(panel.Documents, query, true))
                    return panel;
                if (partial == null
                    && (Contains(panel.Title, query, false)
                        || Contains(panel.Owner, query, false)
                        || Contains(panel.Documents, query, false)))
                    partial = panel;
            }
            if (partial != null)
                return partial;
            throw new InvalidOperationException("panel_not_found: " + query + ".");
        }

        public UIPanel FindPanel(Predicate<UIPanelInfo> predicate)
        {
            if (predicate == null)
                throw new ArgumentNullException("predicate");
            IList<UIPanel> panels = Panels(true);
            for (int i = 0; i < panels.Count; i++)
            {
                if (predicate(panels[i].Info))
                    return panels[i];
            }
            throw new InvalidOperationException("panel_not_found: no panel matched the predicate.");
        }

        private static bool Contains(string value, string query, bool exact)
        {
            if (string.IsNullOrEmpty(value))
                return false;
            return exact
                ? string.Equals(value, query, StringComparison.OrdinalIgnoreCase)
                : value.IndexOf(query, StringComparison.OrdinalIgnoreCase) >= 0;
        }

        private static bool Contains(IList<string> values, string query, bool exact)
        {
            if (values == null)
                return false;
            for (int i = 0; i < values.Count; i++)
            {
                if (Contains(values[i], query, exact))
                    return true;
            }
            return false;
        }
    }

    public sealed class UIPanel
    {
        private readonly UIToolkitSession _session;

        internal UIPanel(UIToolkitSession session, UIPanelInfo info)
        {
            _session = session;
            Info = info;
        }

        public UIPanelInfo Info { get; private set; }
        public string Id { get { return Info.Id; } }
        public string Kind { get { return Info.Kind; } }
        public string Title { get { return Info.Title; } }
        public string Owner { get { return Info.Owner; } }
        public int ElementCount { get { return Info.ElementCount; } }
        public IList<string> Documents { get { return Info.Documents; } }

        public UIInspection Inspect(UIQuery query = null)
        {
            return InspectCore(0, null, query ?? new UIQuery());
        }

        public UIElement Root(UIQuery query = null)
        {
            UIInspection inspection = InspectCore(0, null, query ?? UIQuery.One());
            return ElementFromInspection(inspection, "root");
        }

        public UIElement Find(string selector, UIQuery query = null)
        {
            if (string.IsNullOrWhiteSpace(selector))
                throw new ArgumentException("A UI Toolkit selector is required.", "selector");
            UIInspection inspection = InspectCore(0, selector, query ?? UIQuery.One());
            return ElementFromInspection(inspection, selector);
        }

        public UIElement Element(int elementId, UIQuery query = null)
        {
            if (elementId <= 0)
                throw new ArgumentOutOfRangeException("elementId");
            UIInspection inspection = InspectCore(elementId, null, query ?? UIQuery.One());
            return ElementFromInspection(inspection, "elementId=" + elementId);
        }

        public Task<UIWaitResult> WaitAsync(
            string selector,
            UIWaitCondition condition,
            string expected = null,
            int timeoutMs = 10000,
            int stableFrames = 3)
        {
            return WaitCoreAsync(0, selector, condition, expected, timeoutMs, stableFrames);
        }

        public UIHighlightResult HighlightInteractions(int maxElements = 40)
        {
            return UIConvert.Highlight(UIToolkitDevTools.Highlight(
                new UIToolkitDevTools.HighlightRequest
                {
                    panelId = Id,
                    operation = "interactions",
                    maxElements = maxElements
                }));
        }

        public UIHighlightResult ClearHighlight()
        {
            return UIConvert.Highlight(UIToolkitDevTools.Highlight(
                new UIToolkitDevTools.HighlightRequest
                {
                    panelId = Id,
                    operation = "clear"
                }));
        }

        internal UIInspection InspectCore(int elementId, string selector, UIQuery query)
        {
            Dictionary<string, object> raw = UIToolkitDevTools.Inspect(
                query.ToRequest(Id, elementId, selector));
            return UIConvert.Inspection(raw);
        }

        internal async Task<UIWaitResult> WaitCoreAsync(
            int elementId,
            string selector,
            UIWaitCondition condition,
            string expected,
            int timeoutMs,
            int stableFrames)
        {
            Dictionary<string, object> raw = await UIToolkitDevTools.Wait(
                new UIToolkitDevTools.WaitRequest
                {
                    panelId = Id,
                    elementId = elementId,
                    selector = selector,
                    condition = UIConvert.WaitCondition(condition),
                    expected = expected,
                    timeoutMs = timeoutMs,
                    stableFrames = stableFrames
                });
            return UIConvert.Wait(raw);
        }

        internal UIActionResult Action(UIToolkitDevTools.ActionRequest request)
        {
            request.panelId = Id;
            return UIConvert.Action(UIToolkitDevTools.Action(request));
        }

        internal UIElement ElementFromInspection(UIInspection inspection, string description)
        {
            if (inspection.Elements == null || inspection.Elements.Count == 0)
                throw new InvalidOperationException("element_not_found: " + description + ".");
            return new UIElement(this, inspection.Elements[0]);
        }
    }

    public sealed class UIElement
    {
        private readonly UIPanel _panel;

        internal UIElement(UIPanel panel, UIElementInfo info)
        {
            _panel = panel;
            Info = info;
        }

        public UIElementInfo Info { get; private set; }
        public UIPanel Panel { get { return _panel; } }
        public int Id { get { return Info.Id; } }

        public UIElementInfo Refresh(UIQuery query = null)
        {
            UIInspection inspection = _panel.InspectCore(Id, null, query ?? UIQuery.One());
            if (inspection.Elements == null || inspection.Elements.Count == 0)
                throw new InvalidOperationException("stale_element: re-inspect the panel.");
            Info = inspection.Elements[0];
            return Info;
        }

        public UIInspection InspectSubtree(UIQuery query = null)
        {
            return _panel.InspectCore(Id, null, query ?? new UIQuery());
        }

        public UIActionResult Click()
        {
            return Act("click");
        }

        public UIActionResult Focus()
        {
            return Act("focus");
        }

        public UIActionResult SetValue(object value)
        {
            return Act("setValue", value);
        }

        public UIActionResult Select(object value)
        {
            return Act("select", value);
        }

        public UIActionResult Toggle(bool? value = null)
        {
            return Act("toggle", value.HasValue ? (object)value.Value : null);
        }

        public UIActionResult Type(string text, bool append = false)
        {
            UIToolkitDevTools.ActionRequest request = Request("type");
            request.text = text ?? "";
            request.append = append;
            return _panel.Action(request);
        }

        public UIActionResult Press(string key)
        {
            UIToolkitDevTools.ActionRequest request = Request("press");
            request.key = key;
            return _panel.Action(request);
        }

        public UIActionResult Scroll(float x, float y)
        {
            UIToolkitDevTools.ActionRequest request = Request("scroll");
            request.x = x;
            request.y = y;
            return _panel.Action(request);
        }

        public UIActionResult DragTo(UIElement destination)
        {
            if (destination == null)
                throw new ArgumentNullException("destination");
            if (!ReferenceEquals(destination.Panel, Panel))
                throw new InvalidOperationException("Drag source and destination must belong to the same panel.");
            UIToolkitDevTools.ActionRequest request = Request("drag");
            request.targetElementId = destination.Id;
            return _panel.Action(request);
        }

        public UIStylePreview SetStyles(params UIStyleChange[] changes)
        {
            return ChangeStyles("set", changes);
        }

        public UIStylePreview ResetStyles(params string[] properties)
        {
            if (properties == null)
                throw new ArgumentNullException("properties");
            UIStyleChange[] changes = new UIStyleChange[properties.Length];
            for (int i = 0; i < properties.Length; i++)
                changes[i] = new UIStyleChange(properties[i], null);
            return ChangeStyles("reset", changes);
        }

        public Task<UIWaitResult> WaitAsync(
            UIWaitCondition condition,
            string expected = null,
            int timeoutMs = 10000,
            int stableFrames = 3)
        {
            return _panel.WaitCoreAsync(Id, null, condition, expected, timeoutMs, stableFrames);
        }

        public UIHighlightResult Highlight()
        {
            return UIConvert.Highlight(UIToolkitDevTools.Highlight(
                new UIToolkitDevTools.HighlightRequest
                {
                    panelId = Panel.Id,
                    operation = "element",
                    elementId = Id
                }));
        }

        private UIActionResult Act(string operation, object value = null)
        {
            UIToolkitDevTools.ActionRequest request = Request(operation);
            request.value = value;
            return _panel.Action(request);
        }

        private UIToolkitDevTools.ActionRequest Request(string operation)
        {
            return new UIToolkitDevTools.ActionRequest
            {
                elementId = Id,
                operation = operation
            };
        }

        private UIStylePreview ChangeStyles(string operation, UIStyleChange[] changes)
        {
            if (changes == null || changes.Length == 0)
                throw new ArgumentException("At least one style change is required.", "changes");
            List<UIToolkitDevTools.StyleEdit> edits = new List<UIToolkitDevTools.StyleEdit>();
            for (int i = 0; i < changes.Length; i++)
            {
                UIStyleChange change = changes[i];
                if (change == null)
                    throw new ArgumentException("Style changes cannot contain null.", "changes");
                edits.Add(new UIToolkitDevTools.StyleEdit
                {
                    property = change.Property,
                    value = change.Value
                });
            }
            Dictionary<string, object> raw = UIToolkitDevTools.Style(
                new UIToolkitDevTools.StyleRequest
                {
                    operation = operation,
                    panelId = Panel.Id,
                    elementId = Id,
                    edits = edits
                });
            return new UIStylePreview(
                Panel,
                Id,
                UIConvert.String(raw, "previewId"),
                UIConvert.Int(raw, "documentRevision"));
        }
    }

    public sealed class UIStylePreview
    {
        private readonly UIPanel _panel;

        internal UIStylePreview(UIPanel panel, int elementId, string previewId, int revision)
        {
            _panel = panel;
            ElementId = elementId;
            PreviewId = previewId;
            Revision = revision;
        }

        public string PreviewId { get; private set; }
        public int ElementId { get; private set; }
        public int Revision { get; private set; }
        [UIJsonOmitDefault]
        public bool IsRolledBack { get; private set; }

        public void Rollback()
        {
            if (IsRolledBack)
                return;
            Dictionary<string, object> raw = UIToolkitDevTools.Style(
                new UIToolkitDevTools.StyleRequest
                {
                    operation = "rollback",
                    panelId = _panel.Id,
                    previewId = PreviewId
                });
            Revision = UIConvert.Int(raw, "documentRevision");
            IsRolledBack = true;
        }
    }

    internal static class UIConvert
    {
        internal static UIPanelInfo PanelInfo(IDictionary<string, object> raw)
        {
            return new UIPanelInfo
            {
                Id = String(raw, "panelId"),
                Kind = String(raw, "kind"),
                Title = String(raw, "title"),
                Owner = String(raw, "owner"),
                ElementCount = Int(raw, "elementCount"),
                Documents = Strings(raw, "documents")
            };
        }

        internal static UIInspection Inspection(IDictionary<string, object> raw)
        {
            List<UIElementInfo> elements = new List<UIElementInfo>();
            IList values = List(raw, "elements");
            for (int i = 0; i < values.Count; i++)
            {
                IDictionary<string, object> item = Map(values[i]);
                if (item != null)
                    elements.Add(ElementInfo(item));
            }
            return new UIInspection
            {
                PanelId = String(raw, "panelId"),
                Revision = Int(raw, "documentRevision"),
                ScopeElementId = Int(raw, "scopeElementId"),
                Truncated = Bool(raw, "truncated"),
                Elements = elements,
                Warnings = Strings(raw, "warnings")
            };
        }

        internal static UIActionResult Action(IDictionary<string, object> raw)
        {
            return new UIActionResult
            {
                ElementId = Int(raw, "elementId"),
                Revision = Int(raw, "documentRevision"),
                Value = Value(raw, "value"),
                Focused = NullableBool(raw, "focused"),
                ScrollOffset = Floats(raw, "scrollOffset"),
                TargetElementId = NullableInt(raw, "targetElementId")
            };
        }

        internal static UIWaitResult Wait(IDictionary<string, object> raw)
        {
            return new UIWaitResult
            {
                ElapsedMs = Int(raw, "elapsedMs"),
                ElementId = NullableInt(raw, "elementId"),
                Text = String(raw, "text"),
                Value = Value(raw, "value"),
                Visible = NullableBool(raw, "visible"),
                Enabled = NullableBool(raw, "enabled"),
                StableFrames = NullableInt(raw, "stableFrames"),
                Rect = Floats(raw, "rect")
            };
        }

        internal static UIHighlightResult Highlight(IDictionary<string, object> raw)
        {
            return new UIHighlightResult
            {
                Cleared = Bool(raw, "cleared"),
                Highlighted = Int(raw, "highlighted"),
                CaptureTarget = String(raw, "captureTarget"),
                RequestEditorStatus = String(raw, "requestEditorStatus"),
                WindowTitle = String(raw, "windowTitle")
            };
        }

        internal static string WaitCondition(UIWaitCondition condition)
        {
            string value = condition.ToString();
            return char.ToLowerInvariant(value[0]) + value.Substring(1);
        }

        internal static IDictionary<string, object> Map(object value)
        {
            return value as IDictionary<string, object>;
        }

        internal static IList List(IDictionary<string, object> raw, string key)
        {
            object value;
            if (raw != null && raw.TryGetValue(key, out value) && value is IList)
                return (IList)value;
            return new object[0];
        }

        internal static IList<string> Strings(IDictionary<string, object> raw, string key)
        {
            IList values = List(raw, key);
            List<string> output = new List<string>();
            for (int i = 0; i < values.Count; i++)
            {
                string value = Convert.ToString(values[i], CultureInfo.InvariantCulture);
                if (!string.IsNullOrEmpty(value))
                    output.Add(value);
            }
            return output;
        }

        internal static string String(IDictionary<string, object> raw, string key)
        {
            object value = Value(raw, key);
            return value == null ? null : Convert.ToString(value, CultureInfo.InvariantCulture);
        }

        internal static int Int(IDictionary<string, object> raw, string key)
        {
            object value = Value(raw, key);
            return value == null ? 0 : Convert.ToInt32(value, CultureInfo.InvariantCulture);
        }

        internal static int? NullableInt(IDictionary<string, object> raw, string key)
        {
            object value = Value(raw, key);
            return value == null ? (int?)null : Convert.ToInt32(value, CultureInfo.InvariantCulture);
        }

        internal static bool Bool(IDictionary<string, object> raw, string key)
        {
            object value = Value(raw, key);
            return value != null && Convert.ToBoolean(value, CultureInfo.InvariantCulture);
        }

        internal static bool? NullableBool(IDictionary<string, object> raw, string key)
        {
            object value = Value(raw, key);
            return value == null ? (bool?)null : Convert.ToBoolean(value, CultureInfo.InvariantCulture);
        }

        internal static object Value(IDictionary<string, object> raw, string key)
        {
            object value;
            return raw != null && raw.TryGetValue(key, out value) ? value : null;
        }

        internal static float[] Floats(IDictionary<string, object> raw, string key)
        {
            object value = Value(raw, key);
            if (value is float[])
                return (float[])value;
            IList list = value as IList;
            if (list == null)
                return null;
            float[] output = new float[list.Count];
            for (int i = 0; i < list.Count; i++)
                output[i] = Convert.ToSingle(list[i], CultureInfo.InvariantCulture);
            return output;
        }

        private static UIElementInfo ElementInfo(IDictionary<string, object> raw)
        {
            Dictionary<string, string> style = null;
            IDictionary<string, object> rawStyle = Map(Value(raw, "computedStyle"));
            if (rawStyle != null)
            {
                style = new Dictionary<string, string>(StringComparer.Ordinal);
                foreach (KeyValuePair<string, object> item in rawStyle)
                    style[item.Key] = Convert.ToString(item.Value, CultureInfo.InvariantCulture);
            }
            List<UIMatchedRule> rules = new List<UIMatchedRule>();
            IList rawRules = List(raw, "matchedRules");
            for (int i = 0; i < rawRules.Count; i++)
            {
                IDictionary<string, object> rule = Map(rawRules[i]);
                if (rule == null)
                    continue;
                rules.Add(new UIMatchedRule
                {
                    Path = String(rule, "path"),
                    Line = Int(rule, "line"),
                    Specificity = Int(rule, "specificity"),
                    Selector = String(rule, "selector")
                });
            }
            return new UIElementInfo
            {
                Id = Int(raw, "elementId"),
                ParentId = NullableInt(raw, "parentId"),
                Type = String(raw, "type"),
                Selector = String(raw, "selectorHint") ?? String(raw, "type"),
                ChildCount = Int(raw, "childCount"),
                Rect = Floats(raw, "rect"),
                Text = String(raw, "text"),
                Value = Value(raw, "value"),
                Actions = Strings(raw, "actions"),
                Visible = NullableBool(raw, "visible"),
                Enabled = NullableBool(raw, "enabled"),
                Focused = NullableBool(raw, "focused"),
                UxmlSource = String(raw, "uxmlSource"),
                ComputedStyle = style,
                MatchedRules = rules
            };
        }
    }

    internal static class UIToolkitJson
    {
        private const int MaxDepth = 12;
        private const int MaxNodes = 4096;
        private const int MaxMembersPerObject = 64;
        private const int MaxCollectionItems = 512;
        private const int MaxStringChars = 16384;
        private const int MaxOutputChars = 256 * 1024;
        private const int OutputClosingReserve = 128;
        private static readonly object TypePlanLock = new object();
        private static readonly Dictionary<Type, TypePlan> TypePlans = new Dictionary<Type, TypePlan>();

        private sealed class WriteState
        {
            internal readonly StringBuilder Output = new StringBuilder(256);
            internal readonly HashSet<object> Path =
                new HashSet<object>(ReferenceEqualityComparer.Instance);
            internal int Nodes;
            internal string StopReason;
        }

        private sealed class PropertyPlan
        {
            internal string Name;
            internal PropertyInfo Property;
            internal FieldInfo BackingField;
            internal bool OmitDefault;

            internal object Read(object value)
            {
                return BackingField != null
                    ? BackingField.GetValue(value)
                    : Property.GetValue(value, null);
            }
        }

        private sealed class TypePlan
        {
            internal PropertyPlan[] Properties;
            internal FieldInfo[] Fields;
        }

        internal static string Serialize(object value)
        {
            WriteState state = new WriteState();
            Write(state, value, 0);
            return state.Output.ToString();
        }

        private static void Write(WriteState state, object value, int depth)
        {
            if (state.StopReason != null)
                return;
            if (state.Nodes >= MaxNodes)
            {
                StopWithValue(state, "<max-nodes>");
                return;
            }
            if (state.Output.Length >= MaxOutputChars - OutputClosingReserve)
            {
                StopWithValue(state, "<max-output>");
                return;
            }
            state.Nodes++;

            if (value == null)
            {
                AppendRawValue(state, "null");
                return;
            }
            if (depth > MaxDepth)
            {
                Quote(state, "<max-depth>");
                return;
            }
            if (value is string || value is char)
            {
                Quote(state, Convert.ToString(value, CultureInfo.InvariantCulture));
                return;
            }
            if (value is bool)
            {
                AppendRawValue(state, (bool)value ? "true" : "false");
                return;
            }
            if (IsNumber(value))
            {
                if (value is float && (float.IsNaN((float)value) || float.IsInfinity((float)value)))
                    Quote(state, Convert.ToString(value, CultureInfo.InvariantCulture));
                else if (value is double && (double.IsNaN((double)value) || double.IsInfinity((double)value)))
                    Quote(state, Convert.ToString(value, CultureInfo.InvariantCulture));
                else
                    AppendRawValue(state, Convert.ToString(value, CultureInfo.InvariantCulture));
                return;
            }
            if (value is Enum)
            {
                Quote(state, value.ToString());
                return;
            }
            if (value is DateTime || value is DateTimeOffset || value is TimeSpan
                || value is Guid || value is Uri)
            {
                Quote(state, Convert.ToString(value, CultureInfo.InvariantCulture));
                return;
            }
            if (value is Type || value is MemberInfo || value is Delegate || value is Exception)
            {
                Quote(state, value.ToString());
                return;
            }
            if (value is UnityEngine.Object)
            {
                UnityEngine.Object unityObject = (UnityEngine.Object)value;
                Quote(state, unityObject == null ? null : unityObject.name);
                return;
            }

            Type type = value.GetType();
            bool track = !type.IsValueType;
            if (track && !state.Path.Add(value))
            {
                Quote(state, "<cycle>");
                return;
            }
            try
            {
                IDictionary dictionary = value as IDictionary;
                if (dictionary != null)
                {
                    state.Output.Append('{');
                    bool first = true;
                    int written = 0;
                    foreach (DictionaryEntry item in dictionary)
                    {
                        if (state.StopReason != null)
                            break;
                        if (written >= MaxCollectionItems)
                        {
                            WriteTruncationMember(state, ref first, "<max-items>");
                            break;
                        }
                        if (Omit(item.Value)) continue;
                        string key = Convert.ToString(item.Key, CultureInfo.InvariantCulture);
                        if (!CanStartMember(state, key))
                            break;
                        if (!first) state.Output.Append(',');
                        first = false;
                        Quote(state, key);
                        state.Output.Append(':');
                        Write(state, item.Value, depth + 1);
                        written++;
                    }
                    state.Output.Append('}');
                    return;
                }
                IEnumerable enumerable = value as IEnumerable;
                if (enumerable != null)
                {
                    state.Output.Append('[');
                    bool first = true;
                    int written = 0;
                    foreach (object item in enumerable)
                    {
                        if (state.StopReason != null)
                            break;
                        if (written >= MaxCollectionItems)
                        {
                            if (!first) state.Output.Append(',');
                            Quote(state, "<max-items>");
                            break;
                        }
                        if (!first) state.Output.Append(',');
                        first = false;
                        Write(state, item, depth + 1);
                        written++;
                    }
                    state.Output.Append(']');
                    return;
                }
                WriteObject(state, value, type, depth);
            }
            finally
            {
                if (track) state.Path.Remove(value);
            }
        }

        private static void WriteObject(WriteState state, object value, Type type, int depth)
        {
            state.Output.Append('{');
            bool first = true;
            int written = 0;
            TypePlan plan = GetTypePlan(type);
            for (int i = 0; i < plan.Properties.Length; i++)
            {
                if (state.StopReason != null)
                    break;
                if (written >= MaxMembersPerObject)
                {
                    WriteTruncationMember(state, ref first, "<max-members>");
                    break;
                }
                PropertyPlan property = plan.Properties[i];
                if (!CanStartMember(state, property.Name))
                    break;
                object item;
                try { item = property.Read(value); }
                catch { continue; }
                if (Omit(item)) continue;
                if (property.OmitDefault && IsDefaultValue(item))
                    continue;
                if (!first) state.Output.Append(',');
                first = false;
                Quote(state, property.Name);
                state.Output.Append(':');
                Write(state, item, depth + 1);
                written++;
            }
            for (int i = 0; i < plan.Fields.Length && state.StopReason == null; i++)
            {
                if (written >= MaxMembersPerObject)
                {
                    WriteTruncationMember(state, ref first, "<max-members>");
                    break;
                }
                if (!CanStartMember(state, plan.Fields[i].Name))
                    break;
                object item = plan.Fields[i].GetValue(value);
                if (Omit(item)) continue;
                if (!first) state.Output.Append(',');
                first = false;
                Quote(state, plan.Fields[i].Name);
                state.Output.Append(':');
                Write(state, item, depth + 1);
                written++;
            }
            state.Output.Append('}');
        }

        private static TypePlan GetTypePlan(Type type)
        {
            lock (TypePlanLock)
            {
                TypePlan existing;
                if (TypePlans.TryGetValue(type, out existing))
                    return existing;
            }

            bool trustedGetters = IsAnonymousType(type)
                || string.Equals(type.Namespace, "Locus.Skills", StringComparison.Ordinal)
                || (type.Namespace ?? "").StartsWith("Locus.Skills.", StringComparison.Ordinal);
            List<PropertyPlan> properties = new List<PropertyPlan>();
            if (!type.IsValueType)
            {
                PropertyInfo[] candidates = type.GetProperties(BindingFlags.Instance | BindingFlags.Public);
                for (int i = 0; i < candidates.Length; i++)
                {
                    PropertyInfo property = candidates[i];
                    if (!property.CanRead || property.GetIndexParameters().Length != 0)
                        continue;
                    FieldInfo backingField = type.GetField(
                        "<" + property.Name + ">k__BackingField",
                        BindingFlags.Instance | BindingFlags.NonPublic);
                    if (backingField == null && !trustedGetters)
                        continue;
                    properties.Add(new PropertyPlan
                    {
                        Name = JsonMemberName(property.Name),
                        Property = property,
                        BackingField = backingField,
                        OmitDefault = property.IsDefined(typeof(UIJsonOmitDefaultAttribute), true)
                    });
                }
            }

            TypePlan created = new TypePlan
            {
                Properties = properties.ToArray(),
                Fields = type.GetFields(BindingFlags.Instance | BindingFlags.Public)
            };
            lock (TypePlanLock)
            {
                TypePlan existing;
                if (TypePlans.TryGetValue(type, out existing))
                    return existing;
                TypePlans[type] = created;
                return created;
            }
        }

        private static bool IsAnonymousType(Type type)
        {
            string name = type == null ? "" : type.Name;
            return name.IndexOf("AnonymousType", StringComparison.Ordinal) >= 0
                && (name.StartsWith("<>", StringComparison.Ordinal)
                    || name.StartsWith("VB$", StringComparison.Ordinal));
        }

        private static string JsonMemberName(string name)
        {
            if (string.IsNullOrEmpty(name))
                return "value";
            return char.ToLowerInvariant(name[0]) + name.Substring(1);
        }

        private static void WriteTruncationMember(WriteState state, ref bool first, string reason)
        {
            if (!CanStartMember(state, "__truncated"))
                return;
            if (!first) state.Output.Append(',');
            first = false;
            Quote(state, "__truncated");
            state.Output.Append(':');
            Quote(state, reason);
        }

        private static bool CanStartMember(WriteState state, string name)
        {
            if (state.StopReason != null)
                return false;
            int sourceChars = Math.Min((name ?? "").Length, MaxStringChars);
            int worstCaseChars = sourceChars * 6 + 64;
            if (state.Output.Length + worstCaseChars
                < MaxOutputChars - OutputClosingReserve)
                return true;
            state.StopReason = "<max-output>";
            return false;
        }

        private static bool Omit(object value)
        {
            if (value == null) return true;
            string text = value as string;
            if (text != null) return text.Length == 0;
            ICollection collection = value as ICollection;
            return collection != null && collection.Count == 0;
        }

        private static bool IsDefaultValue(object value)
        {
            if (value == null) return true;
            Type type = value.GetType();
            if (!type.IsValueType) return false;
            return value.Equals(Activator.CreateInstance(type));
        }

        private static bool IsNumber(object value)
        {
            return value is byte || value is sbyte || value is short || value is ushort
                || value is int || value is uint || value is long || value is ulong
                || value is float || value is double || value is decimal;
        }

        private static void AppendRawValue(WriteState state, string value)
        {
            string text = value ?? "null";
            if (state.Output.Length + text.Length >= MaxOutputChars - OutputClosingReserve)
            {
                StopWithValue(state, "<max-output>");
                return;
            }
            state.Output.Append(text);
        }

        private static void StopWithValue(WriteState state, string reason)
        {
            if (state.StopReason != null)
                return;
            state.StopReason = reason;
            Quote(state, reason);
        }

        private static void Quote(WriteState state, string value)
        {
            if (value == null)
            {
                state.Output.Append("null");
                return;
            }
            int sourceLimit = Math.Min(value.Length, MaxStringChars);
            int outputLimit = MaxOutputChars - OutputClosingReserve;
            bool truncated = sourceLimit < value.Length;
            state.Output.Append('"');
            for (int i = 0; i < sourceLimit; i++)
            {
                char ch = value[i];
                int escapedLength = ch == '"' || ch == '\\' || ch == '\b' || ch == '\f'
                    || ch == '\n' || ch == '\r' || ch == '\t'
                    ? 2
                    : (ch < 32 ? 6 : 1);
                if (state.Output.Length + escapedLength + 20 >= outputLimit)
                {
                    truncated = true;
                    if (state.StopReason == null)
                        state.StopReason = "<max-output>";
                    break;
                }
                switch (ch)
                {
                    case '"': state.Output.Append("\\\""); break;
                    case '\\': state.Output.Append("\\\\"); break;
                    case '\b': state.Output.Append("\\b"); break;
                    case '\f': state.Output.Append("\\f"); break;
                    case '\n': state.Output.Append("\\n"); break;
                    case '\r': state.Output.Append("\\r"); break;
                    case '\t': state.Output.Append("\\t"); break;
                    default:
                        if (ch < 32)
                            state.Output.Append("\\u").Append(((int)ch).ToString("x4", CultureInfo.InvariantCulture));
                        else
                            state.Output.Append(ch);
                        break;
                }
            }
            if (truncated && state.Output.Length + 15 < outputLimit)
                state.Output.Append("...<truncated>");
            state.Output.Append('"');
        }

        private sealed class ReferenceEqualityComparer : IEqualityComparer<object>
        {
            internal static readonly ReferenceEqualityComparer Instance = new ReferenceEqualityComparer();
            public new bool Equals(object left, object right) { return ReferenceEquals(left, right); }
            public int GetHashCode(object value) { return System.Runtime.CompilerServices.RuntimeHelpers.GetHashCode(value); }
        }
    }
}
