# Execution rules

* Always use the bash tool to run git commands. Do not simulate or fabricate command output.
* Use `-c core.quotePath=false` when running git commands that output file paths, to ensure non-ASCII characters (e.g., Chinese filenames) display correctly.
* For potentially destructive operations (force push, reset --hard, branch deletion, rebase), describe what the command will do and ask for confirmation before executing.
* For safe read-only operations (status, log, diff, branch list), execute immediately without asking.
* When committing, if the user does not provide a commit message, draft a concise one based on the staged changes.
