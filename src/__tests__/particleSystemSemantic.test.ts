import { describe, expect, it } from "vitest";
import type { InspectorField, InspectorPanel } from "../types";
import { getRendererConfig } from "../components/diff/rendererRegistry";
import { buildParticleSystemSemanticView } from "../components/diff/particleSystemSemantic";

function lastSegment(path: string): string {
  const parts = path.split(".");
  return parts[parts.length - 1] ?? path;
}

function prettify(segment: string): string {
  return segment
    .replace(/^m_/, "")
    .replace(/\[(\d+)\]/g, " $1")
    .replace(/([a-z])([A-Z])/g, "$1 $2")
    .replace(/^./, (char) => char.toUpperCase());
}

function makeField(
  propertyPath: string,
  overrides: Partial<InspectorField> = {},
): InspectorField {
  return {
    id: propertyPath,
    label: prettify(lastSegment(propertyPath)),
    propertyPath,
    valueType: overrides.children?.length ? "group" : "string",
    changeKind: overrides.changeKind ?? "unchanged",
    before: overrides.before,
    after: overrides.after,
    children: overrides.children ?? [],
    fieldType: overrides.fieldType,
    reference: overrides.reference,
  };
}

function makeBuiltinPanel(fields: InspectorField[]): InspectorPanel {
  return {
    panelKind: "component",
    title: "ParticleSystem",
    changeKind: "modified",
    added: false,
    removed: false,
    componentType: "ParticleSystem",
    componentSource: "builtin",
    fields,
  };
}

describe("particleSystemSemantic", () => {
  it("summarizes gradients and hides raw key arrays from optimized sections", () => {
    const panel = makeBuiltinPanel([
      makeField("InitialModule", {
        children: [
          makeField("InitialModule.duration", { after: "5" }),
          makeField("InitialModule.startLifetime", {
            children: [
              makeField("InitialModule.startLifetime.minMaxState", { after: "0" }),
              makeField("InitialModule.startLifetime.scalar", { after: "1.5" }),
            ],
          }),
          makeField("InitialModule.startColor", {
            children: [
              makeField("InitialModule.startColor.minMaxState", { after: "1" }),
              makeField("InitialModule.startColor.maxGradient", {
                children: [
                  makeField("InitialModule.startColor.maxGradient.numColorKeys", { after: "2" }),
                  makeField("InitialModule.startColor.maxGradient.numAlphaKeys", { after: "2" }),
                  makeField("InitialModule.startColor.maxGradient.ctime[0]", { after: "0" }),
                  makeField("InitialModule.startColor.maxGradient.ctime[1]", { after: "65535" }),
                  makeField("InitialModule.startColor.maxGradient.key[0]", {
                    after: "{r: 1, g: 0.4, b: 0.2, a: 1}",
                  }),
                  makeField("InitialModule.startColor.maxGradient.key[1]", {
                    after: "{r: 0.2, g: 0.5, b: 1, a: 1}",
                  }),
                ],
              }),
            ],
          }),
          makeField("InitialModule.maxNumParticles", { after: "1000" }),
        ],
      }),
    ]);

    const config = getRendererConfig(panel);
    const view = buildParticleSystemSemanticView(panel, config!);
    const mainSection = view.sections.find(
      (section) => section.titleKey === "diff.optimized.main",
    );

    expect(mainSection).toBeDefined();
    expect(mainSection?.summaryRows.map((row) => row.propertyPath)).toEqual([
      "InitialModule.duration",
      "InitialModule.startLifetime",
      "InitialModule.startColor",
      "InitialModule.maxNumParticles",
    ]);
    expect(mainSection?.summaryRows[2]?.after?.text).toContain("渐变");
    expect(mainSection?.summaryRows[2]?.after?.gradientStops).toHaveLength(2);
    expect(mainSection?.otherFields).toHaveLength(0);
  });

  it("summarizes min-max curve changes as semantic before/after values", () => {
    const panel = makeBuiltinPanel([
      makeField("InitialModule", {
        children: [
          makeField("InitialModule.startLifetime", {
            changeKind: "modified",
            children: [
              makeField("InitialModule.startLifetime.minMaxState", {
                changeKind: "modified",
                before: "0",
                after: "1",
              }),
              makeField("InitialModule.startLifetime.scalar", {
                changeKind: "modified",
                before: "1",
                after: "2",
              }),
              makeField("InitialModule.startLifetime.maxCurve", {
                changeKind: "modified",
                children: [
                  makeField("InitialModule.startLifetime.maxCurve.time[0]", {
                    changeKind: "added",
                    after: "0",
                  }),
                  makeField("InitialModule.startLifetime.maxCurve.time[1]", {
                    changeKind: "added",
                    after: "1",
                  }),
                ],
              }),
            ],
          }),
        ],
      }),
    ]);

    const config = getRendererConfig(panel);
    const view = buildParticleSystemSemanticView(panel, config!);
    const row = view.sections[0]?.summaryRows[0];

    expect(row?.changeKind).toBe("modified");
    expect(row?.before?.text).toBe("1");
    expect(row?.after?.text).toContain("曲线");
    expect(row?.after?.text).toContain("2 键");
  });

  it("summarizes burst arrays into compact emission descriptions", () => {
    const panel = makeBuiltinPanel([
      makeField("EmissionModule", {
        children: [
          makeField("EmissionModule.bursts", {
            children: [
              makeField("EmissionModule.bursts[0]", {
                children: [
                  makeField("EmissionModule.bursts[0].time", { after: "0" }),
                  makeField("EmissionModule.bursts[0].minCount", { after: "6" }),
                  makeField("EmissionModule.bursts[0].maxCount", { after: "6" }),
                ],
              }),
              makeField("EmissionModule.bursts[1]", {
                children: [
                  makeField("EmissionModule.bursts[1].time", { after: "0.35" }),
                  makeField("EmissionModule.bursts[1].minCount", { after: "10" }),
                  makeField("EmissionModule.bursts[1].maxCount", { after: "18" }),
                  makeField("EmissionModule.bursts[1].cycleCount", { after: "2" }),
                  makeField("EmissionModule.bursts[1].repeatInterval", { after: "0.1" }),
                ],
              }),
            ],
          }),
        ],
      }),
    ]);

    const config = getRendererConfig(panel);
    const view = buildParticleSystemSemanticView(panel, config!);
    const emissionSection = view.sections.find(
      (section) => section.titleKey === "diff.optimized.emission",
    );
    const burstRow = emissionSection?.summaryRows[0];

    expect(burstRow?.propertyPath).toBe("EmissionModule.bursts");
    expect(burstRow?.after?.text).toContain("2 个 Burst");
    expect(burstRow?.after?.text).toContain("t=0.35s");
    expect(burstRow?.after?.text).toContain("10 .. 18");
  });
});
