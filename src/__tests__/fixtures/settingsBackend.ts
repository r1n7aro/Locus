// A desktop settings backend for tests: `settings_*` commands over an
// in-memory value map and `settings-changed` events, installed as the Locus
// runtime. Install it after `vi.resetModules()` and before importing the
// modules under test, so they share this runtime instance.
import type { SettingChange, SettingWrite } from "../../services/settings";

export interface FakeSettingsBackend {
  values: Record<string, unknown>;
  writes: SettingWrite[][];
  /** The `workspaceRef` sent with each write in `writes`. */
  writeScopes: unknown[];
  imports: Record<string, unknown>[];
  /** Broadcasts a change as if another window or the agent wrote it. */
  change(key: string, value: unknown): void;
  /** Rejects the next `settings_set` with this message. */
  failNextWrite(message: string): void;
  /** Holds the next `settings_set` until the returned function is called. */
  holdNextWrite(): () => void;
  /** Resolves once pending IPC promises have settled. */
  settle(): Promise<void>;
  uninstall(): void;
}

function kindOf(value: unknown): string {
  if (typeof value === "boolean") return "bool";
  if (typeof value === "number") return "int";
  if (Array.isArray(value)) return "stringList";
  if (value && typeof value === "object") return "map";
  return "text";
}

export async function installFakeSettingsBackend(
  initial: Record<string, unknown> = {},
): Promise<FakeSettingsBackend> {
  const { installLocusRuntime } = await import("../../services/locusRuntime");
  const values: Record<string, unknown> = { ...initial };
  const handlers = new Set<(payload: unknown) => unknown>();
  const writes: SettingWrite[][] = [];
  const writeScopes: unknown[] = [];
  const imports: Record<string, unknown>[] = [];
  let nextWriteError: string | null = null;
  let nextWriteHold: Promise<void> | null = null;

  const emit = (changes: SettingChange[]) => {
    for (const handler of [...handlers]) handler({ changes });
  };
  const apply = (changes: readonly SettingWrite[]): SettingChange[] => {
    const applied = changes.map(({ key, value }) => ({ key, old: values[key], new: value }));
    for (const { key, value } of changes) values[key] = value;
    emit(applied.filter((change) => JSON.stringify(change.old) !== JSON.stringify(change.new)));
    return applied;
  };

  const uninstall = installLocusRuntime({
    kind: "tauri",
    async invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
      switch (command) {
        case "settings_schema":
          return Object.keys(values).map((key) => ({
            key,
            scope: "app",
            sensitivity: "normal",
            experimental: false,
            restart: false,
            boot: true,
            kind: { type: kindOf(values[key]) },
            default: null,
          })) as T;
        case "settings_get":
          return { ...values } as T;
        case "settings_set": {
          const changes = (args?.changes ?? []) as SettingWrite[];
          writes.push(changes);
          writeScopes.push(args?.workspaceRef ?? null);
          if (nextWriteHold) {
            const hold = nextWriteHold;
            nextWriteHold = null;
            await hold;
          }
          if (nextWriteError) {
            const message = nextWriteError;
            nextWriteError = null;
            throw { code: "settings.invalid_value", message };
          }
          return apply(changes) as T;
        }
        case "settings_import_frontend_legacy": {
          const imported = (args?.values ?? {}) as Record<string, unknown>;
          imports.push(imported);
          const missing = Object.entries(imported).filter(([key]) => !(key in values));
          apply(missing.map(([key, value]) => ({ key, value })));
          return missing.map(([key]) => key) as T;
        }
        default:
          throw new Error(`Unexpected command in settings test backend: ${command}`);
      }
    },
    async subscribe<T>(name: string, handler: (payload: T) => unknown) {
      if (name !== "settings-changed") return () => {};
      const callback = handler as (payload: unknown) => unknown;
      handlers.add(callback);
      return () => handlers.delete(callback);
    },
  });

  return {
    values,
    writes,
    writeScopes,
    imports,
    change(key, value) {
      apply([{ key, value }]);
    },
    failNextWrite(message) {
      nextWriteError = message;
    },
    holdNextWrite() {
      let release!: () => void;
      nextWriteHold = new Promise<void>((resolve) => { release = resolve; });
      return release;
    },
    settle: () => new Promise((resolve) => setTimeout(resolve, 0)),
    uninstall,
  };
}
