using System.Text;

namespace Locus.CompileServer;

/// <summary>
/// Content-Length framed message transport over raw stdio streams, the same
/// framing the LSP (and Locus' csharp_lsp client) uses.
/// </summary>
public sealed class StdioTransport
{
    private readonly Stream _input;
    private readonly Stream _output;
    private readonly object _writeLock = new();

    public StdioTransport(Stream input, Stream output)
    {
        _input = input;
        _output = output;
    }

    /// <summary>Read one framed message body; null when the stream ends.</summary>
    public byte[]? ReadMessage()
    {
        int contentLength = -1;
        while (true)
        {
            string? line = ReadHeaderLine();
            if (line == null)
                return null;
            if (line.Length == 0)
                break;
            int colon = line.IndexOf(':');
            if (colon <= 0)
                continue;
            string name = line[..colon].Trim();
            if (name.Equals("Content-Length", StringComparison.OrdinalIgnoreCase) &&
                int.TryParse(line[(colon + 1)..].Trim(), out int parsed))
            {
                contentLength = parsed;
            }
        }

        if (contentLength < 0)
            return null;

        var body = new byte[contentLength];
        int read = 0;
        while (read < contentLength)
        {
            int n = _input.Read(body, read, contentLength - read);
            if (n <= 0)
                return null;
            read += n;
        }
        return body;
    }

    public void WriteMessage(byte[] body)
    {
        var header = Encoding.ASCII.GetBytes($"Content-Length: {body.Length}\r\n\r\n");
        lock (_writeLock)
        {
            _output.Write(header, 0, header.Length);
            _output.Write(body, 0, body.Length);
            _output.Flush();
        }
    }

    private string? ReadHeaderLine()
    {
        var sb = new StringBuilder(32);
        while (true)
        {
            int b = _input.ReadByte();
            if (b < 0)
                return sb.Length == 0 ? null : sb.ToString();
            if (b == '\n')
                return sb.ToString().TrimEnd('\r');
            sb.Append((char)b);
        }
    }
}

/// <summary>Thrown by handlers when the request payload is malformed.</summary>
public sealed class RpcInvalidParamsException : Exception
{
    public RpcInvalidParamsException(string message) : base(message) { }
}
