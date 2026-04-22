import { t } from "../i18n";

export function formatTokens(n: number): string {
  if (n >= 1_000_000) return (n / 1_000_000).toFixed(1) + "M";
  if (n >= 1_000) return (n / 1_000).toFixed(1) + "k";
  return n.toString();
}

export function formatUsd(n: number): string {
  if (n >= 1) return `$${n.toFixed(2)}`;
  if (n >= 0.01) return `$${n.toFixed(4)}`;
  return `$${n.toFixed(6)}`;
}

export function formatSessionTime(ts: number): string {
  const d = new Date(ts * 1000);
  const now = new Date();
  if (d.toDateString() === now.toDateString()) {
    return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  }
  return d.toLocaleDateString([], { month: "short", day: "numeric" });
}

export function formatRelativeDate(ts: number): string {
  const now = Date.now() / 1000;
  const diff = now - ts;
  if (diff < 60) return t("time.justNow");
  if (diff < 3600) return t("time.minutesAgo", String(Math.floor(diff / 60)));
  if (diff < 86400) return t("time.hoursAgo", String(Math.floor(diff / 3600)));
  if (diff < 604800) return t("time.daysAgo", String(Math.floor(diff / 86400)));
  if (diff < 2592000) return t("time.weeksAgo", String(Math.floor(diff / 604800)));
  const d = new Date(ts * 1000);
  return `${d.getMonth() + 1}/${d.getDate()}`;
}
