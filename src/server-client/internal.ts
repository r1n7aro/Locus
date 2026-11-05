export { default as ServerConnection } from './ServerConnection.vue';
export { additionalModels } from './state';
export function protocolModelId(model: string): string {
  return model.startsWith('locus-server/') ? model.split('/').slice(2).join('/') : model;
}
import { refreshServer } from './state';
// Internal-only bootstrap: no credential/network code is reachable in public builds.
void refreshServer().catch(() => {});
