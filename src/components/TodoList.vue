<script setup lang="ts">
import type { TodoItem } from "../types";

withDefaults(defineProps<{
  todos: TodoItem[];
  emptyText: string;
  compact?: boolean;
}>(), {
  compact: false,
});

const priorityLabel: Record<TodoItem["priority"], string> = {
  high: "H",
  medium: "M",
  low: "L",
};
</script>

<template>
  <div class="todo-list" :class="{ compact }">
    <div
      v-for="(todo, index) in todos"
      :key="`${index}:${todo.content}`"
      class="todo-item"
      :class="`status-${todo.status}`"
    >
      <span class="todo-status" :title="todo.status" aria-hidden="true"></span>
      <span class="todo-content">{{ todo.content }}</span>
      <span
        class="todo-priority"
        :class="`priority-${todo.priority}`"
        :title="todo.priority"
      >{{ priorityLabel[todo.priority] }}</span>
    </div>
    <div v-if="todos.length === 0" class="empty-hint">{{ emptyText }}</div>
  </div>
</template>

<style scoped>
.todo-list {
  flex: 1;
  overflow-y: auto;
  padding: 8px;
}

.todo-list.compact {
  padding: 2px 0;
}

.todo-item {
  display: flex;
  align-items: flex-start;
  gap: 8px;
  padding: 8px 10px;
  border-radius: 6px;
  margin-bottom: 2px;
  transition: background 0.15s;
}

.compact .todo-item {
  padding: 6px 8px;
}

.todo-item:hover {
  background: var(--hover-bg);
}

.todo-status {
  flex-shrink: 0;
  width: 14px;
  min-width: 14px;
  height: 20px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  color: var(--text-secondary);
}

.todo-status::before {
  content: "";
  width: 7px;
  height: 7px;
  border: 1px solid currentColor;
  border-radius: 999px;
  box-sizing: border-box;
}

.status-pending .todo-status {
  color: color-mix(in srgb, var(--text-secondary) 72%, transparent);
}

.status-in_progress .todo-status {
  color: var(--accent-color);
}

.status-in_progress .todo-status::before {
  background: linear-gradient(90deg, currentColor 50%, transparent 50%);
}

.status-completed .todo-status {
  color: var(--status-good-fg);
}

.status-completed .todo-status::before {
  background: currentColor;
}

.status-cancelled .todo-status {
  color: color-mix(in srgb, var(--text-secondary) 70%, transparent);
  opacity: 0.5;
}

.status-cancelled .todo-status::before {
  width: 8px;
  height: 2px;
  border: 0;
  border-radius: 999px;
  background: currentColor;
}

.todo-content {
  flex: 1;
  min-width: 0;
  color: var(--text-color);
  font-size: 13px;
  line-height: 20px;
  word-break: break-word;
}

.status-completed .todo-content {
  text-decoration: line-through;
  opacity: 0.6;
}

.status-cancelled .todo-content {
  text-decoration: line-through;
  opacity: 0.4;
}

.todo-priority {
  flex-shrink: 0;
  width: 18px;
  height: 18px;
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: 3px;
  font-size: 10px;
  font-weight: 600;
  line-height: 1;
}

.priority-high {
  background: var(--status-danger-bg);
  color: var(--status-danger-fg);
}

.priority-medium {
  background: var(--status-warn-bg);
  color: var(--status-warn-fg);
}

.priority-low {
  background: color-mix(in srgb, var(--text-secondary) 10%, transparent);
  color: var(--text-secondary);
}

.empty-hint {
  padding: 24px 0;
  color: var(--text-secondary);
  font-size: 13px;
  text-align: center;
}

.compact .empty-hint {
  padding: 12px 8px;
  text-align: left;
}
</style>
