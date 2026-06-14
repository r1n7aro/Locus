import { ipcInvoke } from "./ipc";
import type { AuthStatus } from "../types";

export interface ProviderStatus {
  id: string;
  name: string;
  hasKey: boolean;
  keyHint: string;
  /** Auth state for providers managing credentials outside Locus (claude_code):
   *  false = installed but not logged in; absent = unknown / not applicable. */
  loggedIn?: boolean;
}

export interface CodexStatus {
  authenticated: boolean;
  accountId?: string | null;
  validationFailed?: boolean;
  validationError?: string | null;
}

export interface CodexRateLimitWindow {
  usedPercent: number;
  remainingPercent: number;
  windowMinutes?: number | null;
  resetsAt?: number | null;
}

export interface CodexCreditsSnapshot {
  hasCredits: boolean;
  unlimited: boolean;
  balance?: string | null;
}

export interface CodexRateLimitSnapshot {
  limitId?: string | null;
  limitName?: string | null;
  primary?: CodexRateLimitWindow | null;
  secondary?: CodexRateLimitWindow | null;
  credits?: CodexCreditsSnapshot | null;
  planType?: string | null;
  rateLimitReachedType?: string | null;
}

export interface CodexRateLimitsResponse {
  fetchedAtMs: number;
  rateLimits: CodexRateLimitSnapshot;
  rateLimitsByLimitId: Record<string, CodexRateLimitSnapshot>;
}

export interface CodexLoginInfo {
  userCode: string;
  url: string;
  deviceAuthId: string;
  interval: number;
}

export interface CodexPollResult {
  status: string;
  message?: string;
}

export function getAuthStatus(): Promise<AuthStatus> {
  return ipcInvoke<AuthStatus>("get_auth_status");
}

export function getAuthUrl(): Promise<{ url: string }> {
  return ipcInvoke<{ url: string }>("get_auth_url");
}

export function exchangeAuthCode(code: string): Promise<boolean> {
  return ipcInvoke<boolean>("exchange_auth_code", { code });
}

export function authLogout(): Promise<void> {
  return ipcInvoke("auth_logout");
}

export function saveApiKey(key: string): Promise<boolean> {
  return ipcInvoke<boolean>("save_api_key", { key });
}

export function getProviders(): Promise<ProviderStatus[]> {
  return ipcInvoke<ProviderStatus[]>("get_providers");
}

/** Live connectivity/auth test for the Claude Code CLI. Resolves with the
 *  model reply on success; rejects with an AppError describing the failure. */
export function testClaudeCodeCli(): Promise<string> {
  return ipcInvoke<string>("test_claude_code_cli");
}

export function saveProviderKey(provider: string, key: string): Promise<boolean> {
  return ipcInvoke<boolean>("save_provider_key", { provider, key });
}

export function deleteProviderKey(provider: string): Promise<void> {
  return ipcInvoke("delete_provider_key", { provider });
}

export function codexStatus(): Promise<CodexStatus> {
  return ipcInvoke<CodexStatus>("codex_status");
}

export function codexStartLogin(): Promise<CodexLoginInfo> {
  return ipcInvoke<CodexLoginInfo>("codex_start_login");
}

export function codexPollLogin(deviceAuthId: string, userCode: string): Promise<CodexPollResult> {
  return ipcInvoke<CodexPollResult>("codex_poll_login", { deviceAuthId, userCode });
}

export function codexLogout(): Promise<void> {
  return ipcInvoke("codex_logout");
}

export function codexRetryAuth(): Promise<CodexStatus> {
  return ipcInvoke<CodexStatus>("codex_retry_auth");
}

export function codexRateLimits(): Promise<CodexRateLimitsResponse> {
  return ipcInvoke<CodexRateLimitsResponse>("codex_rate_limits");
}
