
<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted, watch, nextTick } from "vue";
import { listen } from "@tauri-apps/api/event";
import { cancelChat, chat } from "../services/session";
import { gitExecute } from "../services/git";
import type { UnlistenFn } from "@tauri-apps/api/event";
import type { StreamEvent, ModelOption } from "../types";
import MarkdownRenderer from "./MarkdownRenderer.vue";
import { t } from "../i18n";
import { normalizeAppError } from "../services/errors";
import { useDisplaySettings } from "../composables/useDisplaySettings";

const props = defineProps<{
  workingDir: string;
  selectedModelId: string;
  selectedAgentId: string;
  currentBranch: string;
  models: ModelOption[];
}>();

const emit = defineEmits<{
  (e: "commandDone"): void;
  (e: "workspaceTouched"): void;
  (e: "selectModel", id: string): void;
}>();

const modelDropdownOpen = ref(false);
const modelDropdownRef = ref<HTMLElement | null>(null);
const { state: displaySettings } = useDisplaySettings();

const currentModelName = computed(() => {
  const m = props.models.find(m => m.id === props.selectedModelId);
  return m?.name || "Model";
});

function toggleModelDropdown() {
  modelDropdownOpen.value = !modelDropdownOpen.value;
}

function selectModel(id: string) {
  emit("selectModel", id);
  modelDropdownOpen.value = false;
}

function onModelClickOutside(e: MouseEvent) {
  if (modelDropdownRef.value && !modelDropdownRef.value.contains(e.target as Node)) {
    modelDropdownOpen.value = false;
  }
}

interface ToolExec {
  id: string;
  name: string;
  summary: string;
  output: string;
  status: "running" | "done" | "error" | "interrupted";
  expanded: boolean;
  showFull: boolean;
  rawArgs: string;
}

interface TermLine {
  id: string;
  type: "cmd" | "ai" | "error" | "info" | "tool";
  content: string;
  toolExec?: ToolExec;
}

const lines = ref<TermLine[]>([]);
const input = ref("");
const sessionId = ref<string | null>(null);
const currentRunId = ref<string | null>(null);
const streaming = ref(false);
const streamingText = ref("");
const thinking = ref(false);
const nativeRunning = ref(false);
const scrollEl = ref<HTMLElement | null>(null);
const inputEl = ref<HTMLInputElement | null>(null);
let pendingSessionId: string | null = null;

let activeToolExec: ToolExec | null = null;

const hasRunningTool = computed(() =>
  lines.value.some(l => l.type === "tool" && l.toolExec?.status === "running")
);

const history = ref<string[]>([]);
const historyIdx = ref(-1);
const historyDraft = ref("");

function isNativeCommand(text: string): boolean {
  return /^(git\s|cd\s|ls|dir|pwd|cat\s|echo\s)/.test(text);
}

function extractToolSummary(name: string, argsJson: string): string {
  try {
    const args = JSON.parse(argsJson);
    if (name === "bash") return args.command || "";
    if (name === "read" || name === "write" || name === "edit" || name === "list") {
      return args.filePath || args.file_path || args.path || "";
    }
    if (name === "grep") {
      const pat = args.pattern || "";
      const path = args.filePath || args.file_path || args.path || "";
      if (pat && path) return `/${pat}/ in ${path}`;
      if (pat) return `/${pat}/`;
      return "";
    }
    if (name === "task" || name === "explore") return args.description || "";
    for (const v of Object.values(args)) {
      if (typeof v === "string" && v.length > 0) {
        return v.length <= 80 ? v : v.slice(0, 77) + "...";
      }
    }
  } catch {}
  return "";
}

function shouldRefreshOnToolCompletion(toolName: string): boolean {
  return toolName === "bash" || toolName === "write" || toolName === "edit";
}

async function runNative(text: string) {
  streamingText.value = "";
  thinking.value = false;
  nativeRunning.value = true;
  activeToolExec = null;
  streaming.value = true;
  try {
    const result = await gitExecute(text);
    const output = (result.stdout + result.stderr).trimEnd();
    if (output) {
      lines.value.push({
        id: "out_" + Date.now(),
        type: result.exitCode === 0 ? "info" : "error",
        content: output,
      });
    }
    emit("commandDone");
  } catch (e) {
    lines.value.push({
      id: "err_" + Date.now(),
      type: "error",
      content: normalizeAppError(e).message,
    });
  } finally {
    streaming.value = false;
    nativeRunning.value = false;
    scrollToBottom(true);
  }
}

async function submit() {
  const text = input.value.trim();
  if (!text || streaming.value) return;

  history.value.unshift(text);
  if (history.value.length > 100) history.value.pop();
  historyIdx.value = -1;
  historyDraft.value = "";
  input.value = "";

  lines.value.push({
    id: "cmd_" + Date.now(),
    type: "cmd",
    content: text,
  });
  scrollToBottom(true);

  if (isNativeCommand(text)) {
    await runNative(text);
    return;
  }

  streamingText.value = "";
  streaming.value = true;
  thinking.value = true;
  nativeRunning.value = false;
  pendingSessionId = null;
  activeToolExec = null;

  try {
    const { sessionId: sid, runId } = await chat({
      sessionId: sessionId.value,
      text: text,
      agentId: "git",
      model: props.selectedModelId || null,
      sessionType: "git",
    });
    sessionId.value = sid;
    currentRunId.value = runId;
    pendingSessionId = null;
  } catch (e) {
    streaming.value = false;
    thinking.value = false;
    nativeRunning.value = false;
    lines.value.push({
      id: "err_" + Date.now(),
      type: "error",
      content: normalizeAppError(e).message,
    });
    scrollToBottom();
  }
}

async function cancel() {
  if (!sessionId.value || !streaming.value) return;
  try {
    await cancelChat(sessionId.value);
  } catch (e) {
    console.error("cancel_chat failed:", e);
  }
}

function flushStreamingText() {
  if (streamingText.value) {
    lines.value.push({
      id: "ai_" + Date.now(),
      type: "ai",
      content: streamingText.value,
    });
    streamingText.value = "";
  }
}

function handleStreamEvent(event: StreamEvent) {
  if (event.type === "runStart" && !sessionId.value && streaming.value && pendingSessionId === null) {
    pendingSessionId = event.sessionId;
    sessionId.value = event.sessionId;
  }
  if (event.type === "runStart" && !currentRunId.value && streaming.value) {
    currentRunId.value = event.runId;
  }
  if (event.sessionId !== sessionId.value) return;
  if (currentRunId.value && event.runId !== currentRunId.value) return;

  switch (event.type) {
    case "runStart":
      pendingSessionId = null;
      break;

    case "textDelta":
      thinking.value = false;
      streamingText.value += event.text;
      scrollToBottom();
      break;

    case "toolCallStart": {
      thinking.value = false;
      flushStreamingText();

      const existingTc = findToolExec(event.toolCallId);
      if (existingTc) {
        if (event.arguments) {
          existingTc.rawArgs = event.arguments;
          existingTc.summary = extractToolSummary(event.toolName, event.arguments);
        }
        activeToolExec = existingTc;
        break;
      }

      const summary = extractToolSummary(event.toolName, event.arguments);
      const toolExec: ToolExec = {
        id: event.toolCallId,
        name: event.toolName,
        summary,
        output: "",
        status: "running",
        expanded: event.toolName === "bash",
        showFull: false,
        rawArgs: event.arguments,
      };
      activeToolExec = toolExec;
      lines.value.push({
        id: "tc_" + event.toolCallId,
        type: "tool",
        content: "",
        toolExec,
      });
      scrollToBottom();
      break;
    }

    case "toolCallDelta": {
      if (activeToolExec && activeToolExec.id === event.toolCallId) {
        activeToolExec.output += event.delta;
        activeToolExec.expanded = true;
        scrollToBottom();
      }
      break;
    }

    case "toolCallDone": {
      const tc = findToolExec(event.toolCallId);
      if (tc) {
        tc.status = event.outcome;
        if (!tc.output && event.output) {
          tc.output = event.output;
        }
        if (tc.name === "bash" && tc.output.split("\n").length > 15) {
          tc.expanded = false;
        }
        if (tc.name !== "bash" && tc.output) {
          tc.expanded = false;
        }
      }
      if (shouldRefreshOnToolCompletion(event.toolName)) {
        emit("workspaceTouched");
      }
      activeToolExec = null;
      scrollToBottom();
      break;
    }

    case "toolCallRoundDone":
      if (event.fullText) {
        flushStreamingText();
        const lastAiLine = [...lines.value].reverse().find(l => l.type === "ai");
        if (!lastAiLine || lastAiLine.content !== event.fullText) {
          const hasContent = lines.value.some(l => l.type === "ai" && l.id.startsWith("ai_"));
          if (!hasContent && event.fullText) {
            lines.value.push({ id: event.messageId, type: "ai", content: event.fullText });
          }
        }
      }
      streamingText.value = "";
      thinking.value = true;
      break;

    case "done":
      flushStreamingText();
      if (event.fullText) {
        const lastAi = [...lines.value].reverse().find(l => l.type === "ai");
        if (!lastAi || lastAi.content !== event.fullText) {
          lines.value.push({ id: event.messageId, type: "ai", content: event.fullText });
        }
      }
      streamingText.value = "";
      streaming.value = false;
      thinking.value = false;
      nativeRunning.value = false;
      activeToolExec = null;
      currentRunId.value = null;
      pendingSessionId = null;
      emit("commandDone");
      break;

    case "cancelled":
      streamingText.value = "";
      streaming.value = false;
      thinking.value = false;
      nativeRunning.value = false;
      activeToolExec = null;
      currentRunId.value = null;
      pendingSessionId = null;
      emit("commandDone");
      break;

    case "error":
      streamingText.value = "";
      streaming.value = false;
      thinking.value = false;
      nativeRunning.value = false;
      activeToolExec = null;
      currentRunId.value = null;
      pendingSessionId = null;
      lines.value.push({
        id: "err_" + Date.now(),
        type: "error",
        content: normalizeAppError(event.error).message,
      });
      break;
  }
  scrollToBottom();
}

function findToolExec(toolCallId: string): ToolExec | null {
  for (let i = lines.value.length - 1; i >= 0; i--) {
    const line = lines.value[i];
    if (line.type === "tool" && line.toolExec?.id === toolCallId) {
      return line.toolExec;
    }
  }
  return null;
}

function toggleToolExpand(toolExec: ToolExec) {
  toolExec.expanded = !toolExec.expanded;
}

function truncateOutput(output: string, maxLines: number): { text: string; truncated: boolean; totalLines: number } {
  const outputLines = output.split("\n");
  if (outputLines.length <= maxLines) return { text: output, truncated: false, totalLines: outputLines.length };
  return {
    text: outputLines.slice(0, maxLines).join("\n"),
    truncated: true,
    totalLines: outputLines.length,
  };
}

function handleKeydown(e: KeyboardEvent) {
  if (e.key === "Enter") {
    e.preventDefault();
    submit();
    return;
  }
  if (e.key === "c" && e.ctrlKey) {
    if (streaming.value) {
      e.preventDefault();
      cancel();
      return;
    }
    // Not streaming: if text is selected, let the browser copy it natively
    const sel = window.getSelection();
    if (sel && sel.toString().length > 0) {
      // Don't prevent default – allow native copy
      return;
    }
  }
  if (e.key === "ArrowUp") {
    e.preventDefault();
    if (history.value.length === 0) return;
    if (historyIdx.value === -1) historyDraft.value = input.value;
    if (historyIdx.value < history.value.length - 1) {
      historyIdx.value++;
      input.value = history.value[historyIdx.value];
    }
    return;
  }
  if (e.key === "ArrowDown") {
    e.preventDefault();
    if (historyIdx.value <= 0) {
      historyIdx.value = -1;
      input.value = historyDraft.value;
    } else {
      historyIdx.value--;
      input.value = history.value[historyIdx.value];
    }
    return;
  }
}

/** Whether the user has scrolled away from the bottom */
const userScrolledAway = ref(false);

function onScroll() {
  const el = scrollEl.value;
  if (!el) return;
  const threshold = 40;
  userScrolledAway.value = el.scrollHeight - el.scrollTop - el.clientHeight > threshold;
}

function scrollToBottom(force = false) {
  nextTick(() => {
    const el = scrollEl.value;
    if (!el) return;
    // Only auto-scroll if near bottom or forced (e.g. user submitted a command)
    if (force || !userScrolledAway.value) {
      el.scrollTop = el.scrollHeight;
    }
  });
}

function focusInput() {
  // Don't steal focus if user has selected text (allows copy)
  const sel = window.getSelection();
  if (sel && sel.toString().length > 0) return;
  inputEl.value?.focus();
}

function handleBuiltinOrSubmit() {
  const text = input.value.trim();
  if (text === "clear" || text === "cls") {
    lines.value = [];
    streamingText.value = "";
    input.value = "";
    return;
  }
  submit();
}

function handleTermKeydown(e: KeyboardEvent) {
  // Ctrl+C on the term container: cancel if streaming, otherwise allow native copy
  if (e.key === "c" && e.ctrlKey) {
    if (streaming.value) {
      e.preventDefault();
      cancel();
    }
    // else: let browser handle native copy
  }
}

function handleKeydownWrapped(e: KeyboardEvent) {
  if (e.key === "Enter") {
    e.preventDefault();
    handleBuiltinOrSubmit();
    return;
  }
  handleKeydown(e);
}

let unlisten: UnlistenFn | null = null;
let destroyed = false;

onMounted(async () => {
  const fn = await listen<StreamEvent>("stream-event", (e) => {
    handleStreamEvent(e.payload);
  });
  if (destroyed) {
    fn();
  } else {
    unlisten = fn;
  }
  document.addEventListener("click", onModelClickOutside);
  focusInput();
});

onUnmounted(() => {
  destroyed = true;
  unlisten?.();
  document.removeEventListener("click", onModelClickOutside);
});

watch(
  () => props.workingDir,
  () => {
    lines.value = [];
    sessionId.value = null;
    currentRunId.value = null;
    streamingText.value = "";
    streaming.value = false;
    thinking.value = false;
    nativeRunning.value = false;
    pendingSessionId = null;
    activeToolExec = null;
    history.value = [];
    historyIdx.value = -1;
  },
);

function pushOutput(command: string, output: string, isError = false) {
  lines.value.push({
    id: "ext_cmd_" + Date.now(),
    type: "cmd",
    content: command,
  });
  if (output) {
    lines.value.push({
      id: "ext_out_" + Date.now(),
      type: isError ? "error" : "info",
      content: output,
    });
  }
  scrollToBottom(true);
}

defineExpose({ pushOutput });
</script>

<template>
  <div class="term" @click="focusInput" @keydown="handleTermKeydown">
    <div ref="scrollEl" class="term-output" @scroll="onScroll">
      <template v-for="line in lines" :key="line.id">
        <div v-if="line.type === 'cmd'" class="term-line term-line-cmd">
          <span class="term-prompt ui-select-none">
            <span class="term-model-label">{{ currentModelName }}</span>
            <span class="term-dollar">$</span>
          </span>
          <span class="term-cmd-text ui-select-text">{{ line.content }}</span>
        </div>

        <div v-else-if="line.type === 'ai'" class="term-line term-line-ai">
          <MarkdownRenderer :content="line.content" />
        </div>

        <div v-else-if="line.type === 'tool' && line.toolExec" class="term-line term-tool-inline">
          <div class="tool-line" @click="toggleToolExpand(line.toolExec!)">
            <span class="tool-status-char ui-select-none" :class="'ts-' + line.toolExec.status">{{ line.toolExec.status === 'running' ? '○' : line.toolExec.status === 'done' ? '✓' : '✗' }}</span>
            <span class="tool-label ui-select-text">{{ line.toolExec.name }}</span>
            <span v-if="line.toolExec.summary" class="tool-sum ui-select-text">{{ line.toolExec.summary }}</span>
            <span v-if="line.toolExec.output && line.toolExec.status !== 'running'" class="tool-toggle ui-select-none">{{ line.toolExec.expanded ? '▾' : '▸' }}</span>
          </div>
          <pre v-if="line.toolExec.expanded && line.toolExec.output" class="tool-out ui-select-text" :class="{ 'tool-out-err': line.toolExec.status === 'error' }">{{ line.toolExec.status === 'running' || line.toolExec.showFull ? line.toolExec.output : truncateOutput(line.toolExec.output, 30).text }}<template v-if="!line.toolExec.showFull && line.toolExec.status !== 'running' && truncateOutput(line.toolExec.output, 30).truncated"><span class="tool-out-more ui-select-none" @click.stop="line.toolExec!.showFull = true">
... {{ truncateOutput(line.toolExec.output, 30).totalLines - 30 }} more lines</span></template></pre>
        </div>

        <pre v-else-if="line.type === 'error'" class="term-line term-line-error ui-select-text">{{ line.content }}</pre>

        <pre v-else-if="line.type === 'info'" class="term-line term-line-info ui-select-text">{{ line.content }}</pre>
      </template>

      <div v-if="streamingText" class="term-line term-line-ai term-streaming">
        <MarkdownRenderer :content="streamingText" />
      </div>

      <div v-if="streaming && nativeRunning && !streamingText && !hasRunningTool" class="term-thinking-row">
        <span class="term-status-text">{{ t("tool.status.running") }}</span>
      </div>

      <div v-else-if="streaming && thinking && !streamingText && !hasRunningTool" class="term-thinking-row">
        <span class="thinking-dots">Thinking<span class="dots-anim"></span></span>
        <button class="term-cancel-inline ui-select-none" :title="t('git.cancelTitle')" @click.stop="cancel">
          Ctrl+C <span class="cancel-label">{{ t('git.cancelLabel') }}</span>
        </button>
      </div>

      <div v-if="streaming && !nativeRunning && (streamingText || hasRunningTool)" class="term-thinking-row">
        <button class="term-cancel-inline ui-select-none" :title="t('git.cancelTitle')" @click.stop="cancel">
          Ctrl+C <span class="cancel-label">{{ t('git.cancelLabel') }}</span>
        </button>
      </div>

      <div v-show="!streaming" class="term-input-row">
        <div class="term-prompt-model" ref="modelDropdownRef">
          <span class="term-model-name ui-select-none" @click.stop="toggleModelDropdown">{{ currentModelName }}</span>
          <Transition name="model-dd">
            <div v-if="modelDropdownOpen" class="term-model-dropdown" @click.stop>
              <div
                v-for="m in props.models"
                :key="m.id"
                class="term-model-option"
                :class="{ active: m.id === props.selectedModelId }"
                @click="selectModel(m.id)"
              >{{ m.name }}</div>
            </div>
          </Transition>
        </div>
        <span class="term-dollar ui-select-none">$</span>
        <input
          ref="inputEl"
          v-model="input"
          class="term-input"
          type="text"
          :placeholder="t('git.welcomeHint')"
          spellcheck="false"
          autocomplete="off"
          @keydown="handleKeydownWrapped"
        />
      </div>

      <div v-if="!displaySettings.hideGitCommandSuggestions && lines.length === 0 && !streamingText && !streaming && !input" class="term-examples-inline">
        <span class="term-dim">{{ t("git.examples") }}</span>
        <span class="term-example ui-select-none" @click="input = 'git push'; handleBuiltinOrSubmit()">git push</span>
        <span class="term-example ui-select-none" @click="input = 'git pull'; handleBuiltinOrSubmit()">git pull</span>
        <span class="term-example ui-select-none" @click="input = t('git.exampleCmd'); handleBuiltinOrSubmit()">{{ t("git.exampleCmd") }}</span>
        <span class="term-example ui-select-none" @click="input = 'list branches'; handleBuiltinOrSubmit()">list branches</span>
      </div>
    </div>
  </div>
</template>

<style scoped>
.term {
  display: flex;
  flex-direction: column;
  height: 100%;
  background: var(--git-surface-terminal);
  color: var(--git-text-primary);
  font-family: var(--font-mono-editor);
  font-size: 13px;
  line-height: 1.6;
  cursor: text;
}

.term-output {
  flex: 1;
  overflow-y: auto;
  padding: 8px 12px 8px;
  min-height: 0;
}

.term-output::-webkit-scrollbar { width: 6px; }
.term-output::-webkit-scrollbar-track { background: transparent; }
.term-output::-webkit-scrollbar-thumb { background: var(--git-divider); border-radius: 3px; }

.term-examples-inline {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  align-items: center;
  padding: 4px 0 8px;
}
.term-dim { color: var(--git-text-secondary); }

.term-example {
  padding: 2px 8px;
  border: 1px solid var(--git-divider);
  border-radius: 4px;
  color: var(--git-status-added);
  cursor: pointer;
  transition: all 0.15s;
  font-size: 12px;
}
.term-example:hover {
  background: var(--git-row-hover);
  border-color: var(--git-divider-strong);
  color: var(--git-text-primary);
}

.term-line { margin-bottom: 2px; word-break: break-word; overflow-wrap: break-word; }

.term-line-cmd {
  display: flex;
  align-items: baseline;
  gap: 0;
  padding: 2px 0;
  flex-wrap: wrap;
}

.term-prompt {
  display: inline-flex;
  align-items: baseline;
  gap: 0;
  flex-shrink: 0;
}

.term-model-label { color: var(--git-status-added); font-weight: 500; margin-right: 1px; font-size: 12px; }

.term-prompt-model {
  position: relative;
  flex-shrink: 0;
}

.term-model-name {
  color: var(--git-status-added);
  font-weight: 500;
  font-size: 12px;
  cursor: pointer;
  padding: 1px 4px;
  border-radius: 3px;
  transition: all 0.15s;
}

.term-model-name:hover {
  background: var(--git-row-hover);
  color: var(--git-text-primary);
}

.term-model-dropdown {
  position: absolute;
  top: calc(100% + 4px);
  left: 0;
  min-width: 200px;
  background: var(--git-surface-detail);
  border: 1px solid var(--git-divider-strong);
  border-radius: 8px;
  box-shadow: 0 4px 16px rgba(0, 0, 0, 0.15);
  padding: 4px;
  z-index: 200;
  max-height: 260px;
  overflow-y: auto;
}

:root[data-theme="dark"] .term-model-dropdown { box-shadow: 0 4px 16px rgba(0, 0, 0, 0.5); }

.term-model-option {
  padding: 5px 10px;
  border-radius: 6px;
  cursor: pointer;
  font-size: 12px;
  color: var(--git-text-primary);
  transition: background 0.12s;
  white-space: nowrap;
}

.term-model-option:hover { background: var(--git-row-hover); }
.term-model-option.active { background: var(--git-row-selected); font-weight: 600; }

/* model dropdown transition */
.model-dd-enter-active, .model-dd-leave-active { transition: opacity 0.12s, transform 0.12s; }
.model-dd-enter-from, .model-dd-leave-to { opacity: 0; transform: translateY(-4px); }

.term-dollar { color: var(--git-focus); font-weight: 700; margin-right: 8px; }
.term-cmd-text { color: var(--git-text-primary); font-weight: 500; white-space: pre-wrap; overflow-wrap: break-word; min-width: 0; }

.term-line-ai {
  padding: 2px 0;
  color: var(--git-text-primary);
  font-family: var(--font-prose);
  font-size: 13px;
}
.term-line-ai :deep(p) { margin: 2px 0; }
.term-line-ai :deep(pre) {
  background: var(--git-surface-detail);
  border: 1px solid var(--git-divider);
  border-radius: 6px;
  padding: 8px 10px;
  margin: 4px 0;
  overflow-x: auto;
}
.term-line-ai :deep(code) { font-family: var(--font-mono-inline); font-size: 12px; }
.term-line-ai :deep(pre code) { font-family: var(--font-mono-block); }
.term-line-ai :deep(p code) {
  background: color-mix(in srgb, var(--git-status-modified) 12%, var(--git-surface-detail));
  padding: 1px 4px;
  border-radius: 3px;
  color: var(--git-status-modified);
}
.term-line-ai :deep(ul),
.term-line-ai :deep(ol) { margin: 2px 0; padding-left: 18px; }

.term-streaming { opacity: 0.9; }

.term-line-error {
  color: var(--git-status-deleted);
  padding: 2px 0;
  margin: 0;
  font-family: inherit;
  font-size: inherit;
  white-space: pre-wrap;
  word-break: break-word;
}

.term-line-info {
  color: var(--git-text-primary);
  padding: 2px 0;
  margin: 0;
  font-family: inherit;
  font-size: inherit;
  white-space: pre-wrap;
  word-break: break-word;
}

/* ════════════════════════════════════════
   Tool calls - terminal inline style
   ════════════════════════════════════════ */

.term-tool-inline {
  margin: 1px 0;
}

.tool-line {
  display: flex;
  align-items: baseline;
  gap: 6px;
  cursor: pointer;
  padding: 1px 0;
  font-size: 12px;
  min-width: 0;
}

.tool-line:hover { opacity: 0.85; }

.tool-status-char {
  flex-shrink: 0;
  font-size: 11px;
  font-weight: 700;
}

.tool-status-char.ts-running { color: var(--git-status-conflict); }
.tool-status-char.ts-done { color: var(--git-status-added); }
.tool-status-char.ts-error { color: var(--git-status-deleted); }

.tool-label {
  color: var(--git-text-secondary);
  font-weight: 600;
  flex-shrink: 0;
}

.tool-sum {
  color: var(--git-text-primary);
  opacity: 0.7;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  min-width: 0;
}

.tool-toggle {
  margin-left: auto;
  color: var(--git-text-tertiary);
  opacity: 0.4;
  flex-shrink: 0;
  font-size: 10px;
}

.tool-out {
  font-family: inherit;
  font-size: 12px;
  line-height: 1.5;
  padding: 2px 0 2px 18px;
  margin: 0;
  white-space: pre-wrap;
  word-break: break-word;
  color: var(--git-text-secondary);
  max-height: 300px;
  overflow-y: auto;
}

.tool-out::-webkit-scrollbar { width: 4px; }
.tool-out::-webkit-scrollbar-track { background: transparent; }
.tool-out::-webkit-scrollbar-thumb { background: var(--git-divider); border-radius: 2px; }

.tool-out-err { color: var(--git-status-deleted); }

.tool-out-more {
  color: var(--git-focus);
  cursor: pointer;
  font-style: italic;
  opacity: 0.8;
}

.tool-out-more:hover {
  opacity: 1;
  text-decoration: underline;
}


.term-thinking-row {
  display: flex;
  align-items: center;
  padding: 2px 0;
  gap: 12px;
}

.term-status-text,
.thinking-dots {
  color: var(--git-text-secondary);
  font-size: 13px;
  display: inline-flex;
}

.dots-anim::after {
  content: '...';
  display: inline-block;
  width: 1.05em;
  vertical-align: bottom;
  clip-path: inset(0 100% 0 0);
  animation: dots-clip 1.4s steps(4, end) infinite;
}

@keyframes dots-clip {
  0%   { clip-path: inset(0 100% 0 0); }
  25%  { clip-path: inset(0 66% 0 0); }
  50%  { clip-path: inset(0 33% 0 0); }
  75%  { clip-path: inset(0 0 0 0); }
  100% { clip-path: inset(0 100% 0 0); }
}

.term-cancel-inline {
  background: transparent;
  border: 1px solid var(--git-divider);
  border-radius: 4px;
  color: var(--git-text-secondary);
  font-family: inherit;
  font-size: 11px;
  font-weight: 500;
  padding: 1px 8px;
  cursor: pointer;
  flex-shrink: 0;
  transition: all 0.15s;
}

.term-cancel-inline:hover {
  background: color-mix(in srgb, var(--git-status-deleted) 10%, var(--git-surface-detail));
  border-color: color-mix(in srgb, var(--git-status-deleted) 30%, var(--git-divider));
  color: var(--git-status-deleted);
}

.cancel-label {
  opacity: 0.7;
}

.term-input-row {
  display: flex;
  align-items: center;
  padding: 2px 0;
  gap: 0;
}

.term-input {
  flex: 1;
  background: transparent;
  border: none;
  outline: none;
  color: var(--git-text-primary);
  font-family: inherit;
  font-size: 13px;
  line-height: 1.6;
  caret-color: var(--git-focus);
  padding: 0;
}

.term-input::placeholder { color: var(--git-text-tertiary); opacity: 0.75; }
.term-input:disabled { opacity: 0.5; cursor: not-allowed; }

</style>
