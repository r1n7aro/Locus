using System;
using System.IO;
using System.Linq;
using System.Runtime.InteropServices;
using System.Threading;

namespace Locus
{
    /// <summary>Same-directory atomic replacement, independently testable on Mono.</summary>
    internal static class LocusAssetApiAtomicFile
    {
        [DllImport("kernel32.dll", EntryPoint = "MoveFileExW", CharSet = CharSet.Unicode, ExactSpelling = true, SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool MoveFileEx(string existingName, string newName, uint flags);
        [DllImport("kernel32.dll", EntryPoint = "CreateFileW", CharSet = CharSet.Unicode, ExactSpelling = true, SetLastError = true)]
        private static extern IntPtr OpenForDeleteAccess(string name, uint access, uint share, IntPtr security, uint creation, uint attributes, IntPtr template);
        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool CloseHandle(IntPtr handle);

        internal static void Replace(string path, byte[] bytes, byte[] expected)
        {
            string fullPath = Path.GetFullPath(path);
            string directory = Path.GetDirectoryName(fullPath);
            string temporary = Path.Combine(directory, ".locus-" + Guid.NewGuid().ToString("N") + ".tmp");
            bool windows = Path.DirectorySeparatorChar == '\\';
            string targetIoPath = windows ? WindowsPath(fullPath) : fullPath;
            string temporaryIoPath = windows ? WindowsPath(temporary) : temporary;
            try
            {
                using (var stream = new FileStream(temporaryIoPath, FileMode.CreateNew, FileAccess.Write, FileShare.None))
                {
                    stream.Write(bytes, 0, bytes.Length);
                    stream.Flush(true);
                }
                int[] delays = { 25, 50, 100, 200 };
                for (int attempt = 0; ; attempt++)
                {
                    if (!File.ReadAllBytes(targetIoPath).SequenceEqual(expected))
                        throw new IOException("revision_conflict: external change preserved at " + fullPath);
                    if (!windows)
                    {
                        File.Replace(temporaryIoPath, targetIoPath, null);
                        return;
                    }
                    // Mono's File.Replace uses ReplaceFileW, whose metadata-copy
                    // semantics can fail on Unity/tempfile-created destinations.
                    // The repository's native writers use atomic rename instead.
                    if (MoveFileEx(temporaryIoPath, targetIoPath, 0x1 | 0x8)) return;
                    int error = Marshal.GetLastWin32Error();
                    int nativeError = error;
                    if (error == 5)
                    {
                        // Windows rename reports ACCESS_DENIED for some handles
                        // opened without FILE_SHARE_DELETE. Prove that case with
                        // a non-mutating open; do not retry ACL/readonly failures.
                        IntPtr handle = OpenForDeleteAccess(targetIoPath, 0x10000, 0x7, IntPtr.Zero, 3, 0x80, IntPtr.Zero);
                        if (handle == new IntPtr(-1))
                        {
                            int accessError = Marshal.GetLastWin32Error();
                            if (accessError == 32 || accessError == 33) error = accessError;
                        }
                        else CloseHandle(handle);
                    }
                    if ((error == 32 || error == 33) && attempt < delays.Length)
                    {
                        Thread.Sleep(delays[attempt]);
                        continue;
                    }
                    throw new IOException("atomic_replace_failed: path=" + fullPath + "; win32=" + error
                        + "; native_error=" + nativeError
                        + "; hresult=0x" + unchecked((int)(0x80070000u | (uint)error)).ToString("X8"));
                }
            }
            catch (Exception error)
            {
                throw new IOException("Asset replacement failed: path=" + fullPath + "; hresult=0x"
                    + error.HResult.ToString("X8") + "; " + error.Message, error);
            }
            finally
            {
                // This is always our generated sibling temporary file, never the
                // original asset. A failed replacement leaves the asset intact.
                if (File.Exists(temporaryIoPath)) File.Delete(temporaryIoPath);
            }
        }

        private static string WindowsPath(string path)
        {
            if (path.StartsWith("\\\\?\\", StringComparison.Ordinal)) return path;
            return path.StartsWith("\\\\", StringComparison.Ordinal)
                ? "\\\\?\\UNC\\" + path.Substring(2)
                : "\\\\?\\" + path;
        }
    }
}
