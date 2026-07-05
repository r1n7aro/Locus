import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { useAuthStore } from "../stores/auth";

const authServiceMocks = vi.hoisted(() => ({
  getAuthStatus: vi.fn(),
  getProviders: vi.fn(),
  codexStatus: vi.fn(),
}));

vi.mock("../services/auth", () => authServiceMocks);
vi.mock("../config/providerVisibility", () => ({
  filterVisibleProviders: (providers: unknown) => providers,
}));

function createDeferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

describe("useAuthStore", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  it("marks authChecked only after the full provider check completes", async () => {
    const codexDeferred = createDeferred<{ authenticated: boolean; accountId: string | null }>();

    authServiceMocks.getAuthStatus.mockResolvedValue({
      authenticated: false,
      hasApiKey: false,
      email: null,
    });
    authServiceMocks.getProviders.mockResolvedValue([]);
    authServiceMocks.codexStatus.mockReturnValue(codexDeferred.promise);

    const authStore = useAuthStore();
    const checkPromise = authStore.checkAuth();

    await Promise.resolve();

    expect(authStore.authChecked).toBe(false);

    codexDeferred.resolve({ authenticated: true, accountId: "acct_123" });
    await checkPromise;

    expect(authStore.authChecked).toBe(true);
    expect(authStore.codexAuthenticated).toBe(true);
  });

  it("keeps authChecked latched during a re-check (no mid-session UI teardown)", async () => {
    authServiceMocks.getAuthStatus.mockResolvedValue({
      authenticated: true,
      hasApiKey: true,
      email: "user@example.com",
    });
    authServiceMocks.getProviders.mockResolvedValue([]);
    authServiceMocks.codexStatus.mockResolvedValue({ authenticated: false, accountId: null });

    const authStore = useAuthStore();
    await authStore.checkAuth();
    expect(authStore.authChecked).toBe(true);

    // Re-check (e.g. the fallback refresh when leaving the settings tab) must
    // never flip authChecked back to false — App.vue gates the entire layout
    // on it, and a transient false unmounts and rebuilds every mounted tab.
    const codexDeferred = createDeferred<{ authenticated: boolean; accountId: string | null }>();
    authServiceMocks.codexStatus.mockReturnValue(codexDeferred.promise);

    const recheckPromise = authStore.checkAuth();
    await Promise.resolve();
    expect(authStore.authChecked).toBe(true);

    codexDeferred.resolve({ authenticated: true, accountId: "acct_123" });
    await recheckPromise;
    expect(authStore.authChecked).toBe(true);
  });

  it("keeps the previous codex status when codexStatus fails", async () => {
    authServiceMocks.getAuthStatus.mockResolvedValue({
      authenticated: false,
      hasApiKey: false,
      email: null,
    });
    authServiceMocks.getProviders.mockResolvedValue([]);
    authServiceMocks.codexStatus.mockRejectedValue(new Error("ipc failed"));

    const authStore = useAuthStore();
    authStore.codexAuthenticated = true;

    const failures = await authStore.loadProviderStatus();

    expect(authStore.codexAuthenticated).toBe(true);
    expect(failures).toEqual([
      expect.objectContaining({
        target: "codex",
        error: expect.objectContaining({
          message: "ipc failed",
        }),
      }),
    ]);
  });

  it("returns provider failures from checkAuth when startup restore fails", async () => {
    authServiceMocks.getAuthStatus.mockResolvedValue({
      authenticated: false,
      hasApiKey: false,
      email: null,
    });
    authServiceMocks.getProviders.mockRejectedValue(new Error("keychain unavailable"));
    authServiceMocks.codexStatus.mockResolvedValue({
      authenticated: false,
      accountId: null,
    });

    const authStore = useAuthStore();
    const failures = await authStore.checkAuth();

    expect(failures).toEqual([
      expect.objectContaining({
        target: "providers",
        error: expect.objectContaining({
          message: "keychain unavailable",
        }),
      }),
    ]);
  });
});
