import { describe, expect, it } from "vitest";
import {
  UNITY_PROJECT_ICON,
  projectHasCapability,
  projectHasService,
  projectIconForServices,
} from "../components/icons/projectIcons";

describe("project service presentation", () => {
  it("enables the Unity mark and only the capabilities registered by the project service", () => {
    const services = ["unity"];

    expect(projectIconForServices(services)).toBe(UNITY_PROJECT_ICON);
    expect(projectHasService(services, "unity")).toBe(true);
    expect(projectHasCapability(services, "assetDatabase")).toBe(true);
    expect(projectHasCapability(services, "editorConnection")).toBe(true);
    expect(projectHasCapability(services, "codeAnalysis")).toBe(true);
    expect(projectHasCapability(services, "hotReload")).toBe(true);
  });

  it("keeps unregistered project services free of engine-specific UI", () => {
    const services = ["custom"];

    expect(projectIconForServices(services)).toBeNull();
    expect(projectHasService(services, "unity")).toBe(false);
    expect(projectHasCapability(services, "editorConnection")).toBe(false);
  });

  it("normalizes detected service identifiers", () => {
    expect(projectIconForServices([" Unity "])).toBe(UNITY_PROJECT_ICON);
    expect(projectHasService(["UNITY"], "unity")).toBe(true);
  });
});
