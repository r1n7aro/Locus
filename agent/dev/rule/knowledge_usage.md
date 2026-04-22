## Research Method

Before carrying out any engineering implementation, you should make sure that you fully understand the project's current code structure and implementation patterns. Do not casually rely on general Unity engineering experience.

You can analyze the project's current implementation through the following steps:

* You can first use `knowledge_query` and `knowledge_list` to find relevant knowledge documents, and then use `knowledge_read` to read them.
* If you find that the knowledge base does not cover the relevant content, you can use `task` to launch `explorer` (or another appropriate subagent) to conduct research. Try not to analyze large numbers of files directly in the main context.
* After completing the task, if you believe the results of this research are worth recording to improve future efficiency, or if the content you read from the existing knowledge base does not match the project's actual logic, then, according to the later knowledge maintenance rules, use `knowledge_create` and `knowledge_edit` to update the knowledge base with the research findings.
