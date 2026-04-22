export interface OnboardingGitProbeState {
  available: boolean;
  isRepo: boolean;
}

export type OnboardingVcsStepState =
  | "loading"
  | "needProject"
  | "detected"
  | "notRepo"
  | "gitMissing";

export function resolveOnboardingGitInitTargetPath(projectPath: string, projectValid: boolean): string {
  const normalized = projectPath.trim();
  return projectValid && normalized ? normalized : "";
}

export function resolveOnboardingVcsStepState(
  targetPath: string,
  probe: OnboardingGitProbeState | null,
): OnboardingVcsStepState {
  if (!probe) return "loading";
  if (!targetPath) return "needProject";
  if (probe.isRepo) return "detected";
  return probe.available ? "notRepo" : "gitMissing";
}

export function canInitOnboardingGit(
  targetPath: string,
  probe: OnboardingGitProbeState | null,
): boolean {
  return !!targetPath && !!probe?.available && !probe.isRepo;
}
