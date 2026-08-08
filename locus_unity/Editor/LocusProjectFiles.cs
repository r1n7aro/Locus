using System;
using System.IO;
using System.Reflection;
using System.Text;

using Unity.CodeEditor;
using UnityEditor;

namespace Locus
{
    internal static class LocusProjectFiles
    {
        private static readonly string[] GeneratorTypeNames =
        {
            "Microsoft.Unity.VisualStudio.Editor.ProjectGeneration, Unity.VisualStudio.Editor",
            "Packages.Rider.Editor.ProjectGeneration.ProjectGeneration, Unity.Rider.Editor",
            "VSCodeEditor.ProjectGeneration, Unity.VSCode.Editor",
        };

        internal static readonly string[] ProjectInputExtensions =
        {
            ".cs", ".asmdef", ".asmref", ".rsp", ".additionalfile"
        };

        internal static string SyncAll()
        {
            var report = new StringBuilder();
            foreach (string typeName in GeneratorTypeNames)
            {
                try
                {
                    Type type = Type.GetType(typeName, false);
                    if (type == null)
                        continue;

                    object generator = Activator.CreateInstance(type, true);
                    MethodInfo sync = type.GetMethod(
                        "Sync",
                        BindingFlags.Public | BindingFlags.NonPublic | BindingFlags.Instance,
                        null,
                        Type.EmptyTypes,
                        null);
                    if (sync == null)
                    {
                        report.AppendLine(typeName + ": Sync() not found");
                        continue;
                    }

                    sync.Invoke(generator, null);
                    report.AppendLine(typeName + ": ok");
                    return "synced\n" + report;
                }
                catch (Exception ex)
                {
                    Exception cause = ex.InnerException ?? ex;
                    report.AppendLine(typeName + ": " + cause.Message);
                }
            }

            try
            {
                Type syncVsType = Type.GetType("UnityEditor.SyncVS,UnityEditor", false);
                MethodInfo syncSolution = syncVsType != null
                    ? syncVsType.GetMethod(
                        "SyncSolution",
                        BindingFlags.Public | BindingFlags.NonPublic | BindingFlags.Static)
                    : null;
                if (syncSolution != null)
                {
                    syncSolution.Invoke(null, null);
                    report.AppendLine("UnityEditor.SyncVS: ok");
                    return "synced\n" + report;
                }
                report.AppendLine("UnityEditor.SyncVS: unavailable");
            }
            catch (Exception ex)
            {
                Exception cause = ex.InnerException ?? ex;
                report.AppendLine("UnityEditor.SyncVS: " + cause.Message);
            }

            throw new InvalidOperationException(
                "No Unity project-file generator succeeded.\n" + report);
        }

        internal static void SyncIfNeeded(params string[][] pathGroups)
        {
            if (!ContainsProjectInput(pathGroups))
                return;
            SyncAll();
        }

        private static bool ContainsProjectInput(string[][] pathGroups)
        {
            if (pathGroups == null)
                return false;
            foreach (string[] paths in pathGroups)
            {
                if (paths == null)
                    continue;
                foreach (string path in paths)
                {
                    if (string.IsNullOrEmpty(path))
                        continue;
                    foreach (string extension in ProjectInputExtensions)
                    {
                        if (path.EndsWith(extension, StringComparison.OrdinalIgnoreCase))
                            return true;
                    }
                    if (string.Equals(
                        Path.GetFileName(path),
                        "manifest.json",
                        StringComparison.OrdinalIgnoreCase))
                        return true;
                }
            }
            return false;
        }
    }

    /// Keep project files current when Unity is using its basic external-editor
    /// adapter. IDE packages and the Locus adapter already receive
    /// IExternalCodeEditor.SyncIfNeeded callbacks, so this fallback only owns
    /// the configuration where no project-generating editor is selected.
    internal sealed class LocusProjectFilesAssetPostprocessor : AssetPostprocessor
    {
        private static bool _syncScheduled;

        private static void OnPostprocessAllAssets(
            string[] importedAssets,
            string[] deletedAssets,
            string[] movedAssets,
            string[] movedFromAssetPaths)
        {
            IExternalCodeEditor current = CodeEditor.CurrentEditor;
            if (current != null
                && !string.Equals(
                    current.GetType().Name,
                    "DefaultExternalCodeEditor",
                    StringComparison.Ordinal))
                return;
            if (!ContainsProjectInputs(
                importedAssets,
                deletedAssets,
                movedAssets,
                movedFromAssetPaths))
                return;
            if (_syncScheduled)
                return;

            _syncScheduled = true;
            EditorApplication.delayCall += delegate
            {
                _syncScheduled = false;
                try
                {
                    LocusProjectFiles.SyncAll();
                }
                catch (Exception ex)
                {
                    UnityEngine.Debug.LogWarning(
                        "[Locus] Automatic project-file sync failed: " + ex.Message);
                }
            };
        }

        private static bool ContainsProjectInputs(params string[][] groups)
        {
            foreach (string[] paths in groups)
            {
                if (paths == null)
                    continue;
                foreach (string path in paths)
                {
                    if (string.IsNullOrEmpty(path))
                        continue;
                    foreach (string extension in LocusProjectFiles.ProjectInputExtensions)
                    {
                        if (path.EndsWith(extension, StringComparison.OrdinalIgnoreCase))
                            return true;
                    }
                    if (string.Equals(
                        Path.GetFileName(path),
                        "manifest.json",
                        StringComparison.OrdinalIgnoreCase))
                        return true;
                }
            }
            return false;
        }
    }
}
