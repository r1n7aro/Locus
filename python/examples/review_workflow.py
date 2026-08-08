from __future__ import annotations

import asyncio

import locus


async def main() -> None:
    available_tools = {tool.name for tool in await locus.list_tools()}
    reviewer_tools = [name for name in ("read", "grep", "list") if name in available_tools]

    @locus.tool
    async def review_policy() -> dict[str, object]:
        """Return the project-specific review policy."""
        return {"require_tests": True, "report_order": "severity"}

    reviewer = locus.Agent(
        name="Reviewer",
        id="reviewer",
        description="Review project code and report concrete risks.",
        system_prompt=(
            "You are a focused code reviewer. Inspect relevant files, cite concrete evidence, "
            "and return findings ordered by severity."
        ),
        tools=[*reviewer_tools, review_policy],
    )

    run = await reviewer.prompt("Review the current implementation for correctness risks.")
    result = await run
    print(result.text or "")

    follow_up = await reviewer.run("Continue in the same session and check test coverage.")
    print(follow_up.text or "")


if __name__ == "__main__":
    asyncio.run(main())
