import { t } from "../../i18n";
import type { InspectorField, InspectorPanel } from "../../types";
import {
  partitionFields,
  type ComponentRendererConfig,
} from "./rendererRegistry";

type Side = "before" | "after";

export interface ParticleGradientStop {
  offset: number;
  color: string;
}

export interface ParticleSemanticValue {
  text: string;
  color?: string;
  gradientStops?: ParticleGradientStop[];
}

export interface ParticleSummaryRow {
  id: string;
  label: string;
  propertyPath: string;
  changeKind: InspectorField["changeKind"];
  before?: ParticleSemanticValue;
  after?: ParticleSemanticValue;
}

export interface ParticleSemanticSection {
  titleKey: string;
  summaryRows: ParticleSummaryRow[];
  otherFields: InspectorField[];
}

export interface ParticleSemanticView {
  sections: ParticleSemanticSection[];
  otherFields: InspectorField[];
  hiddenCount: number;
}

interface ModuleSummaryResult {
  summaryRows: ParticleSummaryRow[];
  consumedPaths: Set<string>;
}

const COLOR_RE =
  /^\{?\s*r:\s*([\d.+-]+),\s*g:\s*([\d.+-]+),\s*b:\s*([\d.+-]+),\s*a:\s*([\d.+-]+)\s*\}?$/;

const MAIN_ORDER = [
  "enabled",
  "duration",
  "looping",
  "prewarm",
  "playonawake",
  "startdelay",
  "startlifetime",
  "startspeed",
  "startsize3d",
  "startsize",
  "startsizex",
  "startsizey",
  "startsizez",
  "rotation3d",
  "startrotation",
  "startrotationx",
  "startrotationy",
  "startrotationz",
  "randomizerotationdirection",
  "fliprotation",
  "startcolor",
  "gravitysource",
  "gravitymodifier",
  "simulationspace",
  "customsimulationspace",
  "simulationspeed",
  "scalingmode",
  "emittervelocitymode",
  "maxnumparticles",
];

const EMISSION_ORDER = [
  "enabled",
  "rateovertime",
  "rateoverdistance",
  "bursts",
];

const SHAPE_ORDER = [
  "enabled",
  "shapevalue",
  "randomdirectionamount",
  "sphericaldirectionamount",
  "aligntodirection",
  "radius",
  "radiusthickness",
  "angle",
  "length",
  "arc",
  "box",
  "position",
  "rotation",
  "scale",
  "mesh",
  "meshshapetype",
  "sprite",
];

const CURVE_MODE_LABELS: Record<number, string> = {
  0: "diff.particle.mode.constant",
  1: "diff.particle.mode.curve",
  2: "diff.particle.mode.twoCurves",
  3: "diff.particle.mode.twoConstants",
};

const GRADIENT_MODE_LABELS: Record<number, string> = {
  0: "diff.particle.mode.color",
  1: "diff.particle.mode.gradient",
  2: "diff.particle.mode.twoColors",
  3: "diff.particle.mode.twoGradients",
  4: "diff.particle.mode.randomColor",
};

const BOOL_TRUE = "True";
const BOOL_FALSE = "False";

export function buildParticleSystemSemanticView(
  panel: InspectorPanel,
  config: ComponentRendererConfig,
): ParticleSemanticView {
  const raw = partitionFields(panel.fields, config);
  const sections: ParticleSemanticSection[] = [];

  for (const section of raw.sections) {
    if (section.fields.length === 1 && section.fields[0].children?.length) {
      const moduleField = section.fields[0];
      const summary = buildModuleSummary(section.titleKey, moduleField);
      const otherFields = pruneFields(moduleField.children ?? [], summary.consumedPaths);
      sections.push({
        titleKey: section.titleKey,
        summaryRows: summary.summaryRows,
        otherFields,
      });
      continue;
    }

    sections.push({
      titleKey: section.titleKey,
      summaryRows: [],
      otherFields: section.fields,
    });
  }

  return {
    sections,
    otherFields: raw.otherFields,
    hiddenCount: raw.hiddenCount,
  };
}

function buildModuleSummary(
  titleKey: string,
  moduleField: InspectorField,
): ModuleSummaryResult {
  switch (titleKey) {
    case "diff.optimized.main":
      return buildOrderedModuleSummary(moduleField, MAIN_ORDER, {
        curveKeys: new Set([
          "startdelay",
          "startlifetime",
          "startspeed",
          "startsize",
          "startsizex",
          "startsizey",
          "startsizez",
          "startrotation",
          "startrotationx",
          "startrotationy",
          "startrotationz",
          "gravitymodifier",
        ]),
        gradientKeys: new Set(["startcolor"]),
      });
    case "diff.optimized.emission":
      return buildOrderedModuleSummary(moduleField, EMISSION_ORDER, {
        curveKeys: new Set(["rateovertime", "rateoverdistance"]),
        burstKeys: new Set(["bursts"]),
      });
    case "diff.optimized.shapeModule":
      return buildOrderedModuleSummary(moduleField, SHAPE_ORDER, {});
    default:
      return { summaryRows: [], consumedPaths: new Set<string>() };
  }
}

function buildOrderedModuleSummary(
  moduleField: InspectorField,
  order: string[],
  options: {
    curveKeys?: Set<string>;
    gradientKeys?: Set<string>;
    burstKeys?: Set<string>;
  },
): ModuleSummaryResult {
  const children = moduleField.children ?? [];
  const childMap = buildChildMap(children);
  const summaryRows: ParticleSummaryRow[] = [];
  const consumedPaths = new Set<string>();

  for (const key of order) {
    const field = childMap.get(key);
    if (!field) continue;

    let row: ParticleSummaryRow | null = null;
    if (options.curveKeys?.has(key)) {
      row = buildCurveRow(field);
    } else if (options.gradientKeys?.has(key)) {
      row = buildGradientRow(field);
    } else if (options.burstKeys?.has(key)) {
      row = buildBurstRow(field);
    } else {
      row = buildSimpleRow(field);
    }

    if (!row) continue;
    summaryRows.push(row);
    consumedPaths.add(field.propertyPath);
  }

  return { summaryRows, consumedPaths };
}

function buildChildMap(children: InspectorField[]): Map<string, InspectorField> {
  const map = new Map<string, InspectorField>();
  for (const child of children) {
    const key = normalizeFieldKey(child);
    if (!map.has(key)) {
      map.set(key, child);
    }
  }
  return map;
}

function normalizeFieldKey(fieldOrPath: InspectorField | string): string {
  const raw =
    typeof fieldOrPath === "string"
      ? fieldOrPath
      : lastPathSegment(fieldOrPath.propertyPath);
  return raw
    .replace(/^m_/, "")
    .replace(/\[\d+\]/g, "")
    .replace(/[_\s-]/g, "")
    .toLowerCase();
}

function lastPathSegment(path: string): string {
  const parts = path.split(".");
  return parts[parts.length - 1] ?? path;
}

function pruneFields(
  fields: InspectorField[],
  consumedPaths: Set<string>,
): InspectorField[] {
  const out: InspectorField[] = [];
  for (const field of fields) {
    if (consumedPaths.has(field.propertyPath)) continue;
    if (!field.children?.length) {
      out.push(field);
      continue;
    }

    const children = pruneFields(field.children, consumedPaths);
    if (children.length === 0) {
      out.push(field);
      continue;
    }
    out.push({ ...field, children });
  }
  return out;
}

function buildSimpleRow(field: InspectorField): ParticleSummaryRow | null {
  const before = summarizeSimpleValue(field, "before");
  const after = summarizeSimpleValue(field, "after");
  return buildSummaryRow(field, before, after);
}

function buildCurveRow(field: InspectorField): ParticleSummaryRow | null {
  const before = summarizeCurveValue(field, "before");
  const after = summarizeCurveValue(field, "after");
  return buildSummaryRow(field, before, after);
}

function buildGradientRow(field: InspectorField): ParticleSummaryRow | null {
  const before = summarizeGradientValue(field, "before");
  const after = summarizeGradientValue(field, "after");
  return buildSummaryRow(field, before, after);
}

function buildBurstRow(field: InspectorField): ParticleSummaryRow | null {
  const before = summarizeBurstValue(field, "before");
  const after = summarizeBurstValue(field, "after");
  return buildSummaryRow(field, before, after);
}

function buildSummaryRow(
  field: InspectorField,
  before: ParticleSemanticValue | null,
  after: ParticleSemanticValue | null,
): ParticleSummaryRow | null {
  switch (field.changeKind) {
    case "added":
      if (!after) return null;
      return {
        id: `${field.id}:semantic`,
        label: field.label,
        propertyPath: field.propertyPath,
        changeKind: field.changeKind,
        after,
      };
    case "removed":
      if (!before) return null;
      return {
        id: `${field.id}:semantic`,
        label: field.label,
        propertyPath: field.propertyPath,
        changeKind: field.changeKind,
        before,
      };
    case "modified":
      if (!before && !after) return null;
      return {
        id: `${field.id}:semantic`,
        label: field.label,
        propertyPath: field.propertyPath,
        changeKind: field.changeKind,
        before: before ?? undefined,
        after: after ?? undefined,
      };
    default:
      if (!after && !before) return null;
      return {
        id: `${field.id}:semantic`,
        label: field.label,
        propertyPath: field.propertyPath,
        changeKind: field.changeKind,
        after: after ?? before ?? undefined,
      };
  }
}

function summarizeSimpleValue(
  field: InspectorField,
  side: Side,
): ParticleSemanticValue | null {
  if (!hasSideData(field, side)) return null;

  const vector = summarizeVector(field, side);
  if (vector) {
    return { text: vector };
  }

  const raw = getSideValue(field, side);
  if (raw != null) {
    if (isBoolField(field)) {
      return { text: parseBool(raw) ? BOOL_TRUE : BOOL_FALSE };
    }

    if (parseColor(raw)) {
      return { text: formatColorText(raw), color: raw };
    }

    return { text: raw.trim() || t("diff.particle.none") };
  }

  return null;
}

function summarizeCurveValue(
  field: InspectorField,
  side: Side,
): ParticleSemanticValue | null {
  if (!hasSideData(field, side)) return null;

  const modeValue = parseInteger(getDirectChildValue(field, side, "minmaxstate"));
  const scalar = parseNumber(getDirectChildValue(field, side, "scalar"));
  const minScalar = parseNumber(getDirectChildValue(field, side, "minscalar"));
  const maxCurve = findDirectChild(field, "maxcurve");
  const minCurve = findDirectChild(field, "mincurve");
  const maxKeys = countCurveKeys(maxCurve, side);
  const minKeys = countCurveKeys(minCurve, side);
  const modeKey = resolveCurveModeKey(modeValue, scalar, minScalar, maxKeys, minKeys);

  switch (modeKey) {
    case "diff.particle.mode.constant":
      return {
        text:
          scalar != null
            ? formatNumber(scalar)
            : summarizeSimpleValue(field, side)?.text ?? t("diff.particle.none"),
      };
    case "diff.particle.mode.twoConstants":
      return {
        text: `${t(modeKey)} · ${formatRange(minScalar, scalar)}`,
      };
    case "diff.particle.mode.curve":
      return {
        text: `${t(modeKey)} · ${formatKeyCount(maxKeys)}${formatMultiplier(scalar)}`,
      };
    case "diff.particle.mode.twoCurves":
      return {
        text: `${t(modeKey)} · ${formatDualKeyCount(minKeys, maxKeys)}${formatDualMultiplier(minScalar, scalar)}`,
      };
    default:
      return {
        text: `${t("diff.particle.mode.unknown")} · ${summarizeSimpleValue(field, side)?.text ?? t("diff.particle.none")}`,
      };
  }
}

function summarizeGradientValue(
  field: InspectorField,
  side: Side,
): ParticleSemanticValue | null {
  if (!hasSideData(field, side)) return null;

  const modeValue = parseInteger(getDirectChildValue(field, side, "minmaxstate"));
  const maxColorField = findDirectChild(field, "maxcolor");
  const minColorField = findDirectChild(field, "mincolor");
  const maxGradientField = findDirectChild(field, "maxgradient");
  const minGradientField = findDirectChild(field, "mingradient");
  const maxColor = maxColorField ? getSideValue(maxColorField, side) : undefined;
  const minColor = minColorField ? getSideValue(minColorField, side) : undefined;
  const maxStops = maxGradientField ? parseGradientStops(maxGradientField, side) : [];
  const minStops = minGradientField ? parseGradientStops(minGradientField, side) : [];
  const maxColorKeys = maxGradientField
    ? parseInteger(getDirectChildValue(maxGradientField, side, "numcolorkeys")) ?? maxStops.length
    : 0;
  const maxAlphaKeys = maxGradientField
    ? parseInteger(getDirectChildValue(maxGradientField, side, "numalphakeys")) ?? 0
    : 0;
  const minColorKeys = minGradientField
    ? parseInteger(getDirectChildValue(minGradientField, side, "numcolorkeys")) ?? minStops.length
    : 0;
  const minAlphaKeys = minGradientField
    ? parseInteger(getDirectChildValue(minGradientField, side, "numalphakeys")) ?? 0
    : 0;
  const modeKey = resolveGradientModeKey(
    modeValue,
    maxColor,
    minColor,
    maxStops.length,
    minStops.length,
  );

  switch (modeKey) {
    case "diff.particle.mode.color":
      if (!maxColor) return null;
      return {
        text: formatColorText(maxColor),
        color: maxColor,
      };
    case "diff.particle.mode.twoColors":
      return {
        text: `${t(modeKey)} · ${t("diff.particle.colorPair")}`,
        gradientStops: buildColorPairStops(minColor, maxColor),
      };
    case "diff.particle.mode.gradient":
      return {
        text: `${t(modeKey)} · ${t("diff.particle.gradientSummary", maxColorKeys, maxAlphaKeys)}`,
        gradientStops: maxStops,
      };
    case "diff.particle.mode.twoGradients":
      return {
        text: `${t(modeKey)} · ${t("diff.particle.dualGradientSummary", minColorKeys, minAlphaKeys, maxColorKeys, maxAlphaKeys)}`,
        gradientStops: maxStops.length > 0 ? maxStops : minStops,
      };
    case "diff.particle.mode.randomColor":
      return {
        text: `${t(modeKey)} · ${t("diff.particle.gradientSummary", maxColorKeys, maxAlphaKeys)}`,
        gradientStops: maxStops,
      };
    default:
      if (maxColor && parseColor(maxColor)) {
        return {
          text: formatColorText(maxColor),
          color: maxColor,
        };
      }
      if (maxStops.length > 0) {
        return {
          text: `${t("diff.particle.mode.gradient")} · ${t("diff.particle.gradientSummary", maxColorKeys, maxAlphaKeys)}`,
          gradientStops: maxStops,
        };
      }
      return null;
  }
}

function summarizeBurstValue(
  field: InspectorField,
  side: Side,
): ParticleSemanticValue | null {
  if (!hasSideData(field, side)) return null;

  const bursts = collectBurstEntries(field, side);
  if (bursts.length === 0) return null;

  const preview = bursts
    .slice(0, 2)
    .map((burst) => {
      const parts = [t("diff.particle.burstAt", formatNumber(burst.time))];
      parts.push(formatRange(burst.minCount, burst.maxCount));
      if (burst.cycleCount != null && burst.cycleCount > 1) {
        parts.push(`×${formatNumber(burst.cycleCount)}`);
      }
      if (burst.repeatInterval != null && burst.repeatInterval > 0) {
        parts.push(t("diff.particle.burstRepeat", formatNumber(burst.repeatInterval)));
      }
      return parts.join(" ");
    })
    .join(" · ");

  const more =
    bursts.length > 2 ? ` +${bursts.length - 2}` : "";

  return {
    text: `${t("diff.particle.burstSummary", bursts.length)} · ${preview}${more}`,
  };
}

function resolveCurveModeKey(
  modeValue: number | null,
  scalar: number | null,
  minScalar: number | null,
  maxKeys: number,
  minKeys: number,
): string {
  if (modeValue != null && CURVE_MODE_LABELS[modeValue]) {
    return CURVE_MODE_LABELS[modeValue];
  }
  if (minKeys > 0 && maxKeys > 0) return "diff.particle.mode.twoCurves";
  if (maxKeys > 0) return "diff.particle.mode.curve";
  if (minScalar != null && scalar != null && minScalar !== scalar) {
    return "diff.particle.mode.twoConstants";
  }
  return "diff.particle.mode.constant";
}

function resolveGradientModeKey(
  modeValue: number | null,
  maxColor?: string,
  minColor?: string,
  maxStops = 0,
  minStops = 0,
): string {
  if (modeValue != null && GRADIENT_MODE_LABELS[modeValue]) {
    return GRADIENT_MODE_LABELS[modeValue];
  }
  if (minStops > 0 && maxStops > 0) return "diff.particle.mode.twoGradients";
  if (maxStops > 0) return "diff.particle.mode.gradient";
  if (minColor && maxColor) return "diff.particle.mode.twoColors";
  if (maxColor) return "diff.particle.mode.color";
  return "diff.particle.mode.unknown";
}

function buildColorPairStops(
  minColor?: string,
  maxColor?: string,
): ParticleGradientStop[] | undefined {
  if (!minColor && !maxColor) return undefined;
  return [
    { offset: 0, color: minColor ?? maxColor ?? "{r: 1, g: 1, b: 1, a: 1}" },
    { offset: 1, color: maxColor ?? minColor ?? "{r: 1, g: 1, b: 1, a: 1}" },
  ];
}

function collectBurstEntries(field: InspectorField, side: Side) {
  const entries: {
    time: number;
    minCount: number | null;
    maxCount: number | null;
    cycleCount: number | null;
    repeatInterval: number | null;
  }[] = [];

  for (const child of field.children ?? []) {
    if (!hasSideData(child, side)) continue;
    const time = parseNumber(findDescendantValue(child, side, "time")) ?? 0;
    const minCount = parseNumber(findDescendantValue(child, side, "mincount"));
    const maxCount = parseNumber(findDescendantValue(child, side, "maxcount"));
    const cycleCount = parseNumber(findDescendantValue(child, side, "cyclecount"));
    const repeatInterval = parseNumber(findDescendantValue(child, side, "repeatinterval"));
    entries.push({ time, minCount, maxCount, cycleCount, repeatInterval });
  }

  return entries.sort((a, b) => a.time - b.time);
}

function parseGradientStops(
  field: InspectorField,
  side: Side,
): ParticleGradientStop[] {
  const colorCount =
    parseInteger(getDirectChildValue(field, side, "numcolorkeys")) ?? 0;
  const times = collectIndexedValues(field, side, "ctime")
    .slice(0, colorCount || undefined)
    .map((value) => normalizeGradientTime(parseNumber(value) ?? 0));
  const colors = collectIndexedValues(field, side, "key")
    .slice(0, colorCount || undefined);

  const count = Math.min(times.length, colors.length);
  if (count === 0) return [];

  const stops: ParticleGradientStop[] = [];
  for (let index = 0; index < count; index += 1) {
    stops.push({
      offset: clamp01(times[index] ?? 0),
      color: colors[index],
    });
  }
  return stops.sort((a, b) => a.offset - b.offset);
}

function collectIndexedValues(
  field: InspectorField,
  side: Side,
  key: string,
): string[] {
  const matches: { index: number; value: string }[] = [];
  walkFields(field, (node) => {
    if (normalizeFieldKey(node) !== key) return;
    const value = getSideValue(node, side);
    if (value == null) return;
    matches.push({
      index: extractIndex(node.propertyPath),
      value,
    });
  });
  matches.sort((a, b) => a.index - b.index);
  return matches.map((item) => item.value);
}

function extractIndex(path: string): number {
  const match = path.match(/\[(\d+)\](?!.*\[\d+\])/);
  return match ? Number.parseInt(match[1] ?? "0", 10) : Number.MAX_SAFE_INTEGER;
}

function countCurveKeys(field: InspectorField | undefined, side: Side): number {
  if (!field) return 0;
  let count = 0;
  walkFields(field, (node) => {
    if (normalizeFieldKey(node) !== "time") return;
    if (getSideValue(node, side) != null) count += 1;
  });
  return count;
}

function findDirectChild(
  field: InspectorField,
  ...keys: string[]
): InspectorField | undefined {
  const normalized = new Set(keys.map(normalizeFieldKey));
  return field.children?.find((child) => normalized.has(normalizeFieldKey(child)));
}

function getDirectChildValue(
  field: InspectorField,
  side: Side,
  key: string,
): string | undefined {
  return findDirectChild(field, key)
    ? getSideValue(findDirectChild(field, key)!, side)
    : undefined;
}

function findDescendantValue(
  field: InspectorField,
  side: Side,
  key: string,
): string | undefined {
  let found: string | undefined;
  walkFields(field, (node) => {
    if (found != null) return;
    if (normalizeFieldKey(node) !== key) return;
    const value = getSideValue(node, side);
    if (value != null) found = value;
  });
  return found;
}

function walkFields(
  field: InspectorField,
  visitor: (field: InspectorField) => void,
) {
  for (const child of field.children ?? []) {
    visitor(child);
    walkFields(child, visitor);
  }
}

function summarizeVector(
  field: InspectorField,
  side: Side,
): string | null {
  const children = field.children ?? [];
  if (children.length < 2 || children.length > 4) return null;
  const allowed = new Set(["x", "y", "z", "w", "r", "g", "b", "a"]);
  for (const child of children) {
    if (child.children?.length) return null;
    if (!allowed.has(normalizeFieldKey(child))) return null;
  }

  const parts = children
    .map((child) => {
      const value = getSideValue(child, side);
      if (value == null) return null;
      return `${child.label.toUpperCase()} ${value.trim()}`;
    })
    .filter((part): part is string => Boolean(part));

  return parts.length > 0 ? parts.join(" · ") : null;
}

function hasSideData(field: InspectorField, side: Side): boolean {
  if (getSideValue(field, side) != null) return true;
  return (field.children ?? []).some((child) => hasSideData(child, side));
}

function getSideValue(field: InspectorField, side: Side): string | undefined {
  return side === "before" ? field.before : field.after;
}

function isBoolField(field: InspectorField): boolean {
  return field.fieldType === "bool" || field.valueType === "bool";
}

function parseBool(value: string): boolean {
  const trimmed = value.trim().toLowerCase();
  return trimmed === "1" || trimmed === "true";
}

function parseNumber(value?: string): number | null {
  if (value == null) return null;
  const num = Number.parseFloat(value.trim());
  return Number.isFinite(num) ? num : null;
}

function parseInteger(value?: string): number | null {
  if (value == null) return null;
  const num = Number.parseInt(value.trim(), 10);
  return Number.isFinite(num) ? num : null;
}

function parseColor(value?: string): {
  r: number;
  g: number;
  b: number;
  a: number;
} | null {
  if (!value) return null;
  const match = COLOR_RE.exec(value.trim());
  if (!match) return null;
  return {
    r: Number.parseFloat(match[1] ?? "0"),
    g: Number.parseFloat(match[2] ?? "0"),
    b: Number.parseFloat(match[3] ?? "0"),
    a: Number.parseFloat(match[4] ?? "0"),
  };
}

function formatColorText(value: string): string {
  const color = parseColor(value);
  if (!color) return value.trim();
  return `RGBA ${formatNumber(color.r)}, ${formatNumber(color.g)}, ${formatNumber(color.b)}, ${formatNumber(color.a)}`;
}

function formatNumber(value: number | null | undefined): string {
  if (value == null || !Number.isFinite(value)) return t("diff.particle.none");
  if (Number.isInteger(value)) return value.toString();
  return value.toFixed(3).replace(/0+$/, "").replace(/\.$/, "");
}

function formatRange(
  minValue: number | null | undefined,
  maxValue: number | null | undefined,
): string {
  if (minValue == null && maxValue == null) return t("diff.particle.none");
  if (minValue == null) return formatNumber(maxValue);
  if (maxValue == null) return formatNumber(minValue);
  if (minValue === maxValue) return formatNumber(maxValue);
  return `${formatNumber(minValue)} .. ${formatNumber(maxValue)}`;
}

function formatKeyCount(count: number): string {
  return t("diff.particle.keyCount", count);
}

function formatDualKeyCount(minCount: number, maxCount: number): string {
  if (minCount <= 0 && maxCount <= 0) return t("diff.particle.none");
  if (minCount <= 0 || minCount === maxCount) return formatKeyCount(maxCount);
  return `${formatKeyCount(minCount)} / ${formatKeyCount(maxCount)}`;
}

function formatMultiplier(value: number | null): string {
  if (value == null || value === 1) return "";
  return ` × ${formatNumber(value)}`;
}

function formatDualMultiplier(
  minValue: number | null,
  maxValue: number | null,
): string {
  if (minValue == null && maxValue == null) return "";
  if (minValue == null || maxValue == null || minValue === maxValue) {
    return formatMultiplier(maxValue ?? minValue);
  }
  return ` × ${formatNumber(minValue)} / ${formatNumber(maxValue)}`;
}

function normalizeGradientTime(value: number): number {
  if (value > 1) {
    return value / 65535;
  }
  return value;
}

function clamp01(value: number): number {
  if (value < 0) return 0;
  if (value > 1) return 1;
  return value;
}
