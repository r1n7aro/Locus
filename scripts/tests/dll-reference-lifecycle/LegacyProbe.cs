using System;
using System.IO;
using System.Linq;
using System.Collections.Generic;
using System.Threading;
using System.Threading.Tasks;
using Microsoft.CodeAnalysis;
using Microsoft.CodeAnalysis.CSharp;

// Run with the unmodified Locus.Roslyn.dll on each installed Unity Mono host.
// This exercises the same factory used by MaterializeMetadataReferences.
static class LegacyProbe
{
    static MetadataReference[] bcl;
    static byte[] v1, v2;
    static int failures;

    static int Main(string[] args)
    {
        bcl = new[] { typeof(object).Assembly.Location, typeof(Enumerable).Assembly.Location }
            .Distinct().Select(p => (MetadataReference)MetadataReference.CreateFromFile(p)).ToArray();
        v1 = Build("OldType"); v2 = Build("NewType");
        var observations = new List<string>();
        foreach (bool atomic in new[] { false, true })
        foreach (bool warm in new[] { false, true })
        {
            string path = Path.Combine(args[0], "fixture-" + atomic + "-" + warm + ".dll");
            File.WriteAllBytes(path, v1);
            var reference = MetadataReference.CreateFromFile(path);
            var old = Enumerable.Range(0, 4).Select(_ => Consumer(reference, "OldType")).ToArray();
            if (warm) foreach (var compilation in old) if (!Emit(compilation)) failures++;
            using (var ready = new CountdownEvent(4))
            using (var resume = new ManualResetEventSlim())
            {
                var readers = old.Select(compilation => Task.Run(() => { ready.Signal(); resume.Wait(); return Emit(compilation); })).ToArray();
                bool replaced = false, newCompleted = false;
                if (!ready.Wait(TimeSpan.FromSeconds(20))) throw new TimeoutException("old readers did not start");
                try
                {
                    if (atomic) { string next = path + ".next"; File.WriteAllBytes(next, v2); File.Replace(next, path, null); }
                    else File.WriteAllBytes(path, v2);
                    replaced = true;
                    newCompleted = Emit(Consumer(MetadataReference.CreateFromFile(path), "NewType"));
                }
                catch (Exception ex) { Console.Error.WriteLine(ex); }
                finally { resume.Set(); }
                Task.WaitAll(readers);
                bool oldCompleted = readers.All(task => task.Result);
                GC.KeepAlive(reference);
                if (!replaced || !newCompleted || !oldCompleted) failures++;
                observations.Add("{\"atomic\":" + Json(atomic) + ",\"warm\":" + Json(warm) + ",\"readers\":4,\"replaced\":" + Json(replaced) + ",\"oldCompleted\":" + Json(oldCompleted) + ",\"newCompleted\":" + Json(newCompleted) + "}");
            }
        }
        Console.WriteLine("{\"roslyn\":\"" + typeof(MetadataReference).Assembly.GetName().Version + "\",\"fixtureBytes\":" + v1.Length + ",\"failures\":" + failures + ",\"cases\":[" + string.Join(",", observations) + "]}");
        return failures == 0 ? 0 : 1;
    }
    static string Json(bool value) { return value ? "true" : "false"; }
    static byte[] Build(string marker)
    {
        string source = "public class " + marker + " {}" + string.Concat(Enumerable.Range(0, 1500).Select(i => "public class Padding" + i + " { public int Field" + i + "; }"));
        var compilation = CSharpCompilation.Create("ReplaceableFixture", new[] { CSharpSyntaxTree.ParseText(source) }, bcl, new CSharpCompilationOptions(OutputKind.DynamicallyLinkedLibrary));
        using (var output = new MemoryStream())
        {
            var result = compilation.Emit(output);
            if (!result.Success) throw new Exception(string.Join("\n", result.Diagnostics));
            return output.ToArray();
        }
    }
    static CSharpCompilation Consumer(MetadataReference reference, string marker)
    {
        return CSharpCompilation.Create("Consumer" + Guid.NewGuid().ToString("N"), new[] { CSharpSyntaxTree.ParseText("public class Consumer { public " + marker + " Value; }") }, bcl.Concat(new[] { reference }), new CSharpCompilationOptions(OutputKind.DynamicallyLinkedLibrary));
    }
    static bool Emit(CSharpCompilation compilation)
    {
        try
        {
            using (var output = new MemoryStream())
            {
                var result = compilation.Emit(output);
                if (!result.Success) Console.Error.WriteLine(string.Join("\n", result.Diagnostics));
                return result.Success;
            }
        }
        catch (Exception ex) { Console.Error.WriteLine(ex); return false; }
    }
}
