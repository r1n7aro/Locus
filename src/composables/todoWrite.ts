import type { TodoItem } from "../types";

const TODO_STATUSES = new Set<TodoItem["status"]>([
  "pending",
  "in_progress",
  "completed",
  "cancelled",
]);

const TODO_PRIORITIES = new Set<TodoItem["priority"]>([
  "high",
  "medium",
  "low",
]);

function normalizeTodoItems(value: unknown): TodoItem[] | null {
  if (!Array.isArray(value)) return null;

  const items: TodoItem[] = [];
  for (const valueItem of value) {
    if (!valueItem || typeof valueItem !== "object" || Array.isArray(valueItem)) continue;
    const item = valueItem as Record<string, unknown>;
    const content = typeof item.content === "string" ? item.content : "";
    if (!content) continue;

    const status = TODO_STATUSES.has(item.status as TodoItem["status"])
      ? item.status as TodoItem["status"]
      : "pending";
    const priority = TODO_PRIORITIES.has(item.priority as TodoItem["priority"])
      ? item.priority as TodoItem["priority"]
      : "medium";

    items.push({ content, status, priority });
  }

  return items;
}

export function parseTodoWriteArguments(argumentsText: string): TodoItem[] | null {
  try {
    const args = JSON.parse(argumentsText) as Record<string, unknown> | null;
    return args && typeof args === "object" && !Array.isArray(args)
      ? normalizeTodoItems(args.todos)
      : null;
  } catch {
    return null;
  }
}

/** Compatibility for tool results created before todowrite stopped echoing its input. */
export function parseLegacyTodoWriteOutput(outputText: string): TodoItem[] | null {
  const jsonStart = outputText.indexOf("[");
  if (jsonStart < 0) return null;

  try {
    return normalizeTodoItems(JSON.parse(outputText.slice(jsonStart)));
  } catch {
    return null;
  }
}
