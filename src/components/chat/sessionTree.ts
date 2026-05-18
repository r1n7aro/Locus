import type { SessionSummary } from "../../types";

export type SessionTreeStatus =
  | "running"
  | "queued"
  | "starting"
  | "waiting_input"
  | "finishing"
  | "cancelling"
  | "error";

export interface SessionTreeFolderNode {
  kind: "folder";
  key: string;
  label: string;
  updatedAt: number;
  status: SessionTreeStatus | null;
  children: SessionTreeNode[];
  sourceSessionId: string | null;
  session?: SessionSummary;
}

export interface SessionTreeSessionNode {
  kind: "session";
  key: string;
  sessionId: string | null;
  sourceSessionId: string | null;
  title: string;
  rawTitle: string;
  sessionType: string;
  parentSessionId: string | null;
  agentId?: string | null;
  updatedAt: number;
  status: SessionTreeStatus | null;
  children: SessionTreeNode[];
  isVirtual: boolean;
  selectable: boolean;
  session?: SessionSummary;
}

export type SessionTreeNode = SessionTreeFolderNode | SessionTreeSessionNode;

function normalizeSessionTitle(title: string, sessionType: string, isChild: boolean): string {
  const trimmed = title.trim();
  if (!trimmed) return "";
  if (isChild && trimmed.startsWith("sub:")) {
    return trimmed.slice(4).trim();
  }
  if (sessionType === "docgen" || sessionType === "knowledge") {
    if (trimmed.startsWith("Knowledge:")) {
      return trimmed.slice("Knowledge:".length).trim();
    }
    if (trimmed.startsWith("Doc:")) {
      return trimmed.slice(4).trim();
    }
    if (trimmed.startsWith("Wiki:")) {
      return trimmed.slice("Wiki:".length).trim();
    }
  }
  return trimmed;
}

function statusPriority(status: SessionTreeStatus | null): number {
  switch (status) {
    case "running":
    case "waiting_input":
    case "finishing":
      return 5;
    case "cancelling":
      return 4;
    case "starting":
      return 3;
    case "queued":
      return 2;
    case "error":
      return 1;
    default:
      return 0;
  }
}

function maxStatus(a: SessionTreeStatus | null, b: SessionTreeStatus | null): SessionTreeStatus | null {
  return statusPriority(a) >= statusPriority(b) ? a : b;
}

function sortNodes(nodes: SessionTreeNode[], isChildren = false): SessionTreeNode[] {
  if (isChildren) {
    // Children: sort by key ascending for stable order (no reordering on updates)
    nodes.sort((a, b) => a.key.localeCompare(b.key));
  } else {
    // Root: sort by most recent first
    nodes.sort((a, b) => {
      if (b.updatedAt !== a.updatedAt) return b.updatedAt - a.updatedAt;
      if (a.kind !== b.kind) return a.kind === "folder" ? -1 : 1;
      const aLabel = a.kind === "folder" ? a.label : a.rawTitle;
      const bLabel = b.kind === "folder" ? b.label : b.rawTitle;
      return aLabel.localeCompare(bLabel);
    });
  }
  for (const node of nodes) {
    if (node.children.length > 0) sortNodes(node.children, true);
  }
  return nodes;
}

function createActualNode(
  session: SessionSummary,
  streamingSessionIds: Set<string>,
): SessionTreeNode {
  if (session.sessionType === "folder") {
    return {
      kind: "folder",
      key: `session:${session.id}`,
      label: session.title.trim(),
      updatedAt: session.updatedAt,
      status: null,
      children: [],
      sourceSessionId: session.id,
      session,
    };
  }

  const runtimeStatus = session.runtimeStatus ?? null;
  const status = streamingSessionIds.has(session.id) ? "running" : runtimeStatus;
  const updatedAt = session.updatedAt;
  const parentSessionId = session.parentSessionId ?? null;
  return {
    kind: "session",
    key: `session:${session.id}`,
    sessionId: session.id,
    sourceSessionId: session.id,
    title: normalizeSessionTitle(session.title, session.sessionType, !!parentSessionId),
    rawTitle: session.title,
    sessionType: session.sessionType,
    parentSessionId,
    agentId: session.agentId ?? null,
    updatedAt,
    status,
    children: [],
    isVirtual: false,
    selectable: true,
    session,
  };
}

function foldFolderMetadata(node: SessionTreeNode): SessionTreeStatus | null {
  if (node.kind === "session" && node.children.length === 0) {
    return node.status;
  }

  let status: SessionTreeStatus | null = node.kind === "session" ? node.status : null;
  let updatedAt = node.updatedAt;
  for (const child of node.children) {
    const childStatus = foldFolderMetadata(child);
    status = maxStatus(status, childStatus);
    updatedAt = Math.max(updatedAt, child.updatedAt);
  }
  node.status = status;
  node.updatedAt = updatedAt;
  return status;
}

export function buildSessionTree(options: {
  sessions: SessionSummary[];
  streamingSessionIds?: Set<string>;
}): SessionTreeNode[] {
  const streamingSessionIds = options.streamingSessionIds ?? new Set<string>();
  const actualNodeById = new Map<string, SessionTreeNode>();
  for (const session of options.sessions) {
    actualNodeById.set(
      session.id,
      createActualNode(session, streamingSessionIds),
    );
  }

  const roots: SessionTreeNode[] = [];
  for (const session of options.sessions) {
    const node = actualNodeById.get(session.id)!;
    const parentId = session.parentSessionId ?? null;
    const parent = parentId ? actualNodeById.get(parentId) : null;
    if (parent) {
      parent.children.push(node);
    } else {
      roots.push(node);
    }
  }

  for (const node of roots) {
    foldFolderMetadata(node);
  }
  sortNodes(roots);
  return roots;
}

export function nodeContainsSession(node: SessionTreeNode, sessionId: string | null): boolean {
  if (!sessionId) return false;
  if (node.kind === "session" && node.sessionId === sessionId) {
    return true;
  }
  if (node.kind === "folder" && node.sourceSessionId === sessionId) {
    return true;
  }
  return node.children.some((child) => nodeContainsSession(child, sessionId));
}

export function nodeHasActiveDescendant(node: SessionTreeNode): boolean {
  if (node.kind === "session" && (
    node.status === "running" ||
    node.status === "finishing" ||
    node.status === "starting" ||
    node.status === "queued"
  )) {
    return true;
  }
  if (node.kind === "folder" && (
    node.status === "running" ||
    node.status === "finishing" ||
    node.status === "starting" ||
    node.status === "queued"
  )) {
    return true;
  }
  return node.children.some((child) => nodeHasActiveDescendant(child));
}
