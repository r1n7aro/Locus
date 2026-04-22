export interface ParsedDisplayValue {
  text: string;
  primary: string;
  secondary?: string;
  isReference: boolean;
}

function formatBoolValue(val: string): string {
  const trimmed = val.trim();
  if (trimmed === "0" || trimmed.toLowerCase() === "false") return "False";
  if (trimmed === "1" || trimmed.toLowerCase() === "true") return "True";
  return val;
}

export function parseDisplayValue(
  value: string | undefined,
  valueType?: string,
): ParsedDisplayValue {
  if (value == null || value === "") {
    return { text: "(empty)", primary: "(empty)", isReference: false };
  }

  if (valueType === "bool") {
    const text = formatBoolValue(value);
    return { text, primary: text, isReference: false };
  }

  if (/^\(?\s*fileID:\s*0\s*\)?$/.test(value.trim())) {
    return { text: "None", primary: "None", isReference: false };
  }

  const cleaned = value.replace(/\s*\(fileID:\d+\)/g, "").trim();
  const pathMatch = cleaned.match(/^Assets\/(.+)\/([^/]+)$/);
  if (pathMatch) {
    const segments = pathMatch[1].split("/");
    const parentDir = segments[segments.length - 1] ?? "";
    const fileName = pathMatch[2];
    const nameNoExt = fileName.replace(/\.(asset|prefab|mat|controller|anim|unity|meta)$/i, "");
    return {
      text: cleaned,
      primary: nameNoExt,
      secondary: parentDir ? `${parentDir}/` : undefined,
      isReference: true,
    };
  }

  if (/^fileID:\d+$/.test(cleaned)) {
    return { text: "None", primary: "None", isReference: false };
  }

  return { text: cleaned || "(empty)", primary: cleaned || "(empty)", isReference: false };
}
