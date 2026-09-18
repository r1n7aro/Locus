"""Bounded history results; reading a page never loads a full Session."""
from __future__ import annotations

from dataclasses import dataclass
from typing import Any

from ._models import SessionMessage


def positive_int(value: int, name: str, maximum: int) -> None:
    if type(value) is not int or not 1 <= value <= maximum:
        raise ValueError(f"{name} must be an integer between 1 and {maximum}")


@dataclass(frozen=True, slots=True)
class SessionMessagePage:
    """Messages in chronological order. Follow oldest_message_row_id backwards.

    Pages keep tool rounds together, so limit is a target size. An empty
    normalized page can still have older history; always use has_more_history.
    """

    messages: tuple[SessionMessage, ...]
    oldest_message_row_id: int | None
    has_more_history: bool

    @classmethod
    def from_payload(cls, payload: dict[str, Any]) -> "SessionMessagePage":
        return cls(
            messages=tuple(SessionMessage.from_payload(row) for row in payload.get("messages", ())),
            oldest_message_row_id=payload.get("oldestMessageRowId"),
            has_more_history=bool(payload.get("hasMoreHistory", False)),
        )


@dataclass(frozen=True, slots=True)
class SessionSearchMatch:
    session_id: str
    session_title: str
    message_id: str | None
    message_row_id: int | None
    role: str | None
    field: str
    excerpt: str

    @classmethod
    def from_payload(cls, payload: dict[str, Any]) -> "SessionSearchMatch":
        return cls(
            session_id=payload["sessionId"],
            session_title=payload.get("sessionTitle", ""),
            message_id=payload.get("messageId"),
            message_row_id=payload.get("messageRowId"),
            role=payload.get("role"),
            field=payload["field"],
            excerpt=payload.get("excerpt", ""),
        )


@dataclass(frozen=True, slots=True)
class SessionSearchPage:
    matches: tuple[SessionSearchMatch, ...]
    next_cursor: str | None
    scanned_messages: int = 0
    scanned_bytes: int = 0

    @property
    def has_more(self) -> bool:
        """More history remains to scan, even if this page has no matches."""
        return self.next_cursor is not None

    @classmethod
    def from_payload(cls, payload: dict[str, Any]) -> "SessionSearchPage":
        return cls(
            matches=tuple(SessionSearchMatch.from_payload(row) for row in payload.get("matches", ())),
            next_cursor=payload.get("nextCursor"),
            scanned_messages=int(payload.get("scannedMessages", 0)),
            scanned_bytes=int(payload.get("scannedBytes", 0)),
        )
