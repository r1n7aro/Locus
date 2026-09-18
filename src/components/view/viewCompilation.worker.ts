import { compileViewPackageSource } from "./viewCompilationCore";
import type { ViewCompileReply, ViewCompileRequest } from "./viewCompilationTypes";

const worker = self as unknown as { onmessage: ((event: MessageEvent<ViewCompileRequest>) => void) | null; postMessage(reply: ViewCompileReply): void };
worker.onmessage = ({ data }) => {
  try { worker.postMessage({ id: data.id, result: compileViewPackageSource(data) }); }
  catch (error) { worker.postMessage({ id: data.id, error: error instanceof Error ? error.message : String(error) }); }
};
