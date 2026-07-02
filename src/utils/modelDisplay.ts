export function formatModelDisplayName(name: string): string {
  const display = name
    .replace(/\s*\[1m\]\s*/gi, " ")
    .replace(/\s+/g, " ")
    .trim();
  return display || name.trim();
}
