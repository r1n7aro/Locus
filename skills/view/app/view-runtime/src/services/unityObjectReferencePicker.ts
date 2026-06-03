import type { AssetSearchResult } from "../types";

export const UNITY_OBJECT_REFERENCE_SEARCH_ROOTS = [
  "Assets",
  "Packages",
  "ProjectSettings",
] as const;

export interface UnityObjectReferenceFilter {
  referenceTypeFullName?: string;
  referenceTypeAssembly?: string;
  currentValue?: string;
  limit?: number;
}

export interface UnityObjectReferenceTypeRule {
  typeNames: string[];
  queryType?: string;
  queryText?: string;
  extensions?: string[];
  kinds?: string[];
  subAssetTypeNames?: string[];
  requireSubAsset?: boolean;
  mainAssetOnly?: boolean;
  scriptableObject?: boolean;
  component?: boolean;
  broad?: boolean;
}

const AUDIO_EXTENSIONS = ["wav", "mp3", "ogg", "aif", "aiff"];
const TEXTURE_EXTENSIONS = [
  "png",
  "jpg",
  "jpeg",
  "tga",
  "psd",
  "tif",
  "tiff",
  "bmp",
  "gif",
  "exr",
  "hdr",
];
const MODEL_EXTENSIONS = ["fbx", "obj", "blend", "dae", "3ds", "max"];

const TYPE_RULES: UnityObjectReferenceTypeRule[] = [
  {
    typeNames: ["monoscript", "script"],
    queryType: "script",
    extensions: ["cs"],
    kinds: ["script"],
  },
  {
    typeNames: ["audioclip"],
    queryType: "audio",
    extensions: AUDIO_EXTENSIONS,
    kinds: ["audio"],
  },
  {
    typeNames: ["audiomixer"],
    queryText: "mixer",
    extensions: ["mixer"],
    mainAssetOnly: true,
  },
  {
    typeNames: ["audiomixergroup"],
    queryType: "audiomixergroup",
    subAssetTypeNames: ["audiomixergroup", "audiomixergroupcontroller"],
    requireSubAsset: true,
  },
  {
    typeNames: ["audiomixersnapshot"],
    queryType: "audiomixersnapshot",
    subAssetTypeNames: ["audiomixersnapshot", "audiomixersnapshotcontroller"],
    requireSubAsset: true,
  },
  {
    typeNames: ["texture", "texture2d"],
    queryType: "texture",
    extensions: TEXTURE_EXTENSIONS,
    kinds: ["texture"],
  },
  {
    typeNames: ["sprite"],
    queryType: "sprite",
    subAssetTypeNames: ["sprite"],
    requireSubAsset: true,
  },
  {
    typeNames: ["material"],
    queryType: "material",
    extensions: ["mat"],
    kinds: ["material"],
  },
  {
    typeNames: ["animationclip"],
    queryType: "animation",
    extensions: ["anim"],
    kinds: ["animation"],
    subAssetTypeNames: ["animationclip"],
  },
  {
    typeNames: ["animatorcontroller", "runtimeanimatorcontroller", "animatoroverridecontroller"],
    queryType: "controller",
    extensions: ["controller", "overridecontroller"],
    kinds: ["animatorController"],
  },
  {
    typeNames: ["shader"],
    queryType: "shader",
    extensions: ["shader", "compute", "cginc", "hlsl", "glsl"],
    kinds: ["shader"],
  },
  {
    typeNames: ["sceneasset"],
    queryType: "scene",
    extensions: ["unity"],
    kinds: ["scene"],
  },
  {
    typeNames: ["gameobject", "transform", "recttransform"],
    queryType: "prefab",
    extensions: ["prefab"],
    kinds: ["prefab"],
  },
  {
    typeNames: ["mesh"],
    queryType: "model",
    extensions: MODEL_EXTENSIONS,
    kinds: ["model"],
    subAssetTypeNames: ["mesh"],
  },
  {
    typeNames: ["physicmaterial", "physicmaterial2d", "physicsmaterial2d"],
    extensions: ["physicmaterial", "physicsmaterial2d"],
  },
  {
    typeNames: ["scriptableobject"],
    queryType: "asset",
    extensions: ["asset"],
    kinds: ["genericAsset"],
    scriptableObject: true,
  },
  {
    typeNames: ["object"],
    broad: true,
  },
];

const COMPONENT_BASE_TYPES = new Set([
  "behaviour",
  "component",
  "monobehaviour",
  "renderer",
  "collider",
  "collider2d",
  "rigidbody",
  "rigidbody2d",
]);

const BROAD_OBJECT_RULE: UnityObjectReferenceTypeRule = {
  typeNames: ["object"],
  broad: true,
};

export function normalizeUnityObjectReferenceType(typeFullName?: string): string {
  const raw = (typeFullName || "").trim();
  if (!raw) return "";
  const withoutAssembly = raw.split(",")[0]?.trim() || raw;
  const parts = withoutAssembly.split(".");
  const last = parts[parts.length - 1] || withoutAssembly;
  const nested = last.split("+").pop() || last;
  return nested.trim();
}

export function unityObjectReferenceTypeKey(typeFullName?: string): string {
  return normalizeUnityObjectReferenceType(typeFullName)
    .replace(/[^a-z0-9]/gi, "")
    .toLowerCase();
}

export function unityObjectReferenceTypeHint(typeFullName?: string): string {
  const shortName = normalizeUnityObjectReferenceType(typeFullName);
  return shortName || "Unity Object";
}

export function getUnityObjectReferenceTypeRule(typeFullName?: string): UnityObjectReferenceTypeRule {
  const key = unityObjectReferenceTypeKey(typeFullName);
  const rule = TYPE_RULES.find((candidate) => candidate.typeNames.includes(key));
  if (rule) return rule;
  if (!key) return BROAD_OBJECT_RULE;
  if (COMPONENT_BASE_TYPES.has(key)) {
    return {
      typeNames: [key],
      queryType: "prefab",
      extensions: ["prefab"],
      kinds: ["prefab"],
    };
  }
  if (key.endsWith("component") || key.endsWith("behaviour")) {
    return {
      typeNames: [key],
      queryType: "prefab",
      component: true,
      extensions: ["prefab"],
      kinds: ["prefab"],
    };
  }
  return {
    typeNames: [key],
    queryType: "asset",
    extensions: ["asset"],
    kinds: ["genericAsset"],
    scriptableObject: true,
  };
}

export function unityObjectReferenceSearchQuery(
  input: string,
  filter: UnityObjectReferenceFilter = {},
): string {
  const query = input.trim();
  const rule = getUnityObjectReferenceTypeRule(filter.referenceTypeFullName);
  const shortName = normalizeUnityObjectReferenceType(filter.referenceTypeFullName);
  const tokens: string[] = [];
  const typeKey = unityObjectReferenceTypeKey(shortName);
  if (rule.scriptableObject && shortName && typeKey !== "scriptableobject") {
    tokens.push(`t:${shortName}`);
  } else if (rule.queryType) {
    tokens.push(`t:${rule.queryType}`);
  }
  if (rule.component && shortName) tokens.push(`component:${shortName}`);
  if (rule.queryText) tokens.push(rule.queryText);
  if (!query && !rule.queryType && !rule.queryText && shortName && !rule.broad) tokens.push(shortName);
  if (query) tokens.push(query);
  return tokens.join(" ").trim();
}

export function normalizeUnityObjectReferencePath(path: string): string {
  return path.trim().replace(/\\/g, "/");
}

function unityObjectReferenceBasePath(path: string): string {
  return normalizeUnityObjectReferencePath(path).split(/[?#]/)[0] || "";
}

export function unityObjectReferenceAssetKey(result: AssetSearchResult): string {
  const path = normalizeUnityObjectReferencePath(result.path);
  if (result.objectKey) return result.objectKey;
  if (result.isSubAsset && result.name.trim()) return `${path}/${result.name.trim()}`;
  if (result.fileId != null) return `sub:${path}:${result.fileId}`;
  return path;
}

export function unityObjectReferenceValueForSearchResult(result: AssetSearchResult): string {
  const path = normalizeUnityObjectReferencePath(result.path);
  if (result.isSubAsset && result.name.trim()) {
    return `${path}/${result.name.trim()}`;
  }
  return path;
}

export function unityObjectReferenceExtension(path: string): string {
  const clean = unityObjectReferenceBasePath(path);
  const name = clean.split("/").pop() || clean;
  const dot = name.lastIndexOf(".");
  return dot >= 0 ? name.slice(dot + 1).toLowerCase() : "";
}

function matchesExt(path: string, extensions: readonly string[]): boolean {
  if (extensions.length === 0) return true;
  return extensions.includes(unityObjectReferenceExtension(path));
}

function matchesKind(result: AssetSearchResult, kinds: readonly string[]): boolean {
  if (kinds.length === 0) return true;
  const kind = (result.kind || "").toLowerCase();
  return kinds.some((item) => item.toLowerCase() === kind);
}

function normalizeSearchText(value: string): string {
  return value.replace(/[^a-z0-9]/gi, "").toLowerCase();
}

function resultSubAssetTypeMatches(result: AssetSearchResult, typeNames: readonly string[] = []): boolean {
  if (!result.isSubAsset || typeNames.length === 0) return false;
  const typeLabel = normalizeSearchText(result.typeLabel || "");
  if (!typeLabel) return false;
  return typeNames.some((typeName) => {
    const expected = normalizeSearchText(typeName);
    return typeLabel === expected || typeLabel.includes(expected);
  });
}

function resultTypeText(result: AssetSearchResult): string {
  return [
    result.name,
    result.path,
    result.kind,
    result.typeLabel,
    result.typeSearch,
  ]
    .filter(Boolean)
    .join(" ");
}

function matchesScriptableObjectType(result: AssetSearchResult, typeFullName?: string): boolean {
  const shortName = normalizeUnityObjectReferenceType(typeFullName);
  if (!shortName || unityObjectReferenceTypeKey(shortName) === "scriptableobject") return true;
  const expected = normalizeSearchText(shortName);
  const typeLabel = normalizeSearchText(result.typeLabel || "");
  if (typeLabel && (typeLabel === expected || typeLabel.includes(expected))) return true;
  const typeSearch = normalizeSearchText(result.typeSearch || "");
  if (typeSearch && (typeSearch === expected || typeSearch.includes(expected))) return true;
  return normalizeSearchText(resultTypeText(result)).includes(expected);
}

export function isUnityObjectReferenceSearchResult(
  result: AssetSearchResult,
  filter: UnityObjectReferenceFilter = {},
): boolean {
  const rule = getUnityObjectReferenceTypeRule(filter.referenceTypeFullName);
  if (rule.broad) return true;
  if (rule.mainAssetOnly && result.isSubAsset) return false;
  const subAssetTypeMatch = resultSubAssetTypeMatches(result, rule.subAssetTypeNames);
  if (rule.requireSubAsset) return subAssetTypeMatch;
  if (result.isSubAsset && (rule.subAssetTypeNames?.length ?? 0) > 0) return subAssetTypeMatch;
  if (subAssetTypeMatch) return true;
  const extensions = rule.extensions ?? [];
  const kinds = rule.kinds ?? [];
  const pathMatch = matchesExt(result.path, extensions);
  const kindMatch = matchesKind(result, kinds);
  if (extensions.length > 0 && kinds.length > 0) {
    if (!pathMatch && !kindMatch) return false;
  } else if (extensions.length > 0) {
    if (!pathMatch) return false;
  } else if (kinds.length > 0) {
    if (!kindMatch) return false;
  }
  if (rule.scriptableObject) {
    return matchesScriptableObjectType(result, filter.referenceTypeFullName);
  }
  return true;
}

function referencePickerScore(result: AssetSearchResult, filter: UnityObjectReferenceFilter): number {
  const rule = getUnityObjectReferenceTypeRule(filter.referenceTypeFullName);
  let score = result.matchScore || 0;
  if (resultSubAssetTypeMatches(result, rule.subAssetTypeNames)) score += 140;
  if (rule.extensions?.includes(unityObjectReferenceExtension(result.path))) score += 80;
  if (rule.kinds?.some((kind) => kind.toLowerCase() === (result.kind || "").toLowerCase())) score += 60;
  if (rule.scriptableObject && matchesScriptableObjectType(result, filter.referenceTypeFullName)) score += 100;
  if (normalizeUnityObjectReferencePath(filter.currentValue || "") === unityObjectReferenceValueForSearchResult(result)) {
    score += 200;
  }
  return score;
}

export function filterUnityObjectReferenceSearchResults(
  results: AssetSearchResult[],
  filter: UnityObjectReferenceFilter = {},
): AssetSearchResult[] {
  const seen = new Set<string>();
  const filtered = results
    .filter((result) => {
      const path = normalizeUnityObjectReferencePath(result.path);
      const key = unityObjectReferenceAssetKey(result);
      if (!path || seen.has(key)) return false;
      seen.add(key);
      return isUnityObjectReferenceSearchResult(result, filter);
    })
    .sort((a, b) => referencePickerScore(b, filter) - referencePickerScore(a, filter));
  const limit = Number.isFinite(filter.limit) ? Math.max(1, Number(filter.limit)) : 0;
  return limit > 0 ? filtered.slice(0, limit) : filtered;
}
