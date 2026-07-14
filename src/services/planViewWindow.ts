import { getCurrentWebviewWindow, WebviewWindow } from "@tauri-apps/api/webviewWindow";
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

export function buildPlanViewWindowUrl(payload: PlanViewWindowPayload): string {
  const params = new URLSearchParams({
    [PLAN_VIEW_WINDOW_FLAG]: "1",
    planFilePath: payload.planFilePath,
  });
  if (payload.questionId) {
    params.set("questionId", payload.questionId);
  }
  return `${PLAN_VIEW_WINDOW_PATH}?${params.toString()}`;
}

export async function openPlanViewWindow(
  payload: PlanViewWindowPayload,
): Promise<boolean> {
  if (!hasTauriWindowRuntime()) return false;
  if (!payload.planFilePath.trim()) return false;

  const existingWindow = await WebviewWindow.getByLabel(PLAN_VIEW_WINDOW_LABEL);
  if (existingWindow) {
    await existingWindow.emit(PLAN_VIEW_WINDOW_EVENT, payload);
    await existingWindow.setFocus();
    return true;
  }

  await new Promise<void>((resolve, reject) => {
    const planWindow = new WebviewWindow(PLAN_VIEW_WINDOW_LABEL, {
      url: buildPlanViewWindowUrl(payload),
      title: PLAN_VIEW_WINDOW_TITLE,
      width: 920,
      height: 760,
      minWidth: 600,
      minHeight: 420,
      decorations: false,
      resizable: true,
      closable: true,
      minimizable: false,
      maximizable: true,
      parent: getCurrentWebviewWindow(),
      center: true,
      shadow: true,
    });

    planWindow.once("tauri://created", () => {
      resolve();
    });
    planWindow.once("tauri://error", (event) => {
      reject(event);
    });
  });

  return true;
}

/** Tell an open plan review window that the pending approval is settled. */
export async function broadcastPlanApprovalResolved(questionId: string): Promise<void> {
  if (!hasTauriWindowRuntime()) return;
  const existingWindow = await WebviewWindow.getByLabel(PLAN_VIEW_WINDOW_LABEL);
  if (!existingWindow) return;
  await existingWindow.emit(PLAN_VIEW_RESOLVED_EVENT, { questionId });
}
