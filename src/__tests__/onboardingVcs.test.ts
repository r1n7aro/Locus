import { describe, expect, it } from "vitest";
import {
  canInitOnboardingGit,
  resolveOnboardingGitInitTargetPath,
  resolveOnboardingVcsStepState,
} from "../components/onboarding/onboardingVcs";

describe("onboarding vcs step state", () => {
  it("uses the validated Unity project path as the git init target", () => {
    expect(resolveOnboardingGitInitTargetPath("F:/UnityProject", true)).toBe("F:/UnityProject");
    expect(resolveOnboardingGitInitTargetPath("F:/UnityProject", false)).toBe("");
    expect(resolveOnboardingGitInitTargetPath("   ", true)).toBe("");
  });

  it("requires a Unity project before offering repository initialization", () => {
    const probe = { available: true, isRepo: false };
    expect(resolveOnboardingVcsStepState("", probe)).toBe("needProject");
    expect(canInitOnboardingGit("", probe)).toBe(false);
  });

  it("shows the correct repository detection state for the init target path", () => {
    expect(resolveOnboardingVcsStepState("F:/UnityProject", { available: true, isRepo: true })).toBe("detected");
    expect(resolveOnboardingVcsStepState("F:/UnityProject", { available: true, isRepo: false })).toBe("notRepo");
    expect(resolveOnboardingVcsStepState("F:/UnityProject", { available: false, isRepo: false })).toBe("gitMissing");
  });

  it("only enables init when Git is available and the target path is not already a repository", () => {
    expect(canInitOnboardingGit("F:/UnityProject", { available: true, isRepo: false })).toBe(true);
    expect(canInitOnboardingGit("F:/UnityProject", { available: true, isRepo: true })).toBe(false);
    expect(canInitOnboardingGit("F:/UnityProject", { available: false, isRepo: false })).toBe(false);
    expect(canInitOnboardingGit("F:/UnityProject", null)).toBe(false);
  });
});
