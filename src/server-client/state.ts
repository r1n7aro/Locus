import { computed, readonly, ref } from 'vue';
import { ipcInvoke } from '../services/ipc';
import type { ModelOption } from '../types';

export interface ServerBinding {
  id: string; serverUrl: string; userId: string; upstreamId: string; kind: 'codex' | 'claude'; name: string;
}
export interface ServerStatus {
  serverUrl: string | null;
  user: { id: string; username: string; role: string } | null;
  upstreams: ServerBinding[];
  serverRevocationConfirmed?: boolean | null;
}
const status = ref<ServerStatus>({ serverUrl: null, user: null, upstreams: [] });
const models = ref<ModelOption[]>([]);
export const serverStatus = readonly(status);
export const additionalModels = computed(() => models.value);
let generation = 0;

export async function refreshServer() {
  const current = ++generation;
  const next = await ipcInvoke<ServerStatus>('server_client_status');
  if (current !== generation) return;
  status.value = next;
  models.value = [];
  if (!next.user) return;
  const available = await ipcInvoke<ModelOption[]>('server_client_models');
  if (current === generation) models.value = available;
}
export async function login(serverUrl: string, username: string, password: string) {
  const current = ++generation;
  models.value = [];
  const next = await ipcInvoke<ServerStatus>('server_client_login', { serverUrl, username, password });
  if (current !== generation) return;
  status.value = next;
  await refreshServer();
}
export async function cancelLogin() {
  ++generation;
  await ipcInvoke('server_client_cancel_login');
}
export async function logout() {
  ++generation;
  models.value = [];
  const next = await ipcInvoke<ServerStatus>('server_client_logout');
  status.value = next;
  return next;
}
