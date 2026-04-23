import { computed, type Ref } from "vue";
import type { SkillManifest } from "../types";
import type { CommandDef } from "./chatInputIntents";
import { rankSearchResults, scoreSearchFields, splitSearchTerms } from "./searchMatcher";
import { resolveSkillCommandTrigger, skillHasCommandEnabled } from "./skillCommands";

export const COMPACT_INSTRUCTION =
  "请对上面的对话进行总结和压缩：保留关键的技术决策、代码变更、未完成任务和重要上下文，删除冗余的中间过程。用精简的格式输出总结，以便后续对话可以基于这个总结继续。";

export function useCommandRegistry(
  skills: Ref<SkillManifest[] | undefined>,
  agentId: Ref<string>,
) {
  const commands = computed<CommandDef[]>(() => {
    const builtins: CommandDef[] = [
      {
        name: "/clear",
        description: "清空当前会话并开始新对话",
        commandKind: "action",
        commandType: "clear",
      },
      {
        name: "/compact",
        description: "压缩当前上下文并总结历史消息",
        commandKind: "action",
        commandType: "compact",
      },
      {
        name: "/plan",
        description: "切换到规划模式",
        commandKind: "intent",
        commandType: "plan",
        agentOnly: "dev",
      },
    ];

    const skillCommands: CommandDef[] = (skills.value || [])
      .filter((skill) => skillHasCommandEnabled(skill))
      .map((skill) => ({
        name: resolveSkillCommandTrigger(skill),
        description: skill.skillDescription || skill.description || skill.name,
        commandKind: "intent",
        commandType: "skill",
        skill: {
          dirName: skill.dirName,
          source: skill.source,
          name: skill.name,
        },
        argumentHint: skill.argumentHint,
      }));

    return [...builtins, ...skillCommands];
  });

  const availableCommands = computed(() =>
    commands.value.filter((command) => !command.agentOnly || command.agentOnly === agentId.value),
  );

  function filteredCommands(
    token: string,
    options?: { includeActions?: boolean },
  ): CommandDef[] {
    const normalized = token.trim().toLowerCase();
    if (!normalized.startsWith("/")) return [];
    if (normalized !== "/" && splitSearchTerms(normalized).length === 0) return [];

    const candidates = availableCommands.value.filter((command) => {
      if (!options?.includeActions && command.commandKind === "action") return false;
      return scoreSearchFields(normalized, [
        { text: command.name, weight: 160 },
        { text: command.skill?.name || "", weight: 120 },
        { text: command.skill?.dirName || "", weight: 120 },
        { text: command.description, weight: 40 },
        { text: command.argumentHint || "", weight: 10 },
      ]) != null;
    });

    return rankSearchResults(candidates, normalized, (command) => [
      { text: command.name, weight: 160 },
      { text: command.skill?.name || "", weight: 120 },
      { text: command.skill?.dirName || "", weight: 120 },
      { text: command.description, weight: 40 },
      { text: command.argumentHint || "", weight: 10 },
    ]);
  }

  function findExactAvailableCommand(token: string): CommandDef | null {
    const normalized = token.trim().toLowerCase();
    return availableCommands.value.find((command) => command.name.toLowerCase() === normalized) ?? null;
  }

  function findExactCommand(token: string): CommandDef | null {
    const normalized = token.trim().toLowerCase();
    return commands.value.find((command) => command.name.toLowerCase() === normalized) ?? null;
  }

  return {
    commands: availableCommands,
    allCommands: commands,
    filteredCommands,
    findExactAvailableCommand,
    findExactCommand,
  };
}
