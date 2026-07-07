export interface ReferenceExternalImportOption {
  value: string;
  label: string;
  disabled?: boolean;
}

export interface ReferenceExternalImportWindowRow {
  label: string;
  value: string;
  mono?: boolean;
}

export interface ReferenceExternalImportUnityStageItem {
  key: string;
  label: string;
  complete: boolean;
  current: boolean;
  error: boolean;
  progress: number;
  statusText: string;
}

export interface ReferenceExternalImportUnityWindowModel {
  summary: string;
  locale: string;
  localeOptions: ReferenceExternalImportOption[];
  localeDisabled: boolean;
  foreignBindingText: string;
  canOpenExisting: boolean;
  stageTitle: string;
  stageCaption: string;
  progressLabel: string;
  progressRatio: number;
  stageItems: ReferenceExternalImportUnityStageItem[];
  rows: ReferenceExternalImportWindowRow[];
  detail: string;
  currentPath: string;
  currentPathLabel: string;
  canDelete: boolean;
  canCancel: boolean;
  cancelDisabled: boolean;
  primaryDisabled: boolean;
  primaryClosesWindow: boolean;
  primaryLabel: string;
  cancelLabel: string;
  deleteLabel: string;
  openExistingLabel: string;
}

export interface ReferenceExternalImportLocalWindowModel {
  summary: string;
  sourcePath: string;
  sourcePathPlaceholder: string;
  sourceLabel: string;
  pickFileLabel: string;
  pickFolderLabel: string;
  pickDisabled: boolean;
  previewText: string;
  modeLabel: string;
  mode: string;
  modeOptions: ReferenceExternalImportOption[];
  modeDisabled: boolean;
  modeHint: string;
  aiEditableVisible: boolean;
  aiEditableChecked: boolean;
  aiEditableDisabled: boolean;
  aiEditableLabel: string;
  aiEditableHint: string;
  targetNameLabel: string;
  targetName: string;
  targetNameDisabled: boolean;
  sourceMissingText: string;
  stageTitle: string;
  progressLabel: string;
  progressRatio: number;
  detail: string;
  rows: ReferenceExternalImportWindowRow[];
  currentPath: string;
  currentPathLabel: string;
  canSync: boolean;
  syncDisabled: boolean;
  syncLabel: string;
  canDelete: boolean;
  deleteLabel: string;
  canCancel: boolean;
  cancelDisabled: boolean;
  cancelLabel: string;
  primaryDisabled: boolean;
  primaryClosesWindow: boolean;
  primaryLabel: string;
}

export interface ReferenceExternalImportFeishuTreeRowModel {
  key: string;
  depth: number;
  canExpand: boolean;
  expanded: boolean;
  title: string;
  pathLabel: string;
  selected: boolean;
  disabled: boolean;
}

export interface ReferenceExternalImportFeishuStepItem {
  key: string;
  label: string;
}

export interface ReferenceExternalImportFeishuWindowModel {
  summary: string;
  steps: ReferenceExternalImportFeishuStepItem[];
  authMode: string;
  authModeOptions: ReferenceExternalImportOption[];
  authDisabled: boolean;
  appId: string;
  appIdPlaceholder: string;
  appSecret: string;
  appSecretPlaceholder: string;
  openBaseUrl: string;
  persistenceMode: string;
  persistenceModeOptions: ReferenceExternalImportOption[];
  showOauthSettings: boolean;
  persistenceHint: string;
  callbackUrls: string[];
  oauthAdminHint: string;
  oauthRedirectHint: string;
  showTest: boolean;
  canTest: boolean;
  testLabel: string;
  authorized: boolean;
  showAuthorize: boolean;
  canAuthorize: boolean;
  authorizeLabel: string;
  canContinueConnection: boolean;
  missingScopesHint: string;
  spaceId: string;
  spaceOptions: ReferenceExternalImportOption[];
  spacePlaceholder: string;
  selectedScopeLabel: string;
  selectedScopeHint: string;
  canUseSpaceRoot: boolean;
  useSpaceRootLabel: string;
  nodeLoading: boolean;
  nodeError: string;
  treeEmptyText: string;
  treeRows: ReferenceExternalImportFeishuTreeRowModel[];
  stageTitle: string;
  progressLabel: string;
  progressRatio: number;
  detail: string;
  rows: ReferenceExternalImportWindowRow[];
  currentItem: string;
  currentItemLabel: string;
  isRunning: boolean;
  waitingForAuthorization: boolean;
  canDelete: boolean;
  canCancelAuthorization: boolean;
  cancelAuthorizationDisabled: boolean;
  cancelAuthorizationLabel: string;
  canCancelImport: boolean;
  cancelImportDisabled: boolean;
  cancelImportLabel: string;
  primaryDisabled: boolean;
  primaryLabel: string;
  deleteLabel: string;
}
