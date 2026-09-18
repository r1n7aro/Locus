import type { ViewPackageDetail } from "../../services/view";
import type { ViewSfcCompileResult } from "./viewCompiler";

// Bump when output, scope rules, or compiler options change. The app version
// also participates in cache keys so an upgrade never executes an old ABI.
export const VIEW_COMPILATION_VERSION = 2;
export interface CompiledViewPackage {
  key: string;
  scopeId: string;
  modules: Record<string, ViewSfcCompileResult>;
  styles: string[];
  scriptKey: string;
}
export interface ViewCompileRequest {
  id: number;
  key: string;
  scopeId: string;
  detail: ViewPackageDetail;
}
export type ViewCompileReply = { id: number; result: CompiledViewPackage } | { id: number; error: string };

export function viewSourceHash(source: string): string {
  let a = 2166136261;
  let b = 5381;
  for (let i = 0; i < source.length; i += 1) {
    a = Math.imul(a ^ source.charCodeAt(i), 16777619);
    b = Math.imul(b, 33) ^ source.charCodeAt(i);
  }
  return `${(a >>> 0).toString(36)}${(b >>> 0).toString(36)}`;
}
