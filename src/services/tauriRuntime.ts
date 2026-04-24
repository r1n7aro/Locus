type TauriInternals = {
  metadata?: {
    currentWindow?: {
      label?: string;
    };
  };
  invoke?: (command: string, args?: Record<string, unknown>) => Promise<unknown>;
};

function getTauriInternals(): TauriInternals | null {
  const internals = (window as unknown as { __TAURI_INTERNALS__?: TauriInternals })
    .__TAURI_INTERNALS__;
  return internals ?? null;
}

export function hasTauriWindowRuntime(): boolean {
  const internals = getTauriInternals();
  return Boolean(internals?.metadata?.currentWindow?.label);
}

export function toggleTauriDevtools(): void {
  const invoke = getTauriInternals()?.invoke;
  if (typeof invoke !== "function") return;
  void invoke("plugin:webview|internal_toggle_devtools").catch(() => {
    /* Devtools toggle is only available in debug builds. */
  });
}

export function installTauriDevtoolsHotkeys(): void {
  window.addEventListener("keydown", (event) => {
    if (event.key !== "F12") return;
    event.preventDefault();
    toggleTauriDevtools();
  });
}
