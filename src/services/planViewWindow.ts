import { emit } from "@tauri-apps/api/event";
import { buildSubWindowUrl, openSubWindow } from "./subWindow";
import { hasTauriWindowRuntime } from "./tauriRuntime";

export const PLAN_VIEW_WINDOW_LABEL = "plan-view";
export const PLAN_VIEW_WINDOW_PATH = "/plan-view";
export const PLAN_VIEW_WINDOW_EVENT = "plan-view:payload";
/** Broadcast by the window that owns the approval flow once the pending
 *  planApproval confirm is answered (from either surface) or disappears. */
export const PLAN_VIEW_RESOLVED_EVENT = "plan-view:approval-resolved";
export const PLAN_VIEW_WINDOW_FLAG = "planView";
export const PLAN_VIEW_WINDOW_TITLE = "Locus Plan Review";

export interface PlanViewWindowPayload {
  planFilePath: string;
  /** Present while the plan awaits approval: enables the approve / send-back
   *  actions inside the window (answered via the global answer_question). */
  questionId?: string;
}

function trimOrEmpty(value: string | null | undefined): string {
  return value?.trim() || "";
}

export function isPlanViewWindowLocation(
  locationLike: Pick<Location, "pathname" | "search"> = window.location,
): boolean {
  return locationLike.pathname === PLAN_VIEW_WINDOW_PATH
    || locationLike.search.includes(`${PLAN_VIEW_WINDOW_FLAG}=1`);
}

export function getPlanViewWindowPayload(
  search = window.location.search,
): PlanViewWindowPayload {
  const params = new URLSearchParams(search);
  return {
    planFilePath: trimOrEmpty(params.get("planFilePath")),
    questionId: trimOrEmpty(params.get("questionId")) || undefined,
  };
}

export function buildPlanViewWindowQuery(payload: PlanViewWindowPayload): string {
  const params = new URLSearchParams({
    [PLAN_VIEW_WINDOW_FLAG]: "1",
    planFilePath: payload.planFilePath,
  });
  if (payload.questionId) {
    params.set("questionId", payload.questionId);
  }
  return params.toString();
}

export function buildPlanViewWindowUrl(payload: PlanViewWindowPayload): string {
  return buildSubWindowUrl(buildPlanViewWindowQuery(payload));
}

export async function openPlanViewWindow(
  payload: PlanViewWindowPayload,
): Promise<boolean> {
  if (!hasTauriWindowRuntime()) return false;
  if (!payload.planFilePath.trim()) return false;

  const result = await openSubWindow({
    kind: PLAN_VIEW_WINDOW_LABEL,
    title: PLAN_VIEW_WINDOW_TITLE,
    width: 920,
    height: 760,
    minWidth: 600,
    minHeight: 420,
    resizable: true,
    maximizable: true,
    minimizable: false,
  }, buildPlanViewWindowQuery(payload));
  if (result.existing) {
    await result.window?.emit(PLAN_VIEW_WINDOW_EVENT, payload);
  }
  return true;
}

/** Tell an open plan review window that the pending approval is settled.
 *  Broadcast globally: the plan window may be pool-claimed under a pool
 *  label, so it cannot be found by a fixed label anymore. */
export async function broadcastPlanApprovalResolved(questionId: string): Promise<void> {
  if (!hasTauriWindowRuntime()) return;
  await emit(PLAN_VIEW_RESOLVED_EVENT, { questionId });
}
