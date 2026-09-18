import { DocumentSessionCache } from "../../document/documentSessionCache";
import type { CsvView } from "../../document/csv/csvView";

export interface CsvEditorSnapshot {
  text: string; hash: string; view: CsvView; viewBaseline: string; viewHash: string | null;
  dirty: boolean; mode: string;
  grid?: { row: number; column: number; endRow?: number; endColumn?: number; scrollTop: number; scrollLeft: number; zoom?: number };
  history: Array<{ text: string; view: CsvView }>; historyIndex: number;
}
export const csvEditorSessions = new DocumentSessionCache<CsvEditorSnapshot>({ capacity: 12, canEvict: (snapshot) => !snapshot.dirty });
