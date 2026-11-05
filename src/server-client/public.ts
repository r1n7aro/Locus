// The public dependency graph deliberately terminates here.
import { computed } from 'vue';
import type { ModelOption } from '../types';
export const ServerConnection = null;
export const additionalModels = computed<ModelOption[]>(() => []);
export function protocolModelId(model: string): string { return model; }
