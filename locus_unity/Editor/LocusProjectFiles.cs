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
        internal const int GeneratorVersion = 1;
        private static bool _syncInProgress;

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
            if (_syncInProgress)
                return "sync_skipped: already in progress";

            _syncInProgress = true;
            var report = new StringBuilder();
            try
            {
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
                    LocusProjectFileGenerator.Generate();
                    report.AppendLine("Locus project generator: ok");
                    return "synced\n" + report;
                }
                catch (Exception ex)
                {
                    Exception cause = ex.InnerException ?? ex;
                    report.AppendLine("Locus project generator: " + cause.Message);
                }

                throw new InvalidOperationException(
                    "No Unity project-file generator succeeded.\n" + report);
            }
            finally
            {
                _syncInProgress = false;
            }
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

    /// Stable batch-mode entry point for validating or repairing a workspace
    /// without relying on the selected external editor.
    public static class LocusProjectFileGeneratorCommand
    {
        public static void Generate()
        {
            UnityEngine.Debug.Log("[LocusProjectSync] " + LocusProjectFiles.SyncAll());
        }
    }
}
