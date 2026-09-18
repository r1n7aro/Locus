import { ipcInvoke } from "./ipc";
import type { WorkspaceRef } from "./project";

export interface CsvViewFile { text?: string | null; contentHash?: string | null }
export interface CsvFileScope { filePath: string; workspaceRef?: WorkspaceRef | null; projectId?: string }
export function readCsvViewFile(scope: CsvFileScope): Promise<CsvViewFile> {
  return ipcInvoke("csv_view_read", { ...scope });
}
export function writeCsvViewFile(scope: CsvFileScope, content: string,
  expectedContentHash: string | null, expectedCsvHash: string): Promise<CsvViewFile> {
  return ipcInvoke("csv_view_write", { ...scope, content, expectedContentHash, expectedCsvHash });
}

export function relocateCsvFile(scope: CsvFileScope, targetPath: string, copy: boolean, expectedContentHash: string): Promise<string> {
  return ipcInvoke("csv_file_relocate", { ...scope, targetPath, copy, expectedContentHash });
}
