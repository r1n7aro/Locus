declare module '@locus/server-client' {
  import type { Component } from 'vue';
  export const ServerConnection: Component | null;
  export const additionalModels: import('vue').ComputedRef<import('../types').ModelOption[]>;
  export function protocolModelId(model: string): string;
}
declare const __LOCUS_SERVER_CLIENT__: boolean;
