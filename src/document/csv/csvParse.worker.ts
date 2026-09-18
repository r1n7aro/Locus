import { parseCsvDocument } from "./csvDocument";
self.onmessage = ({ data }: MessageEvent<{ id: number; source: string; delimiter?: string }>) => {
  try { self.postMessage({ id: data.id, document: parseCsvDocument(data.source, data.delimiter) }); }
  catch (error) { self.postMessage({ id: data.id, error: error instanceof Error ? error.message : String(error),
    offset: (error as { offset?: number }).offset ?? 0 }); }
};
