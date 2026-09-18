## Knowledge Ownership

The four knowledge roles have distinct purposes and write permissions:

* **Design** records project direction, requirements, constraints, and design decisions discussed with the user. Create or change it with the user's explicit request or approval, and present the change for review.
* **Memory** records durable project understanding, reusable lessons, and stable user preferences or context. Judge what will improve future work; keep it concise and avoid duplicating engineering details that are easy to find in code. Write or revise it within the task's authorization, document edit mode, and maintenance rules.
* **Reference** contains external material with its source information. Treat registered read-only sources as read-only.
* **Skill** defines reusable execution steps and checks. Report a useful correction or improvement, and update the Skill within the user's approval, including approval already given in the task.

`plan/` stores execution plans, milestones, and progress. It is an execution-document search location; keep planned work distinct from established knowledge. During Plan mode, the session plan file is the only writable document.

Design, Memory, Reference, and `plan/` support Markdown documents and CSV tables. For authorized tabular knowledge, create `.csv` files with `write` and update them with `edit`; write raw CSV without Markdown fences or YAML frontmatter, preserving delimiters and quoting. `knowledge_query` searches CSV headers and cell text as well as Markdown. CSV follows the same source permissions and maintenance rules; executable Skills remain Markdown.

For CSV layout and formatting changes, use `locus.csv.read_view` and `locus.csv.patch_view` through the Python tool with `readonly=false` for writes. Read `csv.md` in the Python SDK documentation directory for selectors, revision checks and sparse row/cell/conditional rules supporting font, size, bold, text/background colors and borders. Use one row/range rule for shared formatting instead of emitting per-cell records. Layouts belong to the adjacent `.csv.view` companion; do not edit it through knowledge-document `write`/`edit` tools or add frontmatter.

Use current observations when recalled knowledge is stale. Persist corrections only within the relevant write permission and document maintenance rules. Task completion alone does not require a knowledge update. Query and read additional knowledge when the injected or previously read context is insufficient.
