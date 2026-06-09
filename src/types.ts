export type SessionRuntimeStatus =
  | "running"
  | "queued"
  | "starting"
  | "waiting_input"
  | "finishing"
  | "cancelling"
  | "error";

export interface SessionSummary {
  id: string;
  title: string;
  agentId?: string | null;
  sessionType: string;
  parentSessionId?: string | null;
  updatedAt: number;
  runtimeStatus?: SessionRuntimeStatus | null;
}

export type ServerToolKind = "web_search";

export interface ToolCallInfo {
  id: string;
  name: string;
  arguments: string;
  order?: number;
  serverTool?: ServerToolKind;
  serverToolOutput?: string;
  outcome?: ToolCallOutcome;
  recordedOutput?: string;
  nestedToolCalls?: ToolCallInfo[];
}

export type ToolCallOutcome = "done" | "error" | "interrupted";

export interface RenderOrderKey {
  runId: string;
  seq: number;
}

export type AssistantRenderPart =
  | {
      kind: "thinking";
      id: string;
      order: RenderOrderKey;
      content: string;
      active?: boolean;
      duration?: number;
      signature?: string;
    }
  | {
      kind: "text";
      id: string;
      order: RenderOrderKey;
      content: string;
    }
  | {
      kind: "toolCall";
      id: string;
      order: RenderOrderKey;
      toolCall: ToolCallInfo;
    }
  | {
      kind: "knowledgeProposal";
      id: string;
      order: RenderOrderKey;
      message: ChatMessage;
    };

export interface ImageAttachment {
  data: string;
  mimeType: string;
}

export type AssetRefKind = "asset" | "sceneObject" | "knowledge";

export type KnowledgeAccessMode = "disabled" | "read_only" | "full";

export interface AssetRefAttachment {
  path: string;
  kind: AssetRefKind;
  name?: string;
  typeLabel?: string;
  source?: "unity" | "manual";
}

export type UnityEditorProcessState = "running" | "not_running" | "unknown";

export type UnityBackgroundHookState =
  | "disabled"
  | "inactive"
  | "patched"
  | "failed"
  | "unsupported";

export interface UnityBackgroundHookStatus {
  enabled: boolean;
  supported: boolean;
  state: UnityBackgroundHookState;
  patched: boolean;
  processId?: number | null;
  editorProcessPath?: string | null;
  symbolCount: number;
  error?: string | null;
  updatedAtMs: number;
}

export interface UnityConnectionStatus {
  connected: boolean;
  editorStatus: string;
  scenePath?: string | null;
  editorProcessState: UnityEditorProcessState;
  editorProcessId?: number | null;
  editorProcessPath?: string | null;
  editorProjectPath?: string | null;
  processCheckedAtMs?: number | null;
  processLastError?: string | null;
  pipeName: string;
  latencyMs?: number | null;
  reconnectAttempts: number;
  lastError?: string | null;
  backgroundHook: UnityBackgroundHookStatus;
  checkedAtMs: number;
}

export interface SkillIntentItem {
  dirName: string;
  source: string;
  name: string;
}

export interface UserIntentMeta {
  kind: "user_intent_v1";
  mode: "build" | "plan";
  skills: SkillIntentItem[];
  clientMessageId?: string;
}

export interface ChatComposerSendPayload {
  text: string;
  displayText: string;
  images: ImageAttachment[];
  assetRefs: AssetRefAttachment[];
  mode?: string | null;
  userIntent?: UserIntentMeta | null;
}

export type KnowledgeProposalVerify = "none" | "required";
export type KnowledgeProposalStatus =
  | "pending"
  | "applying"
  | "applied"
  | "invalidated"
  | "stale";
export type KnowledgeProposalItemKind = "memory" | "knowledge";
export type KnowledgeProposalItemMode =
  | "replace"
  | "create_source"
  | "update_source";

export interface KnowledgeProposalItem {
  kind: KnowledgeProposalItemKind;
  mode: KnowledgeProposalItemMode;
  target: string;
  draft: string;
}

export interface KnowledgeProposal {
  proposalId: string;
  status: KnowledgeProposalStatus;
  confidence: number;
  verify: KnowledgeProposalVerify;
  estTokens: number;
  items: KnowledgeProposalItem[];
  createdAt: number;
  updatedAt: number;
}

export interface ChatMessage {
  id: string;
  role: "user" | "assistant" | "tool";
  content: string;
  createdAt: number;
  promptPrefix?: string;
  promptSuffix?: string;
  responseId?: string;
  contentOrder?: number;
  thinkingOrder?: number;
  toolCalls?: ToolCallInfo[];
  toolCallId?: string;
  images?: ImageAttachment[];
  assetRefs?: AssetRefAttachment[];
  thinkingContent?: string;
  thinkingDuration?: number;
  thinkingSignature?: string;
  intentMeta?: UserIntentMeta;
  knowledgeProposal?: KnowledgeProposal;
  renderParts?: AssistantRenderPart[];
}

export interface PendingSessionInput {
  id: string;
  sessionId: string;
  runId: string;
  mergeGroupId: string;
  status: "queued" | "delivering" | "accepted" | "restored" | string;
  delivery?: "after_run" | "immediate" | string;
  text: string;
  displayText: string;
  images?: ImageAttachment[];
  assetRefs?: AssetRefAttachment[];
  mode?: string | null;
  userIntent?: UserIntentMeta | null;
  clientMessageId?: string | null;
  messageId?: string | null;
  createdAt: number;
  updatedAt: number;
}

export interface SessionDetail {
  id: string;
  title: string;
  agentId?: string | null;
  sessionType: string;
  parentSessionId: string | null;
  latestCompletedRunId?: string | null;
  createdAt: number;
  updatedAt: number;
  messages: ChatMessage[];
  pendingInputs?: PendingSessionInput[];
  runtime?: SessionRuntimeSnapshot | null;
}

export type SessionRunStatus =
  | "queued"
  | "starting"
  | "running"
  | "waiting_input"
  | "finishing"
  | "cancelling"
  | "done"
  | "cancelled"
  | "error";

export interface SessionRunSummary {
  runId: string;
  sessionId: string;
  status: SessionRunStatus;
  startedAt: number;
  updatedAt: number;
  finishedAt?: number | null;
  errorMessage?: string | null;
}

export interface SessionEventRecord {
  sessionId: string;
  runId: string;
  seq: number;
  eventType: string;
  payload: Record<string, unknown>;
  createdAt: number;
}

export interface SessionRuntimeSnapshot {
  activeRun: SessionRunSummary;
  activeToolCalls: ToolCallDisplay[];
  streamingText?: string;
  streamingThinking?: string;
  liveRenderParts?: AssistantRenderPart[];
  streamSequence?: number;
  streamingTextOrder?: number;
  thinkingOrder?: number;
  isThinking?: boolean;
  thinkingDuration?: number;
  pendingQuestion?: PendingQuestion | null;
  pendingToolConfirms: PendingToolConfirm[];
  isCompacting: boolean;
}

export interface ActiveSessionSelectionChanged {
  workspaceKey: string;
  sessionId: string | null;
}

export interface SessionContentChangedEvent {
  workingDir: string;
  sessionId: string;
  source:
    | "undo_perform"
    | "undo_perform_to_message"
    | "undo_latest_conversation_turn"
    | "rollback_session_to_message"
    | string;
  changedAt: number;
}

export interface SaveRawContextRequest {
  sessionId: string;
  includeSystemPrompt: boolean;
}

export interface AgentInfo {
  id: string;
  name: string;
  description: string;
  isDefault: boolean;
  defaultEffort?: EffortLevel | null;
  modelRecommendation?: ModelRecommendation | null;
  source: string;
}

export type EffortLevel = "none" | "low" | "medium" | "high" | "xhigh" | "max";
export type ThinkingLevel = EffortLevel;
export type ModelRecommendation = "small" | "large";

export interface ModelOption {
  id: string;
  name: string;
  provider:
    | "openrouter"
    | "anthropic"
    | "anthropic_sdk"
    | "openai_codex"
    | "custom";
  defaultEffort?: EffortLevel | null;
  supportedEfforts?: EffortLevel[];
  additionalSpeedTiers?: string[];
  isDefault?: boolean;
}

export type ApiFormat =
  | "openai_chat"
  | "openai_responses"
  | "anthropic_messages";

export type ReasoningParamFormat =
  | "none"
  | "openai_chat_reasoning_effort"
  | "openai_responses_reasoning_effort"
  | "anthropic_thinking";

export interface CustomEndpointServerTools {
  webSearch: boolean;
}

export interface CustomEndpoint {
  id: string;
  name: string;
  apiModel: string;
  endpoint: string;
  apiFormat: ApiFormat;
  apiKey: string;
  contextLength: number;
  betaFlags: string[];
  supportedReasoningEfforts: EffortLevel[];
  reasoningParamFormat: ReasoningParamFormat;
  replayReasoningContent: boolean;
  serverTools: CustomEndpointServerTools;
  supportsToolLazyLoading: boolean;
  supportsVision: boolean;
}

export interface ModelDefaults {
  mainModel: string;
  planModel: string;
  subagentModels: Record<string, string>;
}

export type CodexTransportMode = "http" | "websocket";

export interface CodexModelConfig {
  transport: CodexTransportMode;
}

export interface AuthStatus {
  authenticated: boolean;
  hasApiKey: boolean;
  email: string | null;
}

export interface AppStorageInfo {
  activePath: string;
  defaultPath: string;
  activeSizeBytes: number;
  usesCustomPath: boolean;
  pendingTargetPath?: string | null;
  restartRequired: boolean;
}

export interface AppTempInfo {
  path: string;
  sizeBytes: number;
}

export type PythonRuntimeSource = "managed" | "system";

export interface PythonRuntimeInfo {
  id: string;
  label: string;
  path: string;
  version?: string | null;
  source: PythonRuntimeSource;
  selected: boolean;
  available: boolean;
}

export interface PythonRuntimeState {
  runtimes: PythonRuntimeInfo[];
  selectedId?: string | null;
  effective?: PythonRuntimeInfo | null;
  missingSelected: boolean;
}

export type GitRuntimeSource = "envOverride" | "managed" | "path" | "commonLocation";

export interface GitRuntimeInfo {
  id: string;
  label: string;
  path: string;
  version?: string | null;
  source: GitRuntimeSource;
  selected: boolean;
  available: boolean;
}

export interface GitRuntimeState {
  runtimes: GitRuntimeInfo[];
  selectedId?: string | null;
  effective?: GitRuntimeInfo | null;
  missingSelected: boolean;
}

export type ProxyMode = "auto" | "manual" | "disabled";
export type ProxyEnvironmentEntryKind = "proxy" | "bypass";
export type ProxyRouteSource = "system" | "environment" | "manual" | "direct";

export interface ProxyEnvironmentEntry {
  key: string;
  value: string;
  kind: ProxyEnvironmentEntryKind;
}

export interface SystemProxyConfig {
  platform: string;
  available: boolean;
  source: string;
  enabled?: boolean | null;
  autoDetect?: boolean | null;
  autoConfigUrl?: string | null;
  proxyServer?: string | null;
  proxyOverride?: string | null;
  httpProxy?: string | null;
  httpsProxy?: string | null;
  socksProxy?: string | null;
}

export interface ManualProxyConfig {
  httpProxy: string;
  httpsProxy: string;
  allProxy: string;
  noProxy: string;
}

export interface ProxyConfig {
  mode: ProxyMode;
  manual: ManualProxyConfig;
}

export interface ProxyRoute {
  targetLabel: string;
  targetUrl: string;
  proxyUrl?: string | null;
  source: ProxyRouteSource;
}

export interface ProxyStatus {
  mode: ProxyMode;
  config: ProxyConfig;
  environment: ProxyEnvironmentEntry[];
  /** Backward-compatible alias for older backend status payloads. Prefer environment. */
  manual: ProxyEnvironmentEntry[];
  system: SystemProxyConfig;
  routes: ProxyRoute[];
}

export interface AuthUrlInfo {
  url: string;
}

export interface AppUpdateChangeGroup {
  title: string;
  items: string[];
}

export interface AppUpdateDownloadChannel {
  label: string;
  url: string;
}

export type AppUpdateChannel = "stable" | "experimental";

export interface AppUpdateInstallerDownload {
  id: string;
  label: string;
  url: string;
  platform: string;
  arch: string;
  includesManagedPython: boolean;
  includesManagedGit: boolean;
  requiresSystemPython: boolean;
  requiresSystemGit: boolean;
}

export interface AppUpdateLocaleEntry {
  title: string;
  summary: string;
  changelogUrl: string;
  changes: AppUpdateChangeGroup[];
  downloadChannels?: AppUpdateDownloadChannel[];
}

export interface AppUpdateManifest {
  version: string;
  releasedAt: string;
  channel: string;
  installers?: AppUpdateInstallerDownload[];
  locales: Record<string, AppUpdateLocaleEntry>;
}

export type AppUpdateSourceKind = "local" | "remote";

export interface AppUpdateManifestFetchResult {
  manifest: AppUpdateManifest;
  sourceKind: AppUpdateSourceKind;
  sourceBaseUrl: string;
}

export interface AppUpdateInfo {
  currentVersion: string;
  latestVersion: string;
  releasedAt: string;
  channel: AppUpdateChannel;
  currentChannel: AppUpdateChannel;
  latestChannel: AppUpdateChannel;
  currentIsExperimental: boolean;
  latestIsExperimental: boolean;
  title: string;
  summary: string;
  changelogUrl: string;
  releaseUrl: string;
  downloadUrl: string;
  downloadLabel: string;
  changes: AppUpdateChangeGroup[];
  installer?: AppUpdateInstallerDownload | null;
  sourceKind: AppUpdateSourceKind;
  sourceBaseUrl: string;
}

export interface TokenUsage {
  totalInputTokens: number;
  totalOutputTokens: number;
  totalCacheReadTokens: number;
  totalCacheWriteTokens: number;
  totalCostUsd: number;
  pricedRounds: number;
  contextTokens: number;
  contextLimit: number;
}

// ── Todo ──

export interface TodoItem {
  content: string;
  status: "pending" | "in_progress" | "completed" | "cancelled";
  priority: "high" | "medium" | "low";
}

export interface TodoSnapshot {
  items: TodoItem[];
  latestRunId: string | null;
}

export type TodoPanelMode = "current" | "all";

export interface DuplicateGuidOverview {
  groupCount: number;
  pathCount: number;
  assetsOnlyGroups: number;
  packagesOnlyGroups: number;
  crossRootGroups: number;
}

export type AssetRiskKind =
  | "brokenReferences"
  | "missingScripts"
  | "parseFailures"
  | "duplicateGuids";

export interface AssetRiskEntry {
  kind: AssetRiskKind;
  count: number;
}

export interface ScanStats {
  dirsScanned: number;
  metaFilesFound: number;
  yamlAssetsFound: number;
  nodesAdded: number;
  edgesAdded: number;
  nodesUpdated: number;
  nodesDeleted: number;
  parseFailures: number;
  elapsedMs: number;
  duplicateGuids: DuplicateGuidOverview;
}

export type AssetDbScanEvent =
  | { phase: "dirScan" }
  | { phase: "metaParse"; total: number; completed: number }
  | { phase: "yamlParse"; total: number; completed: number }
  | { phase: "dbWrite" }
  | {
      phase: "reconcile";
      verifyHashes: boolean;
      stage?: "scanning" | "discovering" | "processing" | string | null;
      total?: number | null;
      completed?: number | null;
      queued?: number | null;
      failed?: number | null;
    }
  | { phase: "reconcileDone" }
  | { phase: "done"; stats: ScanStats }
  | { phase: "error"; error: AppErrorPayload };

export interface KnowledgeChangedEvent {
  workingDir: string;
  source: string;
  changedAt: number;
  docType?: "design" | "memory" | "skill" | "reference";
  path?: string | null;
  parentPath?: string | null;
  targetKind?: "document" | "directory" | "type" | "workspace";
  changeKind?: "content" | "structure" | "config";
  subtree?: boolean;
}

/** Every StreamEvent is wrapped in an envelope that carries a runId for filtering stale events. */
export type StreamEvent = { runId: string } & (
  | { type: "runStart"; sessionId: string }
  | { type: "userMessage"; sessionId: string; message: ChatMessage }
  | { type: "pendingInputQueued"; sessionId: string; input: PendingSessionInput }
  | { type: "pendingInputDeleted"; sessionId: string; pendingInputId: string }
  | { type: "pendingInputAccepted"; sessionId: string; pendingInputId: string; messageId: string }
  | { type: "textDelta"; sessionId: string; text: string; order?: number; partId?: string; renderSeq?: number }
  | { type: "thinkingDelta"; sessionId: string; text: string; order?: number; partId?: string; renderSeq?: number }
  | {
      type: "toolCallStart";
      sessionId: string;
      toolCallId: string;
      toolName: string;
      arguments: string;
      order?: number;
      partId?: string;
      renderSeq?: number;
    }
  | {
      type: "toolCallDone";
      sessionId: string;
      toolCallId: string;
      toolName: string;
      output: string;
      outcome: ToolCallOutcome;
      images?: ImageAttachment[];
    }
  | {
      type: "toolCallDelta";
      sessionId: string;
      toolCallId: string;
      delta: string;
    }
  | {
      type: "toolCallProgress";
      sessionId: string;
      toolCallId: string;
      title: string;
      info: string;
      progress?: number | null;
      state: string;
    }
  | {
      type: "subagentToolCallStart";
      sessionId: string;
      parentToolCallId: string;
      toolCallId: string;
      toolName: string;
      arguments: string;
      order?: number;
      partId?: string;
      renderSeq?: number;
    }
  | {
      type: "subagentToolCallDone";
      sessionId: string;
      parentToolCallId: string;
      toolCallId: string;
      toolName: string;
      output: string;
      outcome: ToolCallOutcome;
      images?: ImageAttachment[];
    }
  | {
      type: "toolCallRoundDone";
      sessionId: string;
      messageId: string;
      fullText: string;
      toolCalls: ToolCallInfo[];
      contentOrder?: number;
      thinkingOrder?: number;
      renderParts?: AssistantRenderPart[];
    }
  | { type: "knowledgeProposal"; sessionId: string; message: ChatMessage }
  | {
      type: "usageUpdate";
      sessionId: string;
      inputTokens: number;
      outputTokens: number;
      cacheReadTokens: number;
      cacheWriteTokens: number;
      totalInputTokens: number;
      totalOutputTokens: number;
      totalCacheReadTokens: number;
      totalCacheWriteTokens: number;
      totalCostUsd: number;
      pricedRounds: number;
      contextTokens: number;
      contextLimit: number;
    }
  | {
      type: "askUser";
      sessionId: string;
      questionId: string;
      toolCallId: string;
      question: string;
      options: AskOption[];
    }
  | {
      type: "toolConfirm";
      sessionId: string;
      questionId: string;
      toolCallId: string;
      display: ToolConfirmDisplay;
    }
  | { type: "inputAnswered"; sessionId: string; questionId: string }
  | { type: "undoAvailable"; sessionId: string; assistantMessageId: string }
  | {
      type: "compactStart";
      sessionId: string;
      contextTokens: number;
      contextLimit: number;
    }
  | {
      type: "compactDone";
      sessionId: string;
      messagesBefore: number;
      messagesAfter: number;
      contextTokens?: number;
      contextLimit?: number;
      messages: ChatMessage[];
    }
  | {
      type: "cancelled";
      sessionId: string;
      messageId?: string | null;
      fullText?: string | null;
      thinkingContent?: string | null;
      thinkingDuration?: number | null;
      renderParts?: AssistantRenderPart[] | null;
    }
  | {
      type: "done";
      sessionId: string;
      messageId: string;
      fullText: string;
      contentOrder?: number;
      thinkingOrder?: number;
      renderParts?: AssistantRenderPart[];
    }
  | { type: "error"; sessionId: string; error: AppErrorPayload }
);

export interface AskOption {
  label: string;
  description: string;
}

export type PluginStatus =
  | { status: "missing" }
  | { status: "outdated" }
  | { status: "upToDate" };

export interface PendingQuestion {
  questionId: string;
  toolCallId: string;
  question: string;
  options: AskOption[];
}

export interface PendingToolConfirm {
  questionId: string;
  toolCallId: string;
  display: ToolConfirmDisplay;
}

export type PendingToolConfirmList = PendingToolConfirm[];

export interface BasicToolConfirmDisplay {
  kind: "basic";
  toolName: string;
  arguments: string;
}

export type KnowledgeToolConfirmDirectoryMode = "auto" | "approval";
export type KnowledgeToolConfirmOperation =
  | "create"
  | "edit"
  | "move"
  | "delete";

export interface KnowledgeToolConfirmPreview {
  kind: "knowledge";
  operation: KnowledgeToolConfirmOperation;
  targetKind: "document" | "directory";
  docType: KnowledgeDocumentType;
  path: string;
  newPath?: string | null;
  directoryPath: string;
  directoryMode: KnowledgeToolConfirmDirectoryMode;
  documentBeforeText?: string | null;
  documentAfterText?: string | null;
  structureBeforePaths?: string[];
  structureAfterPaths?: string[];
}

export interface UnityEditorStatusChangeToolConfirmDisplay {
  kind: "unityEditorStatusChange";
  toolName: string;
  currentStatus: string;
  requestedStatus: string;
}

export type ToolConfirmDisplay =
  | BasicToolConfirmDisplay
  | KnowledgeToolConfirmPreview
  | UnityEditorStatusChangeToolConfirmDisplay;

/**
 * Skill trigger mode:
 * - `command`: only appears as a slash command
 * - `auto`: only participates in semantic recall
 * - `both`: enables both entry points
 */
export type SkillSurface = "command" | "auto" | "both";

export function skillSurfaceAllowsAuto(s: SkillSurface | undefined): boolean {
  return s === "auto" || s === "both";
}

export function skillSurfaceAllowsCommand(
  s: SkillSurface | undefined,
): boolean {
  return s === "command" || s === "both" || s === undefined;
}

export interface SkillManifest {
  name: string;
  description: string;
  argumentHint: string;
  dirName: string;
  source: string;
  relPath: string;
  updatedAt: number;
  skillEnabled: boolean;
  skillSurface: SkillSurface;
  skillDescription: string | null;
  commandTrigger: string;
  tools?: string[];
  kind?: "document" | "package";
  packageId?: string | null;
  packageVersion?: string | null;
  hasUnity?: boolean;
  hasL0?: boolean;
  hasL1?: boolean;
  hasL2?: boolean;
  pluginId?: string | null;
  pluginScope?: "app" | "project" | string | null;
}

export interface SkillConfig {
  enabled: boolean;
  surface: SkillSurface;
  description: string;
  commandTrigger: string;
  injectMode?: KnowledgeInjectMode;
}

export type SkillUnityInstallState =
  | "pluginMissing"
  | "notApplicable"
  | "notInstalled"
  | "installed"
  | "partial"
  | "modified"
  | "sourceMissing";

export interface SkillUnityFileStatus {
  sourcePath: string;
  targetPath: string;
  state: string;
  sourceHash?: string | null;
  installedHash?: string | null;
}

export interface SkillUnityInstallStatus {
  packageId: string;
  hasUnity: boolean;
  state: SkillUnityInstallState;
  pluginRoot: string;
  installRoot: string;
  files: SkillUnityFileStatus[];
  message?: string | null;
}

export interface SkillPackageArchiveResult {
  packageId: string;
  path: string;
  fileCount: number;
  byteSize: number;
}

export interface SkillCreateInput {
  kind?: "md" | "package";
  name: string;
  path?: string;
  packageId?: string | null;
  version?: string | null;
  summary?: string | null;
  body?: string;
  argumentHint?: string | null;
  commandTrigger?: string | null;
  commandEnabled?: boolean;
  modelInvocationEnabled?: boolean;
  tools?: string[];
}

// ---------------------------------------------------------------------------
// Unified Knowledge model
// ---------------------------------------------------------------------------

export type KnowledgeDocumentType = "design" | "memory" | "skill" | "reference";
export type KnowledgeStorageSource = "project" | "app";
export type KnowledgeInjectMode = "none" | "path" | "excerpt" | "full" | "rule";
export type KnowledgeEditMode =
  | "inherit_parent"
  | "read_only"
  | "proposal"
  | "auto";
export type KnowledgeConfigSourceKind =
  | "self"
  | "parent_directory"
  | "type_default";

export interface KnowledgeConfigSource {
  kind: KnowledgeConfigSourceKind;
  path?: string | null;
}

export interface KnowledgeExternalSource {
  provider: "local_folder" | "feishu" | "url" | "package" | "unity" | "custom";
  locator?: string | null;
  sourceId?: string | null;
  syncEnabled?: boolean;
}

export interface KnowledgeFolderDisplayStats {
  directChildCount: number;
  descendantDocumentCount: number;
}

export interface KnowledgeManagedDirectoryStat extends KnowledgeFolderDisplayStats {
  path: string;
}

export interface KnowledgeDocumentSummary {
  id: string;
  type: KnowledgeDocumentType;
  path: string;
  title: string;
  injectMode: KnowledgeInjectMode;
  inheritInjectMode?: boolean;
  injectModeSource?: KnowledgeConfigSource | null;
  summaryEnabled: boolean;
  commandEnabled: boolean;
  readOnly: boolean;
  aiMaintained: boolean;
  storageSource?: KnowledgeStorageSource;
  inheritAiConfig?: boolean;
  aiConfigSource?: KnowledgeConfigSource | null;
  explicitMaintenanceRules: boolean;
  externalSource?: KnowledgeExternalSource | null;
  skillEnabled?: boolean | null;
  skillSurface?: SkillSurface | null;
  commandTrigger?: string | null;
  argumentHint?: string | null;
  tools?: string[];
  summary?: string | null;
  createdAt: number;
  updatedAt: number;
  hasSummary: boolean;
  hasBodyContent?: boolean;
  byteSize?: number;
  lexicalSearchEnabled?: boolean;
  semanticSearchEnabled?: boolean;
}

export interface KnowledgeDocumentFileMetadata {
  byteSize?: number;
  lineCount?: number;
  charCount?: number;
  estimatedTokens?: number;
  modifiedAt?: number;
  lastCommitAuthor?: string | null;
  lastCommitAt?: number | null;
}

export interface KnowledgeDocument extends KnowledgeDocumentSummary {
  body: string;
  maintenanceRules: string | null;
  fileMetadata?: KnowledgeDocumentFileMetadata | null;
}

export interface KnowledgeDirectoryConfig {
  version: number;
  summary: string;
  injectMode: KnowledgeInjectMode;
  inheritInjectMode?: boolean;
  aiMaintained: boolean;
  inheritAiConfig?: boolean;
  explicitMaintenanceRules: boolean;
  lexicalSearch: FolderIndexRuleSetting;
  vectorSearch: FolderIndexRuleSetting;
  inheritToChildren: boolean;
  allowCreateDocuments: boolean;
  allowCreateDirectories: boolean;
  allowMoveDocuments: boolean;
  allowMoveDirectories: boolean;
  maintenanceRules: string;
}

export interface KnowledgeDirectoryConfigRecord extends KnowledgeDirectoryConfig {
  type: KnowledgeDocumentType;
  path: string;
  configPath: string;
  exists: boolean;
  readOnly?: boolean;
  updatedAt: number;
  injectModeSource?: KnowledgeConfigSource | null;
  aiConfigSource?: KnowledgeConfigSource | null;
  effectiveLexicalSearch: EffectiveCapabilityState;
  effectiveVectorSearch: EffectiveCapabilityState;
  externalSources?: KnowledgeExternalSource[];
}

export interface KnowledgeExternalDirectoryBinding {
  path: string;
  externalSources: KnowledgeExternalSource[];
}

export type KnowledgeSearchMatchKind =
  | "lexical"
  | "semantic"
  | "hybrid"
  | "grep"
  | "grepHybrid";
export type KnowledgeSearchMatchSection =
  | "summary"
  | "body"
  | "maintenanceRules";

export interface KnowledgeSearchResult {
  id: string;
  type: KnowledgeDocumentType;
  path: string;
  title: string;
  storageSource?: KnowledgeStorageSource;
  injectMode: KnowledgeInjectMode;
  aiMaintained: boolean;
  snippet: string;
  matchKind: KnowledgeSearchMatchKind;
  matchedSection?: KnowledgeSearchMatchSection | null;
  matchedTerms?: string[];
  score: number;
  semanticScore?: number | null;
  semanticConfidence?: number | null;
  estimatedTokens?: number;
  updatedAt?: number;
}

export interface KnowledgeSearchSelectionContext {
  query: string;
  result: KnowledgeSearchResult;
}

export interface KnowledgeGeneralConfig {
  enabled: boolean;
  lexicalSearchEnabled: boolean;
  semanticSearchEnabled: boolean;
}

export interface KnowledgeFullTextOverview {
  enabled: boolean;
  indexableItemCount: number;
  indexedItemCount: number;
  freshItemCount: number;
  staleItemCount: number;
  pendingItemCount: number;
  chunkCount: number;
  lastBuildAt: string | null;
}

export interface KnowledgeSemanticOverview {
  enabled: boolean;
  ready: boolean;
  backend: string;
  model: string;
  deviceRoute: string;
  deviceName: string;
  indexedItemCount: number;
  embeddedChunkCount: number;
  pendingItemCount: number;
  coverageRatio: number;
  stage: string | null;
  error: string | null;
}

export interface KnowledgePerformanceOverview {
  dbBytes: number;
  lexicalIndexBytes: number;
  localModelBytes: number;
  gpuMemoryBytes: number;
  gpuDedicatedMemoryBytes: number;
  totalBytes: number;
  avgChunksPerItem: number;
}

export interface KnowledgeRetrievalOverview {
  totalDocumentCount: number;
  fullText: KnowledgeFullTextOverview;
  semantic: KnowledgeSemanticOverview;
  performance: KnowledgePerformanceOverview;
}

export type UnityReferenceImportStage =
  | "idle"
  | "resolving_source"
  | "downloading"
  | "extracting"
  | "converting"
  | "reconciling"
  | "ready"
  | "error";

export type UnityReferenceImportState =
  | "missing"
  | "unavailable"
  | "missing_current_version"
  | "outdated"
  | "running"
  | "ready"
  | "error";

export type UnityReferenceImportLastOutcome = "cancelled";
export type UnityReferenceImportLocale = "en" | "zh-CN";

export interface UnityReferenceImportStatus {
  state: UnityReferenceImportState;
  stage: UnityReferenceImportStage;
  running: boolean;
  projectVersion?: string | null;
  docsVersion?: string | null;
  selectedLocale?: UnityReferenceImportLocale | null;
  importedProjectVersion?: string | null;
  importedDocsVersion?: string | null;
  importedLocale?: UnityReferenceImportLocale | null;
  importedAt?: number | null;
  importedDocCount: number;
  managedPath: string;
  progress?: number | null;
  downloadedBytes?: number | null;
  totalBytes?: number | null;
  processedDocs: number;
  totalDocs?: number | null;
  currentPath?: string | null;
  sourceUrl?: string | null;
  message: string;
  error?: string | null;
  lastOutcome?: UnityReferenceImportLastOutcome | null;
}

export type FeishuReferenceAuthMode = "app_credentials" | "oauth";
export type FeishuReferenceOauthPersistenceMode = "session" | "offline";

export type FeishuReferenceImportStage =
  | "idle"
  | "saving_config"
  | "authorizing"
  | "testing_connection"
  | "listing_spaces"
  | "listing_nodes"
  | "importing"
  | "reconciling"
  | "ready"
  | "error";

export type FeishuReferenceImportState =
  | "missing_config"
  | "needs_authorization"
  | "running"
  | "ready"
  | "error";

export type FeishuReferenceImportLastOutcome = "cancelled";

export interface FeishuReferenceImportStatus {
  state: FeishuReferenceImportState;
  stage: FeishuReferenceImportStage;
  running: boolean;
  authMode: FeishuReferenceAuthMode;
  oauthPersistenceMode: FeishuReferenceOauthPersistenceMode;
  appId: string;
  appSecret?: string | null;
  appSecretConfigured: boolean;
  authorized: boolean;
  authorizedUserName?: string | null;
  authorizedUserOpenId?: string | null;
  authorizedUserEmail?: string | null;
  openBaseUrl: string;
  callbackUrls: string[];
  requiredScopes: string[];
  grantedScopes: string[];
  missingScopes: string[];
  accessTokenExpiresAt?: number | null;
  refreshTokenExpiresAt?: number | null;
  canRefresh: boolean;
  spaceId?: string | null;
  spaceName?: string | null;
  selectedRoots?: FeishuReferenceRootSelection[];
  rootNodeToken?: string | null;
  rootNodeTitle?: string | null;
  importedSpaceId?: string | null;
  importedSpaceName?: string | null;
  importedRoots?: FeishuReferenceRootSelection[];
  importedRootNodeToken?: string | null;
  importedRootNodeTitle?: string | null;
  importedAt?: number | null;
  importedDocCount: number;
  managedPath: string;
  progress?: number | null;
  processedDocs: number;
  totalDocs?: number | null;
  currentTitle?: string | null;
  currentPath?: string | null;
  message: string;
  error?: string | null;
  lastOutcome?: FeishuReferenceImportLastOutcome | null;
}

export interface KnowledgeCatalogStats {
  total: number;
  byType: Record<KnowledgeDocumentType, number>;
  byStorageSource: Record<KnowledgeStorageSource, number>;
  commandEnabled: number;
  aiMaintained: number;
  fullInjectable: number;
  summaryMissing: number;
  external: number;
}

export type KnowledgeDocumentSection = "summary" | "maintenanceRules" | "body";
export type KnowledgeTargetKind = "document" | "directory";

export interface KnowledgeDocumentEditOperation {
  section: KnowledgeDocumentSection;
  oldString: string;
  newString: string;
  replaceAll?: boolean;
}

export interface KnowledgeDocumentPatch {
  id?: string;
  type?: KnowledgeDocumentType;
  title?: string;
  injectMode?: KnowledgeInjectMode;
  inheritInjectMode?: boolean;
  summaryEnabled?: boolean;
  commandEnabled?: boolean;
  skillEnabled?: boolean;
  skillSurface?: SkillSurface;
  commandTrigger?: string | null;
  argumentHint?: string | null;
  readOnly?: boolean;
  aiMaintained?: boolean;
  inheritAiConfig?: boolean;
  explicitMaintenanceRules?: boolean;
  externalSource?: KnowledgeExternalSource | null;
  newPath?: string;
  summary?: string | null;
  body?: string | null;
  maintenanceRules?: string | null;
  edits?: KnowledgeDocumentEditOperation[];
}

export interface KnowledgeDocumentCreateInput extends KnowledgeDocumentPatch {
  title?: string;
  body?: string | null;
}

export interface KnowledgeDirectoryConfigPatch {
  version?: number;
  summary?: string;
  injectMode?: KnowledgeInjectMode;
  inheritInjectMode?: boolean;
  aiMaintained?: boolean;
  inheritAiConfig?: boolean;
  explicitMaintenanceRules?: boolean;
  lexicalSearch?: FolderIndexRuleSetting;
  vectorSearch?: FolderIndexRuleSetting;
  inheritToChildren?: boolean;
  allowCreateDocuments?: boolean;
  allowCreateDirectories?: boolean;
  allowMoveDocuments?: boolean;
  allowMoveDirectories?: boolean;
  maintenanceRules?: string;
}

export interface KnowledgeReadInput {
  kind: KnowledgeTargetKind;
  path: string;
  type?: KnowledgeDocumentType;
  part?: "full" | "summary" | "body";
}

export interface KnowledgeCreateInput {
  kind: KnowledgeTargetKind;
  path: string;
  type?: KnowledgeDocumentType;
  document?: KnowledgeDocumentCreateInput;
}

export interface KnowledgeEditInput {
  kind: KnowledgeTargetKind;
  path: string;
  type?: KnowledgeDocumentType;
  document?: KnowledgeDocumentPatch;
  config?: KnowledgeDirectoryConfigPatch;
}

export interface KnowledgeMoveInput {
  kind: KnowledgeTargetKind;
  path: string;
  type?: KnowledgeDocumentType;
  newPath: string;
}

export interface KnowledgeDeleteInput {
  kind: KnowledgeTargetKind;
  path: string;
  type?: KnowledgeDocumentType;
}

export interface KnowledgeReadResult {
  kind: KnowledgeTargetKind;
  document?: KnowledgeDocument | null;
  directory?: KnowledgeDirectoryConfigRecord | null;
}

export interface KnowledgeMutationResult {
  kind: KnowledgeTargetKind;
  type: KnowledgeDocumentType;
  path: string;
  resultPath?: string | null;
  document?: KnowledgeDocument | null;
  directory?: KnowledgeDirectoryConfigRecord | null;
}

export interface KnowledgeDocumentListInput {
  type?: KnowledgeDocumentType;
  pathPrefix?: string;
  includeHidden?: boolean;
  limit?: number;
  cursor?: string | null;
}

export interface KnowledgeDocumentListPage {
  items: KnowledgeDocumentSummary[];
  nextCursor?: string | null;
}

export interface KnowledgeDocumentQueryInput {
  query: string;
  limit?: number;
  types?: KnowledgeDocumentType[];
  pathPrefix?: string;
  includeHidden?: boolean;
}

// ---------------------------------------------------------------------------
// Knowledge source and virtualization types
// ---------------------------------------------------------------------------

export interface KnowledgeSourceConfig {
  id: string;
  type: "filesystem" | "feishu";
  syncMode: "append" | "mirror";
  displayName: string;
  sourceKind: "app" | "project" | "feishu";
  rootPath: string;
  includeGlobs: string[];
  excludeGlobs: string[];
  enabled: boolean;
  readOnly: boolean;
  lastSyncedAt?: string | null;
  feishu?: FeishuSourceConfig | null;
}

export interface FeishuSourceConfig {
  authMode?: FeishuReferenceAuthMode;
  appId: string;
  appSecret: string;
  appSecretConfigured: boolean;
  clearAppSecret: boolean;
  openBaseUrl: string;
  spaceId: string;
  spaceName?: string;
  rootNodeToken: string;
  rootNodeTitle?: string;
  allowedObjTypes: ("doc" | "docx")[];
}

export interface KnowledgeSourceSyncReport {
  imported: number;
  updated: number;
  removed: number;
  skipped: number;
  errors: number;
}

export interface FeishuSpaceSummary {
  spaceId: string;
  name: string;
}

export interface FeishuSourceTestResult {
  summary: string;
  openBaseUrl: string;
  spaceCount: number;
  spaces: FeishuSpaceSummary[];
  resolvedSpaceId?: string | null;
  resolvedSpaceName?: string | null;
  resolvedRootNodeToken?: string | null;
  resolvedRootNodeTitle?: string | null;
}

export interface FeishuReferenceNodeSummary {
  nodeToken: string;
  title: string;
  objToken: string;
  objType: string;
  hasChild: boolean;
  parentNodeToken?: string | null;
}

export interface FeishuReferenceRootSelection {
  nodeToken: string;
  nodeTitle?: string | null;
}

export interface FeishuReferenceConfigInput {
  targetPath?: string | null;
  authMode: FeishuReferenceAuthMode;
  oauthPersistenceMode: FeishuReferenceOauthPersistenceMode;
  appId: string;
  appSecret?: string | null;
  clearAppSecret: boolean;
  openBaseUrl: string;
  spaceId?: string | null;
  spaceName?: string | null;
  roots?: FeishuReferenceRootSelection[];
  rootNodeToken?: string | null;
  rootNodeTitle?: string | null;
}

export interface FeishuReferenceImportRequest {
  targetPath?: string | null;
  spaceId: string;
  spaceName?: string | null;
  roots?: FeishuReferenceRootSelection[];
  rootNodeToken?: string | null;
  rootNodeTitle?: string | null;
}

export interface FeishuReferenceOauthStartResult {
  authorizeUrl: string;
  callbackUrl: string;
  callbackUrls: string[];
  state: string;
}

export interface KnowledgeDocItem {
  id: string;
  title: string;
  sourceKind: string;
  sourceId: string;
  docKind: string;
  revisionId: string;
  commitId?: string | null;
  updatedAt: number;
  validityState: "fresh" | "stale" | "unknown";
  hasL0: boolean;
  hasL1: boolean;
  hasL2: boolean;
  subcategory: string;
  relativeDir: string;
  relativePath: string;
  fileName: string;
}

export interface KnowledgeVirtualDirectoryEntry {
  path: string;
  name: string;
  entryType: string;
}

export interface KnowledgeVirtualDocVariantEntry {
  variant: "doc" | "source";
  docId: string;
  docKind: string;
  sourceKind: string;
  revisionId: string;
  validityState: string;
  updatedAt: number;
  physicalPath: string;
}

export interface KnowledgeVirtualDocEntry {
  path: string;
  entryType: string;
  title: string;
  relativeDir: string;
  availableVariants: string[];
  variants: KnowledgeVirtualDocVariantEntry[];
}

export interface KnowledgeVirtualTree {
  path: string;
  directories: KnowledgeVirtualDirectoryEntry[];
  documents: KnowledgeVirtualDocEntry[];
}

export interface KnowledgeVirtualDocReadResult {
  path: string;
  variant: "doc" | "source";
  availableVariants: string[];
  physicalPath: string;
  docId: string;
  title: string;
  relativeDir: string;
  docKind: string;
  sourceKind: string;
  revisionId: string;
  commitId?: string | null;
  validityState: string;
  hasL0: boolean;
  hasL1: boolean;
  hasL2: boolean;
  level: string;
  content: string;
}

export interface KnowledgeVirtualDocWriteResult {
  path: string;
  variant: "doc" | "source";
  physicalPath: string;
  docId: string;
  title: string;
  docKind: string;
  sourceKind: string;
  revisionId: string;
  updatedAt: number;
}

export interface KnowledgeDocUpdateTaskSummary {
  sessionId: string;
  targetPath: string;
  targetTitle: string;
  status: SessionRuntimeStatus;
  requestedAt: number;
  startedAt?: number | null;
  sourceTokens?: number;
  existingDocTokens?: number;
  estimatedInputTokens?: number;
}

export interface EffectiveCapabilityState {
  enabled: boolean;
  source: string;
  reasonCode?: string;
  sourceDir?: string;
}

export type FolderIndexRuleSetting = "inherit" | "enabled" | "disabled";

export interface FolderIndexRule {
  lexicalSearch: FolderIndexRuleSetting;
  vectorSearch: FolderIndexRuleSetting;
}

export interface AgentInjectionCapability {
  supported: boolean;
  active: boolean;
  reason: "disabled" | "active" | "not_in_window" | "unsupported" | string;
}

export interface DocAgentAccess {
  directoryBrowseVisible: boolean;
  lexicalSearch: EffectiveCapabilityState;
  vectorSearch: EffectiveCapabilityState;
  contextInjectionL0: AgentInjectionCapability;
  contextInjectionL1: AgentInjectionCapability;
  contextInjectionL2: AgentInjectionCapability;
}

export interface FolderAlwaysOnInjectionSummary {
  supported: boolean;
  active: boolean;
  reason: string;
  totalDocCount: number;
  includedDocCount: number;
  bytes: number;
  estimatedTokens: number;
  truncated: boolean;
}

export interface WikiFolderAccessSummary {
  relativeDir: string;
  directoryBrowseVisible: boolean;
  lexicalSearch: EffectiveCapabilityState;
  vectorSearch: EffectiveCapabilityState;
  folderRule: FolderIndexRule;
  alwaysOnInjection: FolderAlwaysOnInjectionSummary;
}

export interface WikiQueryResult {
  docId: string;
  title: string;
  level: string;
  score: number;
  semanticScore?: number | null;
  semanticConfidence?: number | null;
  matchKind: "semantic" | "lexical" | "exact" | "hybrid";
  snippet: string;
  sourceKind: string;
  validityState: string;
  suggestedReadAnchor: string | null;
}

export interface WikiGeneralConfig {
  enabled: boolean;
  lexicalSearchEnabled: boolean;
  injectIndex: boolean;
  agentTools: boolean;
}

export interface EmbeddingConfig {
  enabled: boolean;
  embeddingMode: string;
  // local
  devicePolicy: string;
  localRuntime: string;
  localModel: string;
  localModelPath: string;
  localModelDownloadSource: string;
  // remote (OpenAI-compatible /v1/embeddings)
  remoteEndpoint: string;
  remoteApiKey: string;
  remoteModel: string;
  remoteDimensions: number;
  remoteMaxBatch: number;
}

export interface EmbeddingModelPreset {
  id: string;
  label: string;
  downloaded: boolean;
  dimensions: number;
}

export interface EmbeddingAvailableLocalModel {
  modelId: string;
  label: string;
  localModelPath: string;
  dimensions: number;
}

export interface EmbeddingLocalModelCatalog {
  managedDirectory: string;
  presets: EmbeddingModelPreset[];
  availableModels: EmbeddingAvailableLocalModel[];
}

export interface EmbeddingLocalModelDirectoryInspection {
  path: string;
  label: string;
  ready: boolean;
  modelFile: string | null;
  missingFiles: string[];
}

export interface EmbeddingDownloadNetworkStatus {
  source: string;
  endpoint: string;
  proxyState: string;
  proxyEnvKey: string | null;
  proxyUrl: string | null;
}

export interface EmbeddingStatus {
  enabled: boolean;
  ready: boolean;
  activating: boolean;
  modelDownloaded: boolean;
  modelDownloadProgress: number | null;
  indexProgress: number | null;
  error: string | null;
  stage: string | null;
  detail: string | null;
  currentFile: string | null;
  downloadedBytes: number | null;
  totalBytes: number | null;
  processedDocs: number | null;
  totalDocs: number | null;
  failedDocs?: number | null;
  lastFailedFile?: string | null;
  lastFailure?: string | null;
  downloadNetwork: EmbeddingDownloadNetworkStatus | null;
  lastTestSummary: string | null;
  lastTestPassed: boolean | null;
}

export interface EmbeddingRuntimeTestResult {
  passed: boolean;
  summary: string;
  backend: string;
  modelId: string;
  dimension: number;
  vectorCount: number;
  latencyMs: number;
  cases: EmbeddingRuntimeTestCaseResult[];
  diagnostics?: EmbeddingRuntimeTestDiagnostics | null;
}

export interface EmbeddingRuntimeTestCaseResult {
  caseId: string;
  route: string;
  provider: string;
  backend: string;
  modelId: string;
  dimension: number;
  vectorCount: number;
  latencyMs: number;
  outcome: string;
  error?: string | null;
}

export interface EmbeddingRuntimeTestDiagnostics {
  dlls: EmbeddingRuntimeTestDllInfo[];
  adapters: EmbeddingRuntimeTestAdapterInfo[];
}

export interface EmbeddingRuntimeTestDllInfo {
  name: string;
  path?: string | null;
  exists: boolean;
  version?: string | null;
}

export interface EmbeddingRuntimeTestAdapterInfo {
  index: number;
  name: string;
  vendorId: number;
  deviceId: number;
  dedicatedVramBytes: number;
  isSoftware: boolean;
  isHighPerformance: boolean;
}

export interface LexicalRebuildStatus {
  running: boolean;
  stage: string | null;
  detail: string | null;
  currentFile: string | null;
  progress?: number | null;
  processedDocs: number | null;
  totalDocs: number | null;
  error: string | null;
  startedAt: string | null;
  completedAt: string | null;
}

export interface WikiQueryInput {
  query: string;
  scope?: string;
  limit?: number;
  preferLevel?: string;
  intent?: string;
}

export interface VcsRevisionRef {
  provider: string;
  revisionId: string;
  revisionKind: string;
  display: string;
}

// ---------------------------------------------------------------------------

export interface RuleItem {
  key: string;
  fileName: string;
  title: string;
  order: number;
  enabled: boolean;
  updatedAt: number;
  source: string;
  readOnly: boolean;
  pluginId?: string | null;
  pluginScope?: "app" | "project" | string | null;
}

export interface AgentSystemPromptStats {
  baseChars: number;
  envChars: number;
  rulesChars: number;
  knowledgeChars: number;
  totalChars: number;
}

export type InjectedToolLoadMode = "direct" | "lazy" | "skill";

export interface InjectedToolMeta {
  function?: unknown;
  loadMode?: InjectedToolLoadMode;
  loadReason?: string;
  directLoaded?: boolean;
  directLoadDefault?: boolean;
  directLoadOverride?: boolean | null;
  canConfigureDirectLoad?: boolean;
  nativeLazy?: boolean;
  toolSource?: "builtIn" | "skill" | string;
}

export interface InjectedPromptItem {
  id: string;
  title: string;
  kind: "rule" | "context" | "tools";
  content: string;
  source: "builtIn" | "runtime" | "system";
  meta?: InjectedToolMeta | Record<string, unknown> | null;
}

export interface VcsCheckpoint {
  id: string;
  label: string;
  createdAt: number;
}

export interface VcsUndoEntry {
  id: string;
  sessionId: string;
  assistantMessageId: string;
  runId?: string | null;
  checkpoint: VcsCheckpoint;
  changedFiles: ChangedFile[];
  hasUnityExecute: boolean;
  consumed: boolean;
}

export interface UndoConflictInfo {
  sessionId: string;
  sessionTitle: string;
  assistantMessageId: string;
  checkpoint: VcsCheckpoint;
  changedFiles: ChangedFile[];
}

export interface ChangedFile {
  status: string;
  path: string;
  oldPath?: string;
}

export interface GitCommitInfo {
  hash: string;
  shortHash: string;
  parents: string[];
  author: string;
  /** Unix timestamp (seconds) */
  date: number;
  message: string;
  refs: string[];
  isStash: boolean;
}

export interface GitLogResult {
  isRepo: boolean;
  commits: GitCommitInfo[];
  headHash: string | null;
}

export type GitHeadKind = "attached" | "detached";

export interface GitHeadState {
  hash: string | null;
  kind: GitHeadKind;
  refName: string | null;
}

export type GitGraphRefKind = "localBranch" | "remoteBranch" | "tag";

export interface GitGraphRef {
  fullName: string;
  shortName: string;
  targetHash: string;
  kind: GitGraphRefKind;
  isCurrent: boolean;
  remoteName?: string | null;
  branchName?: string | null;
}

export interface GitWorkspaceSummary {
  changeCount: number;
  unstagedCount: number;
  stagedCount: number;
  unmergedCount: number;
}

export interface GitHistorySnapshot {
  isRepo: boolean;
  commits: GitCommitInfo[];
  hasMore: boolean;
  head: GitHeadState;
  refs: GitGraphRef[];
  stashes: GitStashEntry[];
  workspace: GitWorkspaceSummary;
}

export interface GitHistorySearchRequest {
  query?: string | null;
  useRegex?: boolean | null;
  author?: string | null;
  /** Unix timestamp (seconds), inclusive */
  dateFrom?: number | null;
  /** Unix timestamp (seconds), inclusive */
  dateTo?: number | null;
}

export type GitHistorySearchResultKind = "commit" | "stash";

export interface GitHistorySearchResult {
  kind: GitHistorySearchResultKind;
  hash: string;
  shortHash: string;
  author: string;
  /** Unix timestamp (seconds) */
  date: number;
  message: string;
  refName?: string | null;
  files: GitFileChange[];
}

export interface GitHistorySearchResponse {
  isRepo: boolean;
  results: GitHistorySearchResult[];
  truncated: boolean;
}

export interface GitProbeResult {
  available: boolean;
  inPath: boolean;
  path?: string;
  source?: GitRuntimeSource;
  version?: string;
  envOverride?: string;
  isRepo: boolean;
}

export interface GitInstallManager {
  id: string;
  label: string;
  command: string;
  available: boolean;
}

export interface GitInstallHelp {
  os: "windows" | "macos" | "linux";
  packageManagers: GitInstallManager[];
  officialUrl: string;
  chinaMirrorUrl?: string;
}

export type GitConfigScope = "repo" | "global";

export interface GitConfigEntry {
  key: string;
  value: string;
}

export interface GitConfigScopeSnapshot {
  scope: GitConfigScope;
  path?: string | null;
  entries: GitConfigEntry[];
}

export interface GitConfigSnapshot {
  repo: GitConfigScopeSnapshot;
  global: GitConfigScopeSnapshot;
}

export interface GitFileChange {
  path: string;
  /** Old path for renames */
  oldPath?: string;
  /** "M" modified, "A" added, "D" deleted, "R" renamed, "?" untracked */
  status: string;
  /** Whether the file is tracked by Git LFS */
  lfs: boolean;
  /** Best-effort workspace lookup for `.meta` primary paths in working-tree views. */
  primaryExistsInWorkspace?: boolean;
  /** True when the `.meta` primary path currently resolves to a directory. */
  primaryIsDirectoryInWorkspace?: boolean;
}

export type GitBlockedPathReason =
  | "windowsReservedName"
  | "windowsTrailingDot"
  | "windowsTrailingSpace";

export interface GitBlockedPath {
  path: string;
  oldPath?: string;
  status: string;
  reason: GitBlockedPathReason;
  segment: string;
}

export interface GitStatusResult {
  unstaged: GitFileChange[];
  staged: GitFileChange[];
  blocked?: GitBlockedPath[];
  unmerged: UnmergedFileEntry[];
  operation: MergeOperation | null;
  warnings?: AppErrorPayload[];
}

export interface GitStageAllResult {
  stagedCount: number;
  skippedCount: number;
  blocked: GitBlockedPath[];
  stdout: string;
  stderr: string;
}

export interface UnmergedFileEntry {
  path: string;
  conflictCode: string;
  semanticLabel: string;
  baseOid: string;
  leftOid: string;
  rightOid: string;
  lfs: boolean;
  headMode: string;
  stage1Mode: string;
  stage2Mode: string;
  stage3Mode: string;
  /** Best-effort workspace lookup for `.meta` primary paths in working-tree views. */
  primaryExistsInWorkspace?: boolean;
  /** True when the `.meta` primary path currently resolves to a directory. */
  primaryIsDirectoryInWorkspace?: boolean;
}

export type MergeOperationKind =
  | "merge"
  | "cherryPick"
  | "rebase"
  | "revert"
  | "genericConflict";

export interface MergeOperation {
  kind: MergeOperationKind;
  canContinue: boolean;
  canSkip: boolean;
  canAbort: boolean;
  label: string;
}

export interface ConflictBlock {
  index: number;
  startLine: number;
  endLine: number;
  leftContent: string;
  rightContent: string;
  baseContent: string;
  leftMarkerLabel?: string;
  rightMarkerLabel?: string;
}

export interface MergeFileInfo {
  conflictCode: string;
  semanticLabel: string;
  workspaceText: string | null;
  workspaceMatchesCanonical: boolean;
  conflictBlocks: ConflictBlock[];
  isBinary: boolean;
  isLfs: boolean;
  isSubmodule: boolean;
  baseOid: string;
  leftOid: string;
  rightOid: string;
  /** Semantic label for Stage 2 side (e.g. "Current (main)", "Rebase target (main)") */
  leftLabel: string;
  /** Semantic label for Stage 3 side (e.g. "Incoming (feature)", "Your commit (abc123)") */
  rightLabel: string;
}

export type MergeApplyMode =
  | { resolvedText: { text: string } }
  | { takeStage: { stage: "left" | "right" | "base" | "delete" } };

export type MergeActionKind = "continue" | "skip" | "abort";

// ── Merge Semantic Types ──

export type MergeState = "auto" | "conflict" | "unchanged";
export type MergeSide = "base" | "ours" | "theirs";
export type DocMergeStatus =
  | "unchanged"
  | "autoResolved"
  | "hasConflicts"
  | "addedOurs"
  | "addedTheirs"
  | "removedOurs"
  | "removedTheirs";

export interface MergeField {
  id: string;
  propertyPath: string;
  label: string;
  valueType: string;
  base?: string;
  ours?: string;
  theirs?: string;
  result?: string;
  mergeState: MergeState;
  autoChoice?: MergeSide;
  manualChoice?: MergeSide;
  children: MergeField[];
  fieldType?: string;
  referenceBase?: InspectorReference;
  referenceOurs?: InspectorReference;
  referenceTheirs?: InspectorReference;
}

export interface InspectorComponentInference {
  reasonCode: string;
  evidence: string[];
  inferredClassId?: number;
}

export type InspectorComponentSource =
  | "builtin"
  | "script"
  | "gameObjectHeader"
  | "assetRoot"
  | "subObject"
  | "modelImporterMeta"
  | "modelImporterMetaHeuristic"
  | "inferred";

export interface MergePanel {
  panelKind: string;
  title: string;
  scriptClass?: string;
  componentType?: string;
  componentSource?: InspectorComponentSource;
  componentInference?: InspectorComponentInference;
  mergeStatus: DocMergeStatus;
  fields: MergeField[];
}

export interface MergeTargetSummary {
  id: string;
  label: string;
  path: string;
  mergeStatus: DocMergeStatus;
  conflictCount: number;
  autoResolvedCount: number;
}

export interface MergeTargetInspector {
  targetId: string;
  title: string;
  path: string;
  panels: MergePanel[];
}

export interface MergeSummary {
  totalTargets: number;
  conflictingTargets: number;
  autoResolvedTargets: number;
  totalConflicts: number;
  totalAutoResolved: number;
}

export interface MergeSessionPayload {
  key: string;
  filePath: string;
  semanticAvailable: boolean;
  fallbackReason?: string;
  assetKind?: UnityAssetKind;
  layout?: SemanticLayout;
  summary?: MergeSummary;
  tree?: SemanticTreeNode[];
  targets?: MergeTargetSummary[];
  defaultTargetId?: string;
  inspector?: MergeTargetInspector;
}

export interface MergeSessionRequest {
  filePath: string;
  baseOid: string;
  leftOid: string;
  rightOid: string;
}

export interface MergeTargetRequest {
  mergeKey: string;
  targetId: string;
}

export interface FieldResolution {
  side: MergeSide;
}

export interface MergeApplyRequest {
  mergeKey: string;
  filePath: string;
  resolutions: Record<string, FieldResolution>;
}

export interface GitBranchInfo {
  name: string;
  isCurrent: boolean;
  shortHash: string;
  message: string;
}

export interface GitRemoteBranch {
  name: string;
  shortHash: string;
  message: string;
}

export interface GitBranchesResult {
  local: GitBranchInfo[];
  /** [remote_name, branches][] */
  remotes: [string, GitRemoteBranch[]][];
}

export interface GitStashEntry {
  index: number;
  refName: string;
  hash: string;
  shortHash: string;
  author: string;
  date: number;
  message: string;
  parentHashes: string[];
  baseHash?: string | null;
}

export type GitHistorySelection =
  | { kind: "workspace" }
  | { kind: "commit"; hash: string }
  | { kind: "stash"; hash: string; refName: string };

// ── Context-menu target & action types ───────────────────────────

export type GitHistoryTarget =
  | { kind: "commit"; commit: GitCommitInfo }
  | { kind: "stash"; stash: GitStashEntry; selectedStashes?: GitStashEntry[] };

export type GitBranchTarget =
  | { kind: "localBranch"; branch: GitBranchInfo }
  | { kind: "remoteBranch"; remoteName: string; branch: GitRemoteBranch };

export interface GitActionResult {
  status: "success" | "conflict";
  message: string;
  stdout: string;
  stderr: string;
}

export interface GitSubmoduleInfo {
  path: string;
  name: string;
  hash: string;
  /** "ok" | "uninitialized" | "modified" */
  status: string;
}

export interface ToolCallDisplay {
  id: string;
  name: string;
  arguments: string;
  status: "running" | "done" | "error" | "interrupted";
  order?: number;
  output?: string;
  images?: ImageAttachment[];
  progress?: ToolCallProgress | null;
  nestedToolCalls?: ToolCallDisplay[];
}

export interface ToolCallProgress {
  title: string;
  info: string;
  progress?: number | null;
  state: string;
}

export type NotificationLevel = "error" | "warning" | "success" | "info";

export interface AppErrorPayload {
  code: string;
  message: string;
  detail?: string;
  operation?: string;
  retryable: boolean;
  severity: NotificationLevel;
}

export type DebugConsoleLevel = "trace" | "debug" | "info" | "warn" | "error";
export type DebugConsoleSource = "backend" | "frontend";

export interface DebugConsoleEntry {
  id: string;
  timestampMs: number;
  level: DebugConsoleLevel;
  source: DebugConsoleSource;
  module: string;
  target: string;
  message: string;
}

// ── Diff types ──

export type DiffSource =
  | "gitCommit"
  | "gitStaged"
  | "gitUnstaged"
  | "chatCheckpoint"
  | "gitConflictBaseToLeft"
  | "gitConflictBaseToRight";
export type DiffDetail = "preview" | "full";

export interface FileDiffRequest {
  source: DiffSource;
  filePath: string;
  oldPath?: string;
  commitHash?: string;
  sessionId?: string;
  assistantMessageId?: string;
  detail: DiffDetail;
  fullContext?: boolean;
}

export interface DiffStats {
  additions: number;
  deletions: number;
  changedHunks: number;
}

export interface DiffLine {
  kind: "context" | "add" | "delete";
  content: string;
  oldLineNo: number | null;
  newLineNo: number | null;
}

export interface DiffHunk {
  header: string;
  oldStart: number;
  oldCount: number;
  newStart: number;
  newCount: number;
  lines: DiffLine[];
}

export interface TextDiff {
  hunks: DiffHunk[];
}

export type UnityAssetKind =
  | "scene"
  | "prefab"
  | "material"
  | "scriptableObject"
  | "animationClip"
  | "animatorController"
  | "genericYaml";
export type SemanticLayout = "sceneHierarchyInspector" | "assetInspector";

export interface SemanticSummary {
  changedTargets: number;
  changedObjects: number;
  changedComponents: number;
  changedFields: number;
}

export interface SemanticBadgeCounts {
  added: number;
  removed: number;
  modified: number;
  componentsChanged: number;
}

export interface SemanticTreeNode {
  id: string;
  parentId?: string | null;
  label: string;
  objectKind: string;
  changeKind:
    | "added"
    | "removed"
    | "modified"
    | "unchanged"
    | "conflict"
    | "autoResolved"
    | "oursOnly"
    | "theirsOnly";
  path: string;
  childIds: string[];
  badgeCounts: SemanticBadgeCounts;
  hasInspector: boolean;
}

export type SourceMode = "snapshot" | "workspace" | "unityEnhanced";

export interface SemanticTargetSummary {
  id: string;
  label: string;
  subtitle?: string;
  path: string;
  changeKind: "added" | "removed" | "modified";
  hasInspector: boolean;
  targetKind?: "mainAsset" | "subAsset";
  scriptClass?: string;
  isMainObject?: boolean;
  sourceMode?: SourceMode;
}

export interface InspectorReference {
  guid?: string;
  path?: string;
  fileId?: number;
  /** Diagnostic hint when GUID resolution failed */
  resolveHint?: string;
  /** true when path was resolved from current workspace for a snapshot side (may not match historical state) */
  stale?: boolean;
}

export interface InspectorField {
  id: string;
  label: string;
  propertyPath: string;
  valueType: string;
  changeKind: "added" | "removed" | "modified" | "unchanged";
  before?: string;
  after?: string;
  children?: InspectorField[];
  reference?: InspectorReference;
  /** C# declared type from script source (e.g. "int", "float", "Color") */
  fieldType?: string;
}

export type SemanticDisplayMode = "optimized" | "full";

export interface InspectorPanel {
  panelKind: "gameObjectHeader" | "component" | "assetRoot" | "subObject";
  title: string;
  scriptClass?: string;
  changeKind: "added" | "removed" | "modified" | "unchanged";
  added: boolean;
  removed: boolean;
  componentType?: string;
  componentClassId?: number;
  componentSource?: InspectorComponentSource;
  componentResolveReason?: string;
  componentInference?: InspectorComponentInference;
  fields: InspectorField[];
}

export interface SemanticTargetInspector {
  targetId: string;
  title: string;
  subtitle?: string;
  path: string;
  panels: InspectorPanel[];
}

export interface SemanticDiff {
  engine: string;
  assetKind: UnityAssetKind;
  layout: SemanticLayout;
  summary: SemanticSummary;
  defaultTargetId?: string;
  /** Script class name for the main asset (e.g. "PlayerInputConstraint" for ScriptableObjects) */
  scriptClassName?: string;
  tree?: SemanticTreeNode[];
  targets?: SemanticTargetSummary[];
  inspector?: SemanticTargetInspector;
}

export interface SemanticTargetRequest {
  diffKey: string;
  targetId: string;
  includeUnchanged: boolean;
}

export type DiffContentState =
  | { type: "normal" }
  | { type: "lfsResolved" }
  | { type: "lfsNotFetched"; oid: string; size: number };

export interface FileDiffPayload {
  key: string;
  filePath: string;
  oldPath?: string;
  status: string;
  language?: string;
  isBinary: boolean;
  isLarge: boolean;
  contentState: DiffContentState;
  stats: DiffStats;
  previewSummary: string[];
  text?: TextDiff;
  semantic?: SemanticDiff;
  binaryPreview?: BinaryPreview;
}

export type BinaryPreviewKind = "image" | "psd" | "model";

export interface BinaryAssetRef {
  url: string;
  mimeType?: string;
  byteSize: number;
}

export interface BinaryPreview {
  kind: BinaryPreviewKind;
  before?: BinaryAssetRef;
  after?: BinaryAssetRef;
}

// ---------------------------------------------------------------------------
// Asset page (Slice 4 + 5)
// ---------------------------------------------------------------------------

export type AssetDbStatus = "indexed" | "scanning" | "none" | "error";

export interface AssetKindCount {
  kind: string;
  count: number;
}

export interface AssetDbOverview {
  status: AssetDbStatus;
  nodes: number;
  edges: number;
  dbBytes: number;
  assetBytes: number;
  lastScanAt?: number;
  lastScanDurationMs?: number;
  lastScanStats?: ScanStats;
  watcherRunning: boolean;
  /** Number of pending dirty assets in the incremental watcher queue. */
  watcherQueueLen: number;
  /** Workspace-relative path of the asset currently being processed, if any. */
  watcherCurrentFile?: string;
  byKind: AssetKindCount[];
  assetRisks: AssetRiskEntry[];
  duplicateGuids: DuplicateGuidOverview;
  duplicateGuidReportPath?: string;
  /** Sticky scan-phase snapshot. Same shape as AssetDbScanEvent. */
  currentScanPhase?: AssetDbScanEvent;
}

export interface AssetDbLightStatus {
  status: AssetDbStatus;
  nodes: number;
  edges: number;
  lastScanAt?: number;
  lastScanDurationMs?: number;
  lastScanStats?: ScanStats;
  currentScanPhase?: AssetDbScanEvent;
}

export interface RefGraphScanStartResult {
  started: boolean;
  alreadyRunning: boolean;
}

export interface WatcherTuning {
  debounceMs: number;
  workerCount: number;
  maxWorkerCount: number;
}

export type AssetSearchRoot = "assets" | "packages" | "projectSettings";
export type AssetSearchSource = "assetDb" | "filesystem";

export interface AssetSearchResult {
  path: string;
  name: string;
  guid?: string;
  fileId?: number;
  objectKey?: string;
  root: AssetSearchRoot;
  kind: string;
  typeLabel?: string;
  typeSearch?: string;
  isSubAsset?: boolean;
  targetId?: string;
  matchScore: number;
  source: AssetSearchSource;
}

export interface AssetTextPreview {
  snippet: string;
  truncated: boolean;
  totalLines: number;
  language?: string;
}

export interface AssetBinaryMeta {
  path: string;
  name: string;
  size: number;
  ext: string;
  guid?: string;
  unityTexture?: UnityTexturePreviewMeta;
}

export interface UnityTexturePreviewMeta {
  importer?: string;
  alphaIsTransparency?: boolean;
}

export interface AssetTargetMeta {
  id: string;
  title: string;
  subtitle?: string;
}

export type AssetPreviewPayload =
  | ({ kind: "text" } & AssetTextPreview)
  | { kind: "binaryPreview"; preview: BinaryPreview; meta: AssetBinaryMeta }
  | { kind: "binaryInfo"; meta: AssetBinaryMeta }
  | {
      kind: "structured";
      previewKey: string;
      layout: SemanticLayout;
      tree: SemanticTreeNode[];
      targets: AssetTargetMeta[];
    };
