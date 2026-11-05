import { ipcInvoke } from "./ipc";

/** Desktop side of mobile remote access (src-tauri/src/remote_access). */
export const REMOTE_ACCESS_CHANGED_EVENT = "remote-access-changed";
export const REMOTE_ACCESS_PAIRING_REQUEST_EVENT = "remote-access-pairing-request";
export const REMOTE_ACCESS_PAIRING_SETTLED_EVENT = "remote-access-pairing-settled";

export type ListenerKind = "loopback" | "lan" | "tailscale";
export type InterfaceKind = "lan" | "tailscale" | "other";

export interface RemoteAccessSettings {
  enabled: boolean;
  port: number;
  loopback: boolean;
  interfaces: string[];
  hostnames: string[];
  requireConfirmation: boolean;
  tokenIdleDays: number;
  /** Offer the bundled Locus Remote APK on the LAN while pairing. */
  appDownload: boolean;
  desktopId: string;
}

export type RemoteAccessSettingsInput = Omit<RemoteAccessSettings, "desktopId">;

export interface RemoteListener {
  address: string;
  url: string;
  kind: ListenerKind;
  secure: boolean;
}

export interface RemoteListenerError {
  address: string;
  error: string;
}

export interface RemoteNetworkInterface {
  name: string;
  kind: InterfaceKind;
  addresses: string[];
}

export interface RemoteDevice {
  id: string;
  name: string;
  platform: string;
  pairedAtMs: number;
  lastSeenAtMs: number;
  lastAddress: string | null;
  /** Approve tool calls and change things; otherwise view and chat only. */
  fullControl: boolean;
  connected: boolean;
}

export interface RemotePairingStatus {
  sessionId: string;
  expiresAtMs: number;
  attemptsLeft: number;
  awaitingConfirmation: boolean;
}

export interface RemoteAccessState {
  settings: RemoteAccessSettings;
  desktopName: string;
  running: boolean;
  listeners: RemoteListener[];
  listenerErrors: RemoteListenerError[];
  lastError: string | null;
  interfaces: RemoteNetworkInterface[];
  fingerprint: string | null;
  devices: RemoteDevice[];
  connections: number;
  pairing: RemotePairingStatus | null;
  tailscaleServeCommand: string;
  /** Locus Remote APK bundled with this installer; null in other flavors. */
  appPackage: RemoteAppPackage | null;
}

export interface RemoteAppPackage {
  version: string;
  sizeBytes: number;
}

export interface RemoteCandidateAddress {
  u: string;
  pin: boolean;
}

export interface RemotePairingView {
  sessionId: string;
  code: string;
  codeUsable: boolean;
  link: string;
  qrSvg: string;
  expiresAtMs: number;
  fingerprint: string;
  addresses: RemoteCandidateAddress[];
  /** A phone camera scanning the code opens this desktop's download page. */
  appDownload: boolean;
}

export interface RemotePairingRequest {
  requestId: string;
  deviceName: string;
  platform: string;
  address: string;
  expiresAtMs: number;
}

/** The bundled Locus Remote APK, if this installer flavor includes it. */
export function getBundledRemoteApp(): Promise<RemoteAppPackage | null> {
  return ipcInvoke<RemoteAppPackage | null>("remote_access_app_package");
}

export function getRemoteAccessState(): Promise<RemoteAccessState> {
  return ipcInvoke<RemoteAccessState>("remote_access_get_state");
}

export function saveRemoteAccessSettings(settings: RemoteAccessSettingsInput): Promise<RemoteAccessState> {
  return ipcInvoke<RemoteAccessState>("remote_access_save_settings", { settings });
}

export function startRemotePairing(): Promise<RemotePairingView> {
  return ipcInvoke<RemotePairingView>("remote_access_start_pairing");
}

export function cancelRemotePairing(): Promise<void> {
  return ipcInvoke("remote_access_cancel_pairing");
}

export function resolveRemotePairing(requestId: string, allow: boolean): Promise<boolean> {
  return ipcInvoke<boolean>("remote_access_resolve_pairing", { requestId, allow });
}

export function setRemoteDevicePermission(deviceId: string, fullControl: boolean): Promise<void> {
  return ipcInvoke("remote_access_set_device_permission", { deviceId, fullControl });
}

export function renameRemoteDevice(deviceId: string, name: string): Promise<void> {
  return ipcInvoke("remote_access_rename_device", { deviceId, name });
}

export function revokeRemoteDevice(deviceId: string): Promise<void> {
  return ipcInvoke("remote_access_revoke_device", { deviceId });
}

export function revokeAllRemoteDevices(): Promise<void> {
  return ipcInvoke("remote_access_revoke_all_devices");
}

export function resetRemoteCertificate(): Promise<RemoteAccessState> {
  return ipcInvoke<RemoteAccessState>("remote_access_reset_certificate");
}

export function openRemoteFirewallSettings(): Promise<void> {
  return ipcInvoke("remote_access_open_firewall_settings");
}

export function settingsInput(settings: RemoteAccessSettings): RemoteAccessSettingsInput {
  return {
    enabled: settings.enabled,
    port: settings.port,
    loopback: settings.loopback,
    interfaces: [...settings.interfaces],
    hostnames: [...settings.hostnames],
    requireConfirmation: settings.requireConfirmation,
    tokenIdleDays: settings.tokenIdleDays,
    appDownload: settings.appDownload,
  };
}
