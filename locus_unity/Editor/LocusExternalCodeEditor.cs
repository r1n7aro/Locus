using System;
using System.Diagnostics;
using System.IO;
using System.Text;

using Unity.CodeEditor;
using UnityEditor;
using UnityEngine;
using Debug = UnityEngine.Debug;

namespace Locus
{
    [InitializeOnLoad]
    internal sealed class LocusExternalCodeEditor : IExternalCodeEditor
    {
        private const string ExecutablePathKey = "Locus.ExternalEditor.ExecutablePath";
        private const string PreviousEditorPathKey = "Locus.ExternalEditor.PreviousPath";
        private const string ManagedDefaultKey = "Locus.ExternalEditor.ManagedDefault";
        private const string OpenScriptEvent = "locus-open-script";

        [Serializable]
        private sealed class ConfigureRequest
        {
            public string executablePath;
            public bool setDefault;
        }

        [Serializable]
        private sealed class OpenScriptPayload
        {
            public string projectPath;
            public string assetPath;
            public int line;
            public int column;
        }

        static LocusExternalCodeEditor()
        {
            try
            {
                CodeEditor.Register(new LocusExternalCodeEditor());
            }
            catch (Exception ex)
            {
                Debug.LogWarning("[Locus] External editor registration failed: " + ex.Message);
            }
        }

        public CodeEditor.Installation[] Installations
        {
            get
            {
                string path = EditorPrefs.GetString(ExecutablePathKey, "").Trim();
                if (!File.Exists(path))
                    return new CodeEditor.Installation[0];
                return new[]
                {
                    new CodeEditor.Installation { Name = "Locus", Path = path }
                };
            }
        }

        public bool TryGetInstallationForPath(
            string editorPath,
            out CodeEditor.Installation installation)
        {
            installation = default(CodeEditor.Installation);
            string locusPath = EditorPrefs.GetString(ExecutablePathKey, "").Trim();
            if (!PathsEqual(editorPath, locusPath) || !File.Exists(locusPath))
                return false;
            installation = new CodeEditor.Installation { Name = "Locus", Path = locusPath };
            return true;
        }

        public void Initialize(string editorInstallationPath)
        {
            if (File.Exists(editorInstallationPath))
                EditorPrefs.SetString(ExecutablePathKey, Path.GetFullPath(editorInstallationPath));
        }

        public bool OpenProject(string path, int line, int column)
        {
            if (!string.IsNullOrEmpty(path)
                && !path.EndsWith(".cs", StringComparison.OrdinalIgnoreCase))
                return false;

            string projectPath = ProjectRoot();
            string assetPath = NormalizeAssetPath(projectPath, path);
            if (!string.IsNullOrEmpty(assetPath)
                && !assetPath.StartsWith("Assets/", StringComparison.OrdinalIgnoreCase)
                && !assetPath.StartsWith("Packages/", StringComparison.OrdinalIgnoreCase)
                && !assetPath.StartsWith("ProjectSettings/", StringComparison.OrdinalIgnoreCase))
                return false;
            var payload = new OpenScriptPayload
            {
                projectPath = projectPath,
                assetPath = assetPath,
                line = Math.Max(1, line),
                column = Math.Max(1, column),
            };

            if (LocusBridge.HasConnectedDesktopClient())
            {
                LocusBridge.SendEventToRust(OpenScriptEvent, JsonUtility.ToJson(payload));
                return true;
            }

            string executablePath = EditorPrefs.GetString(ExecutablePathKey, "").Trim();
            if (!File.Exists(executablePath))
                return false;

            try
            {
                var startInfo = new ProcessStartInfo
                {
                    FileName = executablePath,
                    Arguments = BuildLaunchArguments(payload),
                    WorkingDirectory = projectPath,
                    UseShellExecute = false,
                };
                Process.Start(startInfo);
                return true;
            }
            catch (Exception ex)
            {
                Debug.LogError("[Locus] Failed to launch Locus: " + ex.Message);
                return false;
            }
        }

        public void SyncAll()
        {
            LocusProjectFiles.SyncAll();
        }

        public void SyncIfNeeded(
            string[] addedFiles,
            string[] deletedFiles,
            string[] movedFiles,
            string[] movedFromFiles,
            string[] importedFiles)
        {
            LocusProjectFiles.SyncIfNeeded(
                addedFiles, deletedFiles, movedFiles, movedFromFiles, importedFiles);
        }

        public void OnGUI()
        {
            EditorGUILayout.LabelField(
                "C# scripts open in the Locus asset preview.",
                EditorStyles.wordWrappedLabel);
            if (GUILayout.Button("Regenerate project files", GUILayout.Width(190)))
                LocusProjectFiles.SyncAll();
        }

        internal static string ConfigureFromJson(string json)
        {
            ConfigureRequest request = JsonUtility.FromJson<ConfigureRequest>(json ?? "{}");
            if (request == null || string.IsNullOrWhiteSpace(request.executablePath))
                throw new ArgumentException("Locus executable path is required");

            string executablePath = Path.GetFullPath(request.executablePath.Trim());
            if (!File.Exists(executablePath))
                throw new FileNotFoundException("Locus executable was not found", executablePath);

            EditorPrefs.SetString(ExecutablePathKey, executablePath);
            string currentPath = CodeEditor.CurrentEditorInstallation ?? "";
            bool managed = EditorPrefs.GetBool(ManagedDefaultKey, false);

            if (request.setDefault)
            {
                if (!managed)
                    EditorPrefs.SetString(PreviousEditorPathKey, currentPath);
                CodeEditor.SetExternalScriptEditor(executablePath);
                EditorPrefs.SetBool(ManagedDefaultKey, true);
                return "registered_and_selected";
            }

            if (managed && PathsEqual(currentPath, executablePath))
            {
                string previousPath = EditorPrefs.GetString(PreviousEditorPathKey, "");
                CodeEditor.SetExternalScriptEditor(previousPath);
            }
            EditorPrefs.SetBool(ManagedDefaultKey, false);
            return "registered";
        }

        private static string ProjectRoot()
        {
            return Path.GetFullPath(Path.Combine(Application.dataPath, ".."));
        }

        private static string NormalizeAssetPath(string projectPath, string path)
        {
            if (string.IsNullOrWhiteSpace(path))
                return "";
            string normalized = path.Replace('\\', '/');
            if (!Path.IsPathRooted(path))
                return normalized.TrimStart('/');

            string fullPath = Path.GetFullPath(path);
            string prefix = projectPath.TrimEnd(Path.DirectorySeparatorChar, Path.AltDirectorySeparatorChar)
                + Path.DirectorySeparatorChar;
            if (fullPath.StartsWith(prefix, StringComparison.OrdinalIgnoreCase))
                return fullPath.Substring(prefix.Length).Replace('\\', '/');
            return fullPath.Replace('\\', '/');
        }

        private static bool PathsEqual(string left, string right)
        {
            if (string.IsNullOrWhiteSpace(left) || string.IsNullOrWhiteSpace(right))
                return false;
            try
            {
                return string.Equals(
                    Path.GetFullPath(left).TrimEnd('\\', '/'),
                    Path.GetFullPath(right).TrimEnd('\\', '/'),
                    StringComparison.OrdinalIgnoreCase);
            }
            catch
            {
                return false;
            }
        }

        private static string BuildLaunchArguments(OpenScriptPayload payload)
        {
            var builder = new StringBuilder();
            AppendArgument(builder, "--locus-project", payload.projectPath);
            AppendArgument(builder, "--locus-open-script", payload.assetPath);
            AppendArgument(builder, "--locus-line", payload.line.ToString());
            AppendArgument(builder, "--locus-column", payload.column.ToString());
            return builder.ToString();
        }

        private static void AppendArgument(StringBuilder builder, string name, string value)
        {
            if (builder.Length > 0)
                builder.Append(' ');
            builder.Append(name).Append(' ').Append(QuoteArgument(value ?? ""));
        }

        private static string QuoteArgument(string value)
        {
            var quoted = new StringBuilder("\"");
            int backslashes = 0;
            foreach (char character in value)
            {
                if (character == '\\')
                {
                    backslashes++;
                    continue;
                }
                if (character == '"')
                {
                    quoted.Append('\\', backslashes * 2 + 1);
                    quoted.Append('"');
                    backslashes = 0;
                    continue;
                }
                quoted.Append('\\', backslashes);
                backslashes = 0;
                quoted.Append(character);
            }
            quoted.Append('\\', backslashes * 2);
            quoted.Append('"');
            return quoted.ToString();
        }
    }
}
