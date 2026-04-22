
using UnityEngine;
using UnityEditor;

using System;
using System.Text;

namespace Locus
{
    public static partial class LocusBridge
    {
        [Serializable]
        private class PipeEnvelope
        {
            public string id;
            public string reply_to;
            public string type;
            public bool ok;
            public string message;
            public string error;
        }

        public sealed class ScriptGlobals
        {
            private readonly StringBuilder _output = new StringBuilder(256);

            /// <summary>
            /// Append obj.ToString() to the result buffer (one line per call).
            /// This is the primary way to return plain-text results to the agent.
            /// </summary>
            public void print(object obj)
            {
                _output.AppendLine(obj != null ? obj.ToString() : "null");
            }

            /// <summary>
            /// Serialize a Unity object (or any object) to JSON and append it to the result buffer.
            /// Uses EditorJsonUtility for UnityEngine.Object types (preserves serialized fields,
            /// references, etc.), falls back to JsonUtility for plain C# objects.
            /// This is the preferred way to return structured data to the agent.
            /// </summary>
            public void printJson(object obj)
            {
                if (obj == null)
                {
                    _output.AppendLine("null");
                    return;
                }

                try
                {
                    string json;
                    if (obj is UnityEngine.Object uObj)
                        json = EditorJsonUtility.ToJson(uObj, true);
                    else
                        json = JsonUtility.ToJson(obj, true);

                    _output.AppendLine(json);
                }
                catch (Exception ex)
                {
                    _output.Append("[printJson error: ").Append(ex.Message).Append("] ")
                           .AppendLine(obj.ToString());
                }
            }

            /// <summary>
            /// Clear the result buffer.
            /// </summary>
            public void clear()
            {
                _output.Length = 0;
            }

            public string GetOutput()
            {
                return _output.ToString();
            }
        }
    }
}
