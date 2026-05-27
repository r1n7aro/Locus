using UnityEngine;
using UnityEditor;

using System;
using System.Collections.Generic;
using System.Reflection;
using System.Text;

namespace Locus
{
    public static partial class LocusBridge
    {
        private const int MaxConsoleEntriesToSend = 200;
        private const int MaxConsoleCharsToSend = 60000;

        [Serializable]
        private sealed class UnityConsoleTextPayload
        {
            public string text;
            public ConsoleTextEntry[] entries;
            public string title;
            public string source;
        }

        [Serializable]
        private sealed class ConsoleTextEntry
        {
            public string title;
            public string text;
            public string source;
            public string level;
        }

        private static string BuildConsoleTextPayloadJson()
        {
            ConsoleTextEntry[] entries = BuildConsoleTextEntries();
            return JsonUtility.ToJson(new UnityConsoleTextPayload
            {
                text = JoinConsoleTextEntries(entries),
                entries = entries,
                title = "Unity Console",
                source = "unity-console"
            });
        }

        private static ConsoleTextEntry[] BuildConsoleTextEntries()
        {
            ConsoleTextEntry[] entries = TryBuildConsoleTextEntriesFromLogEntries();
            if (entries == null || entries.Length == 0)
                return new ConsoleTextEntry[0];

            return entries;
        }

        private static string JoinConsoleTextEntries(ConsoleTextEntry[] entries)
        {
            if (entries == null || entries.Length == 0)
                return "";

            StringBuilder sb = new StringBuilder(Math.Min(MaxConsoleCharsToSend, entries.Length * 256));
            sb.AppendLine("Unity Console");
            bool hasEntry = false;
            for (int i = 0; i < entries.Length; i++)
            {
                ConsoleTextEntry entry = entries[i];
                if (entry == null || string.IsNullOrWhiteSpace(entry.text))
                    continue;
                if (hasEntry)
                    sb.AppendLine();
                sb.AppendLine(entry.text.TrimEnd());
                hasEntry = true;
                TrimStringBuilderStart(sb);
            }

            return TrimConsoleText(sb.ToString());
        }

        private static ConsoleTextEntry[] TryBuildConsoleTextEntriesFromLogEntries()
        {
            try
            {
                Type logEntriesType = FindEditorType("UnityEditor.LogEntries", "UnityEditorInternal.LogEntries");
                Type logEntryType = FindEditorType("UnityEditor.LogEntry", "UnityEditorInternal.LogEntry");
                if (logEntriesType == null)
                    return new ConsoleTextEntry[0];

                MethodInfo getCount = FindStaticMethod(logEntriesType, "GetCount", 0);
                if (getCount == null)
                    return new ConsoleTextEntry[0];

                int count = Convert.ToInt32(getCount.Invoke(null, null));
                if (count <= 0)
                    return new ConsoleTextEntry[0];

                MethodInfo startGettingEntries = FindStaticMethod(logEntriesType, "StartGettingEntries", 0);
                MethodInfo endGettingEntries = FindStaticMethod(logEntriesType, "EndGettingEntries", 0);
                MethodInfo getEntryInternal = FindStaticMethod(logEntriesType, "GetEntryInternal", 2);
                if (logEntryType != null && getEntryInternal != null)
                {
                    return BuildConsoleTextFromLogEntryObjects(
                        count,
                        logEntryType,
                        getEntryInternal,
                        startGettingEntries,
                        endGettingEntries);
                }

                MethodInfo getLinesAndMode = FindStaticMethod(logEntriesType, "GetLinesAndModeFromEntryInternal", 4);
                if (getLinesAndMode != null)
                    return BuildConsoleTextFromLinesAndMode(count, getLinesAndMode);
            }
            catch
            {
            }

            return new ConsoleTextEntry[0];
        }

        private static ConsoleTextEntry[] BuildConsoleTextFromLogEntryObjects(
            int count,
            Type logEntryType,
            MethodInfo getEntryInternal,
            MethodInfo startGettingEntries,
            MethodInfo endGettingEntries)
        {
            List<ConsoleTextEntry> entries = new List<ConsoleTextEntry>();
            int startIndex = Math.Max(0, count - MaxConsoleEntriesToSend);
            object logEntry = Activator.CreateInstance(logEntryType);

            try
            {
                if (startGettingEntries != null)
                    startGettingEntries.Invoke(null, null);

                for (int i = startIndex; i < count; i++)
                {
                    object result = getEntryInternal.Invoke(null, new[] { (object)i, logEntry });
                    if (result is bool && !(bool)result)
                        continue;

                    string condition = ReadStringMember(logEntry, "message", "condition");
                    string stackTrace = ReadStringMember(logEntry, "stacktrace", "stackTrace");
                    int mode = ReadIntMember(logEntry, "mode");
                    AddConsoleEntry(entries, LogModeLabel(mode), condition, stackTrace);
                    TrimConsoleEntries(entries);
                }
            }
            finally
            {
                if (endGettingEntries != null)
                    endGettingEntries.Invoke(null, null);
            }

            return entries.ToArray();
        }

        private static ConsoleTextEntry[] BuildConsoleTextFromLinesAndMode(int count, MethodInfo getLinesAndMode)
        {
            List<ConsoleTextEntry> entries = new List<ConsoleTextEntry>();
            int startIndex = Math.Max(0, count - MaxConsoleEntriesToSend);
            for (int i = startIndex; i < count; i++)
            {
                string lines;
                int mode;
                if (!TryGetLinesAndMode(getLinesAndMode, i, out lines, out mode))
                    continue;
                AddConsoleEntry(entries, LogModeLabel(mode), lines, "");
                TrimConsoleEntries(entries);
            }
            return entries.ToArray();
        }

        private static Type FindEditorType(params string[] names)
        {
            Assembly editorAssembly = typeof(EditorWindow).Assembly;
            foreach (string name in names)
            {
                Type type = editorAssembly.GetType(name);
                if (type != null)
                    return type;
            }
            return null;
        }

        private static bool TryGetLinesAndMode(MethodInfo method, int row, out string lines, out int mode)
        {
            lines = "";
            mode = 0;
            ParameterInfo[] parameters = method.GetParameters();
            if (parameters.Length != 4)
                return false;

            object[] args = new object[4];
            args[0] = row;
            args[1] = 1000;
            int modeIndex = -1;
            int textIndex = -1;

            for (int i = 2; i < parameters.Length; i++)
            {
                Type parameterType = parameters[i].ParameterType;
                Type valueType = parameterType.IsByRef ? parameterType.GetElementType() : parameterType;
                if (valueType == typeof(int) || (valueType != null && valueType.IsEnum))
                {
                    args[i] = valueType != null && valueType.IsEnum ? Enum.ToObject(valueType, 0) : (object)0;
                    modeIndex = i;
                }
                else if (valueType == typeof(string))
                {
                    args[i] = "";
                    textIndex = i;
                }
                else
                {
                    args[i] = null;
                }
            }

            method.Invoke(null, args);
            if (modeIndex >= 0)
                mode = Convert.ToInt32(args[modeIndex]);
            if (textIndex >= 0)
                lines = Convert.ToString(args[textIndex]) ?? "";
            return !string.IsNullOrWhiteSpace(lines);
        }

        private static MethodInfo FindStaticMethod(Type type, string name, int parameterCount)
        {
            MethodInfo[] methods = type.GetMethods(BindingFlags.Static | BindingFlags.Public | BindingFlags.NonPublic);
            foreach (MethodInfo method in methods)
            {
                if (!string.Equals(method.Name, name, StringComparison.Ordinal))
                    continue;
                if (method.GetParameters().Length == parameterCount)
                    return method;
            }
            return null;
        }

        private static string ReadStringMember(object target, params string[] names)
        {
            if (target == null || names == null)
                return "";

            Type type = target.GetType();
            foreach (string name in names)
            {
                FieldInfo field = type.GetField(name, BindingFlags.Instance | BindingFlags.Public | BindingFlags.NonPublic);
                if (field != null)
                    return Convert.ToString(field.GetValue(target)) ?? "";

                PropertyInfo property = type.GetProperty(name, BindingFlags.Instance | BindingFlags.Public | BindingFlags.NonPublic);
                if (property != null)
                    return Convert.ToString(property.GetValue(target, null)) ?? "";
            }
            return "";
        }

        private static int ReadIntMember(object target, params string[] names)
        {
            if (target == null || names == null)
                return 0;

            Type type = target.GetType();
            foreach (string name in names)
            {
                FieldInfo field = type.GetField(name, BindingFlags.Instance | BindingFlags.Public | BindingFlags.NonPublic);
                if (field != null)
                    return ConvertMemberToInt(field.GetValue(target));

                PropertyInfo property = type.GetProperty(name, BindingFlags.Instance | BindingFlags.Public | BindingFlags.NonPublic);
                if (property != null)
                    return ConvertMemberToInt(property.GetValue(target, null));
            }
            return 0;
        }

        private static int ConvertMemberToInt(object value)
        {
            if (value == null)
                return 0;
            try
            {
                return Convert.ToInt32(value);
            }
            catch
            {
                int parsed;
                return int.TryParse(Convert.ToString(value), out parsed) ? parsed : 0;
            }
        }

        private static void AddConsoleEntry(List<ConsoleTextEntry> entries, string type, string condition, string stackTrace)
        {
            condition = (condition ?? "").TrimEnd();
            stackTrace = (stackTrace ?? "").TrimEnd();
            if (string.IsNullOrEmpty(condition) && string.IsNullOrEmpty(stackTrace))
                return;

            string level = string.IsNullOrEmpty(type) ? "Log" : type;
            StringBuilder sb = new StringBuilder(condition.Length + stackTrace.Length + level.Length + 8);
            sb.Append("[").Append(level).Append("] ");
            sb.AppendLine(condition);
            if (!string.IsNullOrEmpty(stackTrace))
                sb.AppendLine(stackTrace);

            entries.Add(new ConsoleTextEntry
            {
                title = ConsoleEntryTitle(level, condition),
                text = sb.ToString().TrimEnd(),
                source = "unity-console",
                level = level
            });
        }

        private static string ConsoleEntryTitle(string level, string condition)
        {
            string summary = FirstNonEmptyLine(condition);
            if (string.IsNullOrEmpty(summary))
                summary = "Unity Console";
            if (summary.Length > 96)
                summary = summary.Substring(0, 93) + "...";
            return "[" + (string.IsNullOrEmpty(level) ? "Log" : level) + "] " + summary;
        }

        private static string FirstNonEmptyLine(string text)
        {
            if (string.IsNullOrEmpty(text))
                return "";

            string[] lines = text.Split(new[] { "\r\n", "\n" }, StringSplitOptions.None);
            foreach (string line in lines)
            {
                string trimmed = line.Trim();
                if (!string.IsNullOrEmpty(trimmed))
                    return trimmed;
            }
            return "";
        }

        private static void TrimConsoleEntries(List<ConsoleTextEntry> entries)
        {
            int total = 0;
            for (int i = 0; i < entries.Count; i++)
            {
                ConsoleTextEntry entry = entries[i];
                total += entry == null || entry.text == null ? 0 : entry.text.Length;
            }

            while (total > MaxConsoleCharsToSend && entries.Count > 1)
            {
                ConsoleTextEntry removed = entries[0];
                total -= removed == null || removed.text == null ? 0 : removed.text.Length;
                entries.RemoveAt(0);
            }

            if (total <= MaxConsoleCharsToSend || entries.Count == 0)
                return;

            ConsoleTextEntry first = entries[0];
            if (first == null || string.IsNullOrEmpty(first.text))
                return;

            int overflow = total - MaxConsoleCharsToSend;
            if (overflow <= 0 || overflow >= first.text.Length)
                return;

            first.text = first.text.Substring(overflow).TrimStart();
        }

        private static string LogModeLabel(int mode)
        {
            const int Error = 1 << 0;
            const int Assert = 1 << 1;
            const int Log = 1 << 2;
            const int Fatal = 1 << 4;
            const int AssetImportError = 1 << 6;
            const int AssetImportWarning = 1 << 7;
            const int ScriptingError = 1 << 8;
            const int ScriptingWarning = 1 << 9;
            const int ScriptingLog = 1 << 10;
            const int ScriptCompileError = 1 << 11;
            const int ScriptCompileWarning = 1 << 12;
            const int ScriptingException = 1 << 17;
            const int GraphCompileError = 1 << 20;
            const int ScriptingAssertion = 1 << 21;
            const int VisualScriptingError = 1 << 22;

            const int ErrorMask =
                Error
                | Assert
                | Fatal
                | AssetImportError
                | ScriptingError
                | ScriptCompileError
                | ScriptingException
                | GraphCompileError
                | ScriptingAssertion
                | VisualScriptingError;
            const int WarningMask = AssetImportWarning | ScriptingWarning | ScriptCompileWarning;
            const int LogMask = Log | ScriptingLog;

            if ((mode & ErrorMask) != 0)
                return "Error";
            if ((mode & WarningMask) != 0)
                return "Warning";
            if ((mode & LogMask) != 0)
                return "Log";
            return "Log";
        }

        private static string TrimConsoleText(string text)
        {
            if (string.IsNullOrEmpty(text) || text.Length <= MaxConsoleCharsToSend)
                return text ?? "";

            return "[older console output truncated]\n" + text.Substring(text.Length - MaxConsoleCharsToSend);
        }

        private static void TrimStringBuilderStart(StringBuilder sb)
        {
            if (sb.Length <= MaxConsoleCharsToSend)
                return;

            sb.Remove(0, sb.Length - MaxConsoleCharsToSend);
        }
    }
}
