import { ipcInvoke } from "./ipc";
import type { AppStorageInfo } from "../types";

export function getAppStorageInfo(): Promise<AppStorageInfo> {
  return ipcInvoke<AppStorageInfo>("get_app_storage_info");
}

export function openAppStorageDirectory(): Promise<void> {
  return ipcInvoke("open_app_storage_dir");
}

export function scheduleAppStorageMigration(targetPath: string): Promise<AppStorageInfo> {
  return ipcInvoke<AppStorageInfo>("schedule_app_storage_migration", { targetPath });
}

export function clearAppStorageMigration(): Promise<AppStorageInfo> {
  return ipcInvoke<AppStorageInfo>("clear_app_storage_migration");
}
