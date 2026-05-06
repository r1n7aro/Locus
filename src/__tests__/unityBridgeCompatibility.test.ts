import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("unityBridgeCompatibility", () => {
  it("uses a Unity 2020-only pipe accept fallback", () => {
    const bridge = read("locus_unity/Editor/LocusBridge.cs");

    expect(bridge).toContain("#if UNITY_2020");
    expect(bridge).toContain("private const PipeOptions ServerPipeOptions = PipeOptions.None;");
    expect(bridge).toContain("private const PipeOptions ServerPipeOptions = PipeOptions.Asynchronous;");
    expect(bridge).toContain("WaitForConnectionCompat(server, ct);");
    expect(bridge).toContain("await server.WaitForConnectionAsync(ct);");
    expect(bridge).toContain("server.WaitForConnection();");
    expect(bridge).toContain("ct.Register(delegate");
  });
});
