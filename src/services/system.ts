import { ipcInvoke } from "./ipc";

export function getSystemLocale(): Promise<string | null> {
  return ipcInvoke<string | null>("get_system_locale");
}

export function sendSystemNotification(title: string, body?: string | null): Promise<void> {
  return ipcInvoke<void>(
    "send_system_notification",
    {
      title,
      body: body ?? null,
    },
    { throwOnError: false },
  );
}
