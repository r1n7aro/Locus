import { shallowRef } from "vue";
import { ipcInvoke } from "../services/ipc";
import type {
  ProjectContextDescriptor,
  WorkspaceCheckoutDescriptor,
  WorkspaceRef,
} from "../services/project";
import type { ModelDefaults, SessionSummary } from "../types";

export interface ProjectEntry {
  projectId: string;
  name: string;
  /** Workspace of the project's entry checkout when it is open on the
   * desktop; new sessions need a live workspace. */
  workspaceRef: WorkspaceRef | null;
  checkouts: WorkspaceCheckoutDescriptor[];
  sessions: SessionSummary[];
  error: string | null;
}

export function listProjectContexts(): Promise<ProjectContextDescriptor[]> {
  return ipcInvoke<ProjectContextDescriptor[]>("list_project_contexts");
}

export function listProjectSessions(projectId: string): Promise<SessionSummary[]> {
  return ipcInvoke<SessionSummary[]>("list_project_sessions", { projectId });
}

export function getModelDefaults(): Promise<ModelDefaults> {
  return ipcInvoke<ModelDefaults>("get_model_defaults");
}

function baseName(path: string): string {
  const parts = path.replace(/[\\/]+$/, "").split(/[\\/]/);
  return parts[parts.length - 1] || path;
}

function isManaged(checkout: WorkspaceCheckoutDescriptor): boolean {
  return checkout.managedWorktree ?? checkout.runtime?.managedWorktree ?? false;
}

/** Same rule as the desktop picker: projects are entered through their
 * source checkout; managed worktrees only stand in when nothing else exists. */
function entryCheckout(project: ProjectContextDescriptor): WorkspaceCheckoutDescriptor | null {
  const sources = project.checkouts.filter((checkout) => !isManaged(checkout));
  return sources.find((checkout) => checkout.available !== false && checkout.runtime)
    ?? sources.find((checkout) => checkout.available !== false)
    ?? sources[0]
    ?? project.checkouts[0]
    ?? null;
}

export function checkoutWorkspaceRef(checkout: WorkspaceCheckoutDescriptor): WorkspaceRef {
  return {
    checkoutId: checkout.checkoutId,
    expectedGeneration: checkout.runtime?.workspaceGeneration ?? null,
    expectedMaterializationEpoch: checkout.runtime?.materializationEpoch ?? null,
  };
}

/** The workspace whose events carry this session: its execution target or
 * default checkout, pinned to that checkout's live runtime when there is one. */
export function sessionWorkspaceRef(project: ProjectEntry, session: SessionSummary): WorkspaceRef | null {
  const checkoutId = session.executionTarget?.checkoutId
    ?? session.defaultCheckoutId
    ?? project.workspaceRef?.checkoutId
    ?? null;
  if (!checkoutId) return null;
  const checkout = project.checkouts.find((candidate) => candidate.checkoutId === checkoutId);
  if (checkout) return checkoutWorkspaceRef(checkout);
  return {
    checkoutId,
    expectedGeneration: null,
    expectedMaterializationEpoch: session.defaultMaterializationEpoch ?? null,
  };
}

/** Top-level conversations only; sub-agent sessions open from their parent. */
function visibleSessions(sessions: SessionSummary[]): SessionSummary[] {
  return sessions
    .filter((session) => !session.parentSessionId)
    .sort((a, b) => b.updatedAt - a.updatedAt);
}

/** Checkouts by id, from the last project list: references in conversations
 * often carry absolute paths, and the session status row needs the root and
 * detected services. Reactive, so a reload updates open sessions. */
const checkouts = shallowRef<ReadonlyMap<string, WorkspaceCheckoutDescriptor>>(new Map());

export function checkoutRoot(checkoutId: string | null | undefined): string | null {
  return checkoutId ? checkouts.value.get(checkoutId)?.root ?? null : null;
}

export function checkoutDescriptor(checkoutId: string | null | undefined): WorkspaceCheckoutDescriptor | null {
  return checkoutId ? checkouts.value.get(checkoutId) ?? null : null;
}

/** Workspaces of a project's checkouts that are open on the desktop. Session
 * events are scoped to the checkout a session runs in, which may be a
 * worktree rather than the project's entry checkout. */
export function liveCheckoutRefs(project: ProjectEntry): WorkspaceRef[] {
  return project.checkouts.filter((checkout) => checkout.runtime).map(checkoutWorkspaceRef);
}

/** Listed sessions by id, for opening one that is not on screen. */
const knownSessions = new Map<string, { session: SessionSummary; project: ProjectEntry }>();
/** Bumped whenever a project's sessions were listed again. */
export const listedSessionsRevision = shallowRef(0);

function rememberSessions(entry: ProjectEntry): ProjectEntry {
  for (const [id, known] of knownSessions) {
    if (known.project.projectId === entry.projectId) knownSessions.delete(id);
  }
  for (const session of entry.sessions) knownSessions.set(session.id, { session, project: entry });
  listedSessionsRevision.value += 1;
  return entry;
}

export function findListedSession(sessionId: string): {
  session: SessionSummary;
  workspaceRef: WorkspaceRef | null;
  projectName: string;
} | null {
  const known = knownSessions.get(sessionId);
  return known
    ? { session: known.session, workspaceRef: sessionWorkspaceRef(known.project, known.session), projectName: known.project.name }
    : null;
}

export async function loadProjects(): Promise<ProjectEntry[]> {
  const contexts = await listProjectContexts();
  const next = new Map(checkouts.value);
  for (const project of contexts) {
    for (const checkout of project.checkouts) next.set(checkout.checkoutId, checkout);
  }
  checkouts.value = next;
  const entries = await Promise.all(contexts.map(async (project): Promise<ProjectEntry> => {
    const entry = entryCheckout(project);
    const base: ProjectEntry = {
      projectId: project.projectId,
      name: entry ? baseName(entry.root) : project.projectId,
      workspaceRef: entry?.runtime ? checkoutWorkspaceRef(entry) : null,
      checkouts: project.checkouts,
      sessions: [],
      error: null,
    };
    try {
      return rememberSessions({ ...base, sessions: visibleSessions(await listProjectSessions(project.projectId)) });
    } catch (error) {
      return { ...base, error: error instanceof Error ? error.message : String(error) };
    }
  }));
  // Projects with recent activity first.
  return entries.sort((a, b) => (b.sessions[0]?.updatedAt ?? 0) - (a.sessions[0]?.updatedAt ?? 0));
}

export async function reloadProject(entry: ProjectEntry): Promise<ProjectEntry> {
  try {
    return rememberSessions({ ...entry, sessions: visibleSessions(await listProjectSessions(entry.projectId)), error: null });
  } catch (error) {
    return { ...entry, error: error instanceof Error ? error.message : String(error) };
  }
}
