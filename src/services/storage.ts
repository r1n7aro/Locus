import { ipcInvoke } from "./ipc";
import type { AppStorageInfo, AppTempInfo } from "../types";

export function getAppStorageInfo(): Promise<AppStorageInfo> {
  return ipcInvoke<AppStorageInfo>("get_app_storage_info");
}

export function getAppTempInfo(): Promise<AppTempInfo> {
  return ipcInvoke<AppTempInfo>("get_app_temp_info");
}

export function clearAppTempDir(): Promise<AppTempInfo> {
  return ipcInvoke<AppTempInfo>("clear_app_temp_dir");
}

export function openAppStorageDirectory(): Promise<void> {
  return ipcInvoke("open_app_storage_dir");
}

export function openAppTempDirectory(): Promise<void> {
  return ipcInvoke("open_app_temp_dir");
}

export function scheduleAppStorageMigration(targetPath: string): Promise<AppStorageInfo> {
  return ipcInvoke<AppStorageInfo>("schedule_app_storage_migration", { targetPath });
}

export function clearAppStorageMigration(): Promise<AppStorageInfo> {
  return ipcInvoke<AppStorageInfo>("clear_app_storage_migration");
}
