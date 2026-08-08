---
id: kd_skill_grill
type: skill
path: grill.md
title: Grill
injectMode: excerpt
summaryEnabled: true
commandEnabled: true
readOnly: false
aiMaintained: false
skillEnabled: true
skillSurface: command
commandTrigger: /grill
argumentHint: "[requirement-or-idea]"
tools:
  - ask
  - knowledge_query
  - read
  - list
  - grep
  - code_symbol_search
  - code_find_references
  - unity_asset_search
  - unity_ref_search
  - unity_yaml_list
  - unity_yaml_search
  - unity_yaml_read
createdAt: 1786196077444
updatedAt: 1786196077444
---

# Grill

## Summary
Use when the user invokes `/grill` to be questioned until an implementation requirement is complete enough to build. Focus on the current conversation and repository; leave persistence and implementation to later steps.

## Content
Command arguments: `[requirement-or-idea]` optionally name what should be clarified. When omitted, use the current proposal from the conversation; ask for a target only when no proposal is available.

## Instructions

1. Establish the target.
   - Identify the intended user-visible outcome and the part of the project it affects.
   - Treat the current conversation as the starting context. Do not make code, asset, configuration, or knowledge changes while grilling.

2. Resolve repository facts yourself.
   - Inspect relevant knowledge, code, Unity assets, references, and existing behavior before asking questions whose answers may already be present.
   - Use existing project patterns to turn abstract questions into concrete implementation choices.
   - Ask the user for decisions, preferences, and unavailable product context. Report discovered facts as context for those decisions.

3. Ask focused rounds of questions.
   - Ask the independent questions that are currently answerable together, usually 2–5 per round. Hold dependent questions for the next round.
   - Give each question a short title, the concrete choice, your recommended answer, and the consequence of that recommendation.
   - Follow each answer to its implementation consequences. A broad answer should produce a narrower follow-up when multiple implementations remain valid.
   - When the user delegates a choice, adopt the recommended answer and record it as settled.
   - Skip questions that are already answered, irrelevant to the implementation, or speculative beyond the requested scope.

4. Cover every implementation-affecting dimension that applies.
   - User-visible behavior, actors, triggers, and interaction flow.
   - Scope, explicit exclusions, and compatibility expectations.
   - Existing systems and assets to reuse, plus the responsibilities of any new pieces.
   - Data, state, ownership, lifecycle, persistence, and synchronization.
   - Failure behavior, edge cases, migration, and rollback where relevant.
   - Observable acceptance criteria and the validation path.

   Use these as coverage prompts, not a fixed questionnaire. A dimension is complete when it cannot still lead to materially different valid implementations.

5. Finish at shared implementation clarity.
   - Continue until every material choice is settled, delegated to your recommendation, or explicitly deferred outside the scope.
   - Present a concise `Implementation brief` containing: outcome, behavior, implementation boundaries, settled decisions, acceptance criteria, validation, and explicitly deferred items.
   - End by asking the user to confirm that the brief is ready for implementation. Do not start implementation in the same turn.
