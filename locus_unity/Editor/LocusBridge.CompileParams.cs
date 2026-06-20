using UnityEngine;

using System;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Text;
using System.Threading.Tasks;

namespace Locus
{
    public static partial class LocusBridge
    {
        // ───────────────── get_compile_params ─────────────────
        //
        // Provider side of the CoreCLR compile-server sidecar: Locus asks for
        // the reference paths / preprocessor defines / language version that
        // the in-Unity snippet compiler would use, plus a fingerprint so the
        // (cheap) roundtrip can answer "unchanged" without resending ~300
        // paths. The sidecar compiles with exactly these parameters, which
        // keeps the two compile paths equivalent.

        /// <summary>
        /// Identity of the current AppDomain load. Regenerated on every
        /// domain reload (same pattern as _viewScriptDomainFingerprint); the
        /// sidecar keys its in-memory snippet image registry on it so images
        /// from an unloaded domain are never referenced again.
        /// </summary>
        private static readonly string _compileDomainGeneration = Guid.NewGuid().ToString("N");

        /// <summary>Language version the snippet compiler pins (LanguageVersion.CSharp9).</summary>
        private const string CompileParamsLanguageVersion = "9";

        [Serializable]
        private sealed class GetCompileParamsRequest
        {
            public string known_fingerprint;
        }

        /// <summary>
        /// Reply shape for the get_reload_state probe (see the message handler
        /// in LocusBridge.cs). domain_generation changes on every domain
        /// reload; converged_serial advances only on a successful compilation,
        /// so the desktop can tell a compile-driven convergence from a
        /// no-compile reload.
        /// </summary>
        [Serializable]
        private sealed class ReloadStatePayload
        {
            public string session_id;
            public string domain_generation;
            public int converged_serial;
        }

        [Serializable]
        private sealed class CompileParamsPayload
        {
            public bool unchanged;
            public string fingerprint;
            public string domain_generation;
            public string lang_version;
            public string[] defines;
            public string[] reference_paths;
            public bool allow_unsafe;
        }

        private static async Task<PipeEnvelope> HandleGetCompileParams(string requestId, string requestJson)
        {
            string knownFingerprint = "";
            if (!string.IsNullOrEmpty(requestJson))
            {
                try
                {
                    GetCompileParamsRequest request = JsonUtility.FromJson<GetCompileParamsRequest>(requestJson);
                    if (request != null && !string.IsNullOrEmpty(request.known_fingerprint))
                        knownFingerprint = request.known_fingerprint;
                }
                catch
                {
                }
            }

            // The path collection needs Unity APIs (main thread) but is
            // cached per domain; the per-request fingerprint hashing is pure
            // file IO and runs here on the pipe worker so a busy editor main
            // thread (e.g. right after a domain reload) does not stall the
            // roundtrip.
            List<string> paths = TryGetCachedCompileReferencePaths();
            if (paths == null)
            {
                var collect = LocusAsync.CreateTcs<List<string>>();
                PostToMainThread(delegate
                {
                    try
                    {
                        collect.SetResult(EnsureCompileReferencePaths());
                    }
                    catch (Exception ex)
                    {
                        collect.SetException(ex);
                    }
                });

                try
                {
                    paths = await collect.Task;
                }
                catch (Exception ex)
                {
                    return ErrorResponse(requestId, "get_compile_params failed: " + ex.Message);
                }
            }

            try
            {
                string[] defines = SnippetPreprocessorSymbols;
                bool allowUnsafe = GetCachedCompileAllowUnsafe();
                string fingerprint;
                if (TryUseCachedCompileParamsFingerprint(knownFingerprint, out fingerprint))
                {
                    var unchangedPayload = new CompileParamsPayload
                    {
                        unchanged = true,
                        fingerprint = fingerprint,
                        domain_generation = _compileDomainGeneration,
                        lang_version = CompileParamsLanguageVersion,
                        defines = Array.Empty<string>(),
                        reference_paths = Array.Empty<string>(),
                        allow_unsafe = allowUnsafe
                    };
                    return OkResponse(requestId, JsonUtility.ToJson(unchangedPayload));
                }

                fingerprint = ComputeCompileParamsFingerprint(
                    paths, defines, CompileParamsLanguageVersion, allowUnsafe);
                CacheCompileParamsFingerprint(fingerprint);

                var payload = new CompileParamsPayload
                {
                    unchanged = false,
                    fingerprint = fingerprint,
                    domain_generation = _compileDomainGeneration,
                    lang_version = CompileParamsLanguageVersion,
                    defines = defines,
                    reference_paths = paths.ToArray(),
                    allow_unsafe = allowUnsafe
                };

                if (string.Equals(fingerprint, knownFingerprint, StringComparison.Ordinal))
                {
                    payload.unchanged = true;
                    payload.defines = Array.Empty<string>();
                    payload.reference_paths = Array.Empty<string>();
                }

                return OkResponse(requestId, JsonUtility.ToJson(payload));
            }
            catch (Exception ex)
            {
                return ErrorResponse(requestId, "get_compile_params failed: " + ex.Message);
            }
        }

        private static bool TryUseCachedCompileParamsFingerprint(
            string knownFingerprint,
            out string fingerprint)
        {
            fingerprint = null;
            if (string.IsNullOrEmpty(knownFingerprint))
                return false;

            long nowTicks = DateTime.UtcNow.Ticks;
            lock (_compileCacheLock)
            {
                if (!_compileReferencePathsReady ||
                    !_compileParamsFingerprintReady ||
                    string.IsNullOrEmpty(_cachedCompileParamsFingerprint))
                {
                    return false;
                }

                if (!string.Equals(
                        knownFingerprint,
                        _cachedCompileParamsFingerprint,
                        StringComparison.Ordinal))
                {
                    return false;
                }

                if (nowTicks - _cachedCompileParamsFingerprintCheckedAtTicks >=
                    CompileParamsFingerprintAuditIntervalTicks)
                {
                    return false;
                }

                fingerprint = _cachedCompileParamsFingerprint;
                return true;
            }
        }

        private static void CacheCompileParamsFingerprint(string fingerprint)
        {
            lock (_compileCacheLock)
            {
                _cachedCompileParamsFingerprint = fingerprint;
                _compileParamsFingerprintReady = true;
                _cachedCompileParamsFingerprintCheckedAtTicks = DateTime.UtcNow.Ticks;
            }
        }

        /// <summary>
        /// Hash of everything the compile result depends on besides the
        /// source itself: language version, the allow-unsafe flag, defines,
        /// and the reference path list with each file's mtime/size (so a
        /// Unity recompile that rewrites ScriptAssemblies changes the
        /// fingerprint even though the path list is identical).
        /// </summary>
        private static string ComputeCompileParamsFingerprint(
            List<string> referencePaths,
            string[] defines,
            string langVersion,
            bool allowUnsafe)
        {
            var sb = new StringBuilder(referencePaths.Count * 96);
            sb.Append("langver:").Append(langVersion).Append('\n');
            sb.Append("unsafe:").Append(allowUnsafe ? "1" : "0").Append('\n');

            for (int i = 0; i < defines.Length; i++)
                sb.Append("define:").Append(defines[i]).Append('\n');

            for (int i = 0; i < referencePaths.Count; i++)
            {
                string path = referencePaths[i];
                sb.Append(path);

                long mtimeTicks = 0;
                long size = -1;
                try
                {
                    var info = new FileInfo(path);
                    if (info.Exists)
                    {
                        mtimeTicks = info.LastWriteTimeUtc.Ticks;
                        size = info.Length;
                    }
                }
                catch
                {
                }

                sb.Append('|').Append(mtimeTicks.ToString(CultureInfo.InvariantCulture));
                sb.Append('|').Append(size.ToString(CultureInfo.InvariantCulture));
                sb.Append('\n');
            }

            byte[] bytes = Encoding.UTF8.GetBytes(sb.ToString());
            using (var sha1 = System.Security.Cryptography.SHA1.Create())
            {
                byte[] hash = sha1.ComputeHash(bytes);
                var hex = new StringBuilder(hash.Length * 2);
                for (int i = 0; i < hash.Length; i++)
                    hex.Append(hash[i].ToString("x2", CultureInfo.InvariantCulture));
                return hex.ToString();
            }
        }
    }
}
