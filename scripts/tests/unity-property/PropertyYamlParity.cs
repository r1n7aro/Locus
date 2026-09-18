using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Globalization;
using System.IO;
using System.Linq;
using System.Security.Cryptography;
using System.Text;
using UnityEditor;
using UnityEngine;

namespace PropertyReview
{
    public sealed class PropertyYamlImportCounter : AssetPostprocessor
    {
        public static readonly Dictionary<string, int> Counts = new Dictionary<string, int>();
        static void OnPostprocessAllAssets(string[] imported, string[] deleted, string[] moved, string[] from)
        {
            foreach (string path in imported)
                if (path.Contains("/Parity")) Counts[path] = Counts.TryGetValue(path, out int count) ? count + 1 : 1;
        }
    }

    public static partial class PropertyReviewRunner
    {
        [Serializable] private sealed class YamlCandidate { public string text; }
        [Serializable] private sealed class DiskProfile { public double preflightMs; public double writeMs; public double importMs; public int changedFiles; }
        [Serializable] private sealed class DiskProfileResult { public DiskProfile timings; }
        private static DiskProfile lastDiskProfile;
        private static string Json(object value) { return (string)Call("AssetApiJson", value); }
        private static string Id(ReviewData data)
        {
            AssetDatabase.TryGetGUIDAndLocalFileIdentifier(data, out string guid, out long id);
            return id.ToString(CultureInfo.InvariantCulture);
        }
        private static Dictionary<string, object> Op(string id, string path, object value)
        {
            return new Dictionary<string, object> { { "op", "set" }, { "object_id", id }, { "property_path", "/MonoBehaviour/" + path }, { "value", value } };
        }
        private static Dictionary<string, object> ArrayOp(string id, string action, params object[] pairs)
        {
            var result = new Dictionary<string, object> { { "op", action }, { "object_id", id }, { "property_path", "/MonoBehaviour/numbers" } };
            for (int i = 0; i < pairs.Length; i += 2) result[(string)pairs[i]] = pairs[i + 1];
            return result;
        }
        private static Dictionary<string, object> PrepareYaml(ReviewData data, object[] operations, bool logicalProperties = false)
        {
            string[] args = Environment.GetCommandLineArgs();
            int index = Array.IndexOf(args, "-locusPropertyYamlDriver");
            if (index < 0 || index + 1 >= args.Length) throw new Exception("YAML parity driver argument is required");
            string path = AssetDatabase.GetAssetPath(data);
            var start = new ProcessStartInfo(args[index + 1]) { UseShellExecute = false, CreateNoWindow = true, RedirectStandardInput = true, RedirectStandardOutput = true, RedirectStandardError = true, StandardOutputEncoding = Encoding.UTF8, StandardErrorEncoding = Encoding.UTF8 };
            byte[] before = File.ReadAllBytes(path);
            string text;
            using (var process = Process.Start(start))
            {
                var output = process.StandardOutput.ReadToEndAsync();
                var errors = process.StandardError.ReadToEndAsync();
                byte[] input = Encoding.UTF8.GetBytes(Json(new Dictionary<string, object> { { "source", Path.GetFullPath(path) }, { "object_id", Id(data) }, { logicalProperties ? "properties" : "operations", operations } }));
                process.StandardInput.BaseStream.Write(input, 0, input.Length);
                process.StandardInput.Close();
                if (!process.WaitForExit(60000)) { process.Kill(); throw new Exception("Owned YAML parity driver timed out"); }
                if (process.ExitCode != 0) throw new Exception(errors.Result);
                text = JsonUtility.FromJson<YamlCandidate>(output.Result).text;
            }
            string sha;
            using (var hash = SHA256.Create()) sha = BitConverter.ToString(hash.ComputeHash(before)).Replace("-", "").ToLowerInvariant();
            return new Dictionary<string, object> { { "path", path }, { "expected_sha256", sha }, { "bytes_base64", Convert.ToBase64String(Encoding.UTF8.GetBytes(text)) } };
        }
        private static void ApplyYaml(params Dictionary<string, object>[] entries)
        {
            string result = (string)Call("ExecuteAssetApiRequest", Json(new Dictionary<string, object> { { "action", "disk_apply" }, { "transaction_id", Guid.NewGuid().ToString() }, { "entries", entries } }));
            lastDiskProfile = JsonUtility.FromJson<DiskProfileResult>(result).timings;
        }
        private static ReviewData ParityAsset(string name)
        {
            var data = ScriptableObject.CreateInstance<ReviewData>();
            data.numbers = new List<int> { 10, 20, 30, 40, 50 };
            data.node = data.alias = new ReviewLeaf { amount = 23, hidden = 61 };
            data.node.next = data.node;
            AssetDatabase.CreateAsset(data, Folder + "/Parity" + name + ".asset");
            AssetDatabase.SaveAssets();
            return data;
        }
        private static void LiveWrite(ReviewData data, string property, string json)
        {
            Call("WritePropertyTree", "yaml-parity", TargetFor(data, property), json, "commit", false);
        }
        private static bool UntouchedGraph(ReviewData data)
        {
            return ReferenceEquals(data.node, data.alias) && ReferenceEquals(data.node, data.node.next) && data.node.hidden == 61;
        }
        private static void YamlParityCases()
        {
            AuthoringParityCases();
            Check("YAML01-scalar-property-parity", "Rust YAML and main-thread PropertyTree writes agree after Unity reimport", () => {
                var yaml = ParityAsset("ScalarYaml"); var live = ParityAsset("ScalarLive");
                string id = Id(yaml);
                object[] operations = {
                    Op(id, "regularInt", 731), Op(id, "wide", new { kind = "int64", value = "9223372036854775806" }),
                    Op(id, "unsignedWide", new { kind = "uint64", value = "18446744073709551614" }),
                    Op(id, "precise", 9.1234567890123), Op(id, "note", "中文\nquoted: \"value\""), Op(id, "toggle", false),
                    Op(id, "intVector/x", 321), Op(id, "reference", new { fileID = "0" })
                };
                var prepared = PrepareYaml(yaml, operations);
                LiveWrite(live, "regularInt", "731"); LiveWrite(live, "wide", "\"9223372036854775806\"");
                LiveWrite(live, "unsignedWide", "\"18446744073709551614\""); LiveWrite(live, "precise", "9.1234567890123");
                LiveWrite(live, "note", Json("中文\nquoted: \"value\"")); LiveWrite(live, "toggle", "false");
                LiveWrite(live, "intVector.x", "321"); LiveWrite(live, "reference", "null");
                AssetDatabase.SaveAssets();
                ApplyYaml(prepared);
                observed = "integer=" + yaml.regularInt + "; int64=" + yaml.wide + "; uint64=" + yaml.unsignedWide + "; text=" + yaml.note;
                return yaml.regularInt == live.regularInt && yaml.wide == live.wide && yaml.unsignedWide == live.unsignedWide
                    && yaml.precise == live.precise && yaml.note == live.note && yaml.toggle == live.toggle && yaml.intVector == live.intVector
                    && yaml.reference == live.reference && UntouchedGraph(yaml) && UntouchedGraph(live);
            });
            Check("YAML02-array-operation-parity", "Ordered array operations agree and preserve untouched shared managed references", () => {
                var yaml = ParityAsset("ArrayYaml"); var live = ParityAsset("ArrayLive"); string id = Id(yaml);
                var prepared = PrepareYaml(yaml, new object[] {
                    ArrayOp(id, "array_move", "index", 1, "to_index", 3), ArrayOp(id, "array_remove", "index", 0),
                    ArrayOp(id, "array_resize", "size", 2), ArrayOp(id, "array_insert", "index", 1, "value", 99)
                });
                LiveWrite(live, "numbers", "{\"action\":\"move\",\"index\":1,\"toIndex\":3}");
                LiveWrite(live, "numbers", "{\"action\":\"delete\",\"index\":0}");
                LiveWrite(live, "numbers", "{\"action\":\"resize\",\"size\":2}");
                LiveWrite(live, "numbers", "{\"action\":\"insert\",\"index\":1}"); LiveWrite(live, "numbers.Array.data[1]", "99");
                AssetDatabase.SaveAssets();
                ApplyYaml(prepared);
                observed = "yaml=" + string.Join(",", yaml.numbers) + "; live=" + string.Join(",", live.numbers);
                return yaml.numbers.SequenceEqual(live.numbers) && yaml.numbers.SequenceEqual(new[] { 30, 99, 40 }) && UntouchedGraph(yaml);
            });
            Check("YAML03-batch-import", "Accumulated edits replace each file once and import each changed asset once", () => {
                var a = ParityAsset("BatchA"); var b = ParityAsset("BatchB");
                string pathA = AssetDatabase.GetAssetPath(a), pathB = AssetDatabase.GetAssetPath(b);
                var first = PrepareYaml(a, new object[] { Op(Id(a), "regularInt", 81), Op(Id(a), "regularInt", 82), Op(Id(a), "note", "batched") });
                var second = PrepareYaml(b, new object[] { Op(Id(b), "regularInt", 83) });
                bool unchanged = a.regularInt == 17 && b.regularInt == 17;
                PropertyYamlImportCounter.Counts.Clear(); ApplyYaml(first, second);
                int importsA = PropertyYamlImportCounter.Counts.TryGetValue(pathA, out int n) ? n : 0;
                int importsB = PropertyYamlImportCounter.Counts.TryGetValue(pathB, out n) ? n : 0;
                observed = "imports=" + importsA + "," + importsB + "; values=" + a.regularInt + "," + b.regularInt;
                return unchanged && a.regularInt == 82 && b.regularInt == 83 && a.note == "batched" && importsA == 1 && importsB == 1;
            });
            Check("YAML04-stale-disk-guard", "A Unity edit between prepare and flush rejects the YAML batch and preserves newer bytes", () => {
                var data = ParityAsset("Stale"); var prepared = PrepareYaml(data, new object[] { Op(Id(data), "regularInt", 81) });
                LiveWrite(data, "regularInt", "99");
                bool dirtyRejected = false;
                try { ApplyYaml(prepared); } catch (Exception error) { dirtyRejected = error.ToString().Contains("dirty_asset"); }
                AssetDatabase.SaveAssets();
                byte[] newer = File.ReadAllBytes(AssetDatabase.GetAssetPath(data));
                bool rejected = false;
                try { ApplyYaml(prepared); } catch (Exception error) { rejected = error.ToString().Contains("revision_conflict"); }
                observed = "dirtyRejected=" + dirtyRejected + "; staleRejected=" + rejected + "; value=" + data.regularInt;
                return dirtyRejected && rejected && data.regularInt == 99 && newer.SequenceEqual(File.ReadAllBytes(AssetDatabase.GetAssetPath(data)));
            });
            Check("YAML05-logical-managed-cycle", "Shared Rust logical paths and Unity PropertyTree agree when editing through a cycle and alias", () => {
                var yaml = ParityAsset("LogicalYaml"); var live = ParityAsset("LogicalLive");
                long yamlId = Snap(yaml, "node").managedReferenceId;
                var prepared = PrepareYaml(yaml, new object[] {
                    new { propertyPath = "node.next.amount", value = 117 },
                    new { propertyPath = "alias.hidden", value = 73 }
                }, true);
                LiveWrite(live, "node.next.amount", "117"); LiveWrite(live, "alias.hidden", "73");
                AssetDatabase.SaveAssets(); ApplyYaml(prepared);
                AssetDatabase.ImportAsset(AssetDatabase.GetAssetPath(yaml), ImportAssetOptions.ForceSynchronousImport | ImportAssetOptions.ForceUpdate);
                observed = "amount=" + yaml.node.amount + "; hidden=" + yaml.alias.hidden + "; sameId=" + (yamlId == Snap(yaml, "node").managedReferenceId);
                return yaml.node.amount == live.node.amount && yaml.alias.hidden == live.alias.hidden
                    && ReferenceEquals(yaml.node, yaml.alias) && ReferenceEquals(yaml.node, yaml.node.next)
                    && yamlId == Snap(yaml, "node").managedReferenceId;
            });
            Check("YAML06-logical-array-batch", "Logical array commands and indexed writes use the same ordered semantics after reimport", () => {
                var yaml = ParityAsset("LogicalArrayYaml"); var live = ParityAsset("LogicalArrayLive");
                var prepared = PrepareYaml(yaml, new object[] {
                    new { propertyPath = "numbers", value = (object)new { action = "insert", index = 5, value = 0 } },
                    new { propertyPath = "numbers.Array.data[5]", value = (object)97 },
                    new { propertyPath = "numbers", value = (object)new { action = "move", index = 5, toIndex = 0 } }
                }, true);
                LiveWrite(live, "numbers", "{\"action\":\"insert\",\"index\":5}");
                LiveWrite(live, "numbers.Array.data[5]", "97");
                LiveWrite(live, "numbers", "{\"action\":\"move\",\"index\":5,\"toIndex\":0}");
                AssetDatabase.SaveAssets(); ApplyYaml(prepared);
                observed = "values=" + string.Join(",", yaml.numbers);
                return yaml.numbers.SequenceEqual(live.numbers) && yaml.numbers[0] == 97 && UntouchedGraph(yaml);
            });
            Check("YAML07-logical-type-rejection", "Unsupported managed type creation is rejected before disk or Editor changes", () => {
                var yaml = ParityAsset("LogicalReject"); byte[] before = File.ReadAllBytes(AssetDatabase.GetAssetPath(yaml));
                bool rejected = false;
                try { PrepareYaml(yaml, new object[] { new { propertyPath = "node", value = new { action = "setType", typeName = "PropertyReview.ReviewOther" } } }, true); }
                catch (Exception error) { rejected = error.ToString().Contains("unsupported_command"); }
                observed = "rejected=" + rejected;
                return rejected && before.SequenceEqual(File.ReadAllBytes(AssetDatabase.GetAssetPath(yaml))) && UntouchedGraph(yaml);
            });
        }
    }
}
