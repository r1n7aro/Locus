import { execFile } from "node:child_process";
import { mkdir, writeFile } from "node:fs/promises";
import path from "node:path";
import { promisify } from "node:util";
import { describe, expect, it } from "vitest";

// Uses the repository Cargo cache; opt in because building the desktop Rust
// test binary is substantially more expensive than the frontend suite.
describe.skipIf(process.env.LOCUS_YAML_READ_RUST_TESTS !== "1")("Unity YAML reader runtime", () => {
  it("executes production dispatch, response decoding, limits and hidden-field resolution", async () => {
    const { stdout, stderr } = await promisify(execFile)("cargo", [
      "test", "--manifest-path", path.resolve("src-tauri/Cargo.toml"), "--lib",
      "--", "yaml_read_extension", "live_hidden_field", "--nocapture",
    ], { timeout: 900_000, maxBuffer: 4 * 1024 * 1024, windowsHide: true });
    await mkdir(path.resolve(".tmp"), { recursive: true });
    await writeFile(path.resolve(".tmp/unity-yaml-read-rust-tests.log"), stdout + stderr);
    expect(stdout).toContain("test result: ok.");
    expect(stdout).toContain("yaml_read_extension_replaces_root_text_before_default_read");
    expect(stdout).toContain("live_hidden_fields_use_exact_serialized_targets_and_unsaved_values");
    console.log(stdout.trim());
    if (process.env.LOCUS_YAML_READ_LIVE_PROJECT && process.env.LOCUS_YAML_READ_LIVE_PATH) {
      const live = await promisify(execFile)("cargo", [
        "test", "--manifest-path", path.resolve("src-tauri/Cargo.toml"), "--lib",
        "live_hidden_field_connected_editor", "--", "--ignored", "--nocapture",
      ], { timeout: 120_000, maxBuffer: 4 * 1024 * 1024, windowsHide: true });
      await writeFile(path.resolve(".tmp/unity-yaml-read-live-test.log"), live.stdout + live.stderr);
      expect(live.stdout).toContain("LIVE_HIDDEN_FIELD");
    }
  }, 900_000);
});
