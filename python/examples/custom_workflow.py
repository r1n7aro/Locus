from __future__ import annotations

import asyncio

import locus


async def main() -> None:
    workspace = await locus.get_workspace()
    if workspace.path is None:
        raise RuntimeError("Open a Unity project in Locus before running this workflow")

    models = await locus.list_models()
    if not models:
        raise RuntimeError("No model is currently available in Locus")
    model = next((item for item in models if item.is_default), models[0])

    tools = {tool.name: tool for tool in await locus.list_tools()}
    inspect_tools = [name for name in ("read", "grep", "list") if name in tools]

    # A deterministic workflow step can invoke the shared Locus tool runtime
    # directly, without spending a model turn.
    project_tree = await locus.call_tool(
        "list",
        {"path": ".", "depth": 2, "include_files": True},
        timeout=30,
    )
    project_tree.raise_for_error()

    analyst = locus.Agent(
        name="Implementation analyst",
        id="workflow-analyst",
        system_prompt=(
            "Inspect the requested implementation. Return concise findings with file evidence."
        ),
        tools=inspect_tools,
    )
    tester = locus.Agent(
        name="Test analyst",
        id="workflow-tester",
        system_prompt=(
            "Inspect test coverage and failure risks. Return concrete missing cases with evidence."
        ),
        tools=inspect_tools,
    )
    coordinator = locus.Agent(
        name="Workflow coordinator",
        id="workflow-coordinator",
        system_prompt=(
            "Merge specialist reports into one prioritized implementation plan. Preserve evidence."
        ),
        tools=[],
    )

    context = f"Project tree:\n{project_tree.output}\n\nReview the current project."
    implementation_run, test_run = await asyncio.gather(
        analyst.prompt(context, model=model.id),
        tester.prompt(context, model=model.id),
    )
    implementation, tests = await asyncio.gather(implementation_run.wait(), test_run.wait())
    implementation.raise_for_error()
    tests.raise_for_error()

    final = await coordinator.run(
        "Merge the two reports.\n\n"
        f"Implementation:\n{implementation.text or ''}\n\n"
        f"Tests:\n{tests.text or ''}",
        model=model.id,
    )
    final.raise_for_error()
    print(final.text or "")
    print(f"Locus session: {final.session_id}")

    analyst.close()
    tester.close()
    coordinator.close()


if __name__ == "__main__":
    asyncio.run(main())
