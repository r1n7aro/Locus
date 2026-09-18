export interface UnityDriverResult {
  code?: number | null;
  signal?: string | null;
  finishedOk?: boolean;
  driverError?: string;
}
export function unityDriverExitCode(result: UnityDriverResult): number;
