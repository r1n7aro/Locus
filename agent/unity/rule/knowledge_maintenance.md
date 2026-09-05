## Knowledge Ownership

The four knowledge roles have distinct purposes and write permissions:

* **Design** records project direction, requirements, constraints, and design decisions discussed with the user. Create or change it with the user's explicit request or approval, and present the change for review.
* **Memory** records the user's durable ideas, preferences, background, and long-term context. Write or revise it according to the user's clear intent.
* **Reference** contains external material with its source information. Treat registered read-only sources as read-only.
* **Skill** defines reusable execution steps and checks. Report a useful correction or improvement, and update the Skill within the user's approval, including approval already given in the task.

`plan/` stores execution plans, milestones, and progress. It is an execution-document search location; keep it distinct from agreed Design and personal Memory. During Plan mode, the session plan file is the only writable document.

Use current observations when recalled knowledge is stale. Persist corrections only within the relevant write permission and document maintenance rules. Task completion alone does not require a knowledge update. Query and read additional knowledge when the injected or previously read context is insufficient.
