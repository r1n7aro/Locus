from __future__ import annotations

import asyncio
import enum
import inspect
import json
import types
from collections.abc import Callable, Mapping, Sequence
from typing import Any, Literal, Union, get_args, get_origin, get_type_hints


def _annotation_schema(annotation: Any) -> dict[str, Any]:
    if annotation in (inspect.Signature.empty, Any):
        return {}
    if annotation is None or annotation is type(None):
        return {"type": "null"}
    if annotation is str:
        return {"type": "string"}
    if annotation is bool:
        return {"type": "boolean"}
    if annotation is int:
        return {"type": "integer"}
    if annotation is float:
        return {"type": "number"}

    origin = get_origin(annotation)
    arguments = get_args(annotation)
    if origin in (Union, types.UnionType):
        return {"anyOf": [_annotation_schema(argument) for argument in arguments]}
    if origin is Literal:
        values = list(arguments)
        schema: dict[str, Any] = {"enum": values}
        value_types = {type(value) for value in values}
        if len(value_types) == 1:
            schema.update(_annotation_schema(next(iter(value_types))))
        return schema
    if origin in (list, tuple, set, frozenset, Sequence):
        item_type = arguments[0] if arguments else Any
        return {"type": "array", "items": _annotation_schema(item_type)}
    if origin in (dict, Mapping):
        value_type = arguments[1] if len(arguments) > 1 else Any
        return {
            "type": "object",
            "additionalProperties": _annotation_schema(value_type),
        }
    if inspect.isclass(annotation) and issubclass(annotation, enum.Enum):
        return {"enum": [member.value for member in annotation]}
    return {}


def _function_schema(function: Callable[..., Any]) -> dict[str, Any]:
    signature = inspect.signature(function)
    try:
        type_hints = get_type_hints(function, include_extras=True)
    except (NameError, TypeError):
        type_hints = {}
    properties: dict[str, Any] = {}
    required: list[str] = []
    for parameter in signature.parameters.values():
        if parameter.kind in (
            inspect.Parameter.POSITIONAL_ONLY,
            inspect.Parameter.VAR_POSITIONAL,
            inspect.Parameter.VAR_KEYWORD,
        ):
            raise TypeError(
                f"Python tool '{function.__name__}' parameter '{parameter.name}' must be "
                "a named positional-or-keyword or keyword-only parameter"
            )
        annotation = type_hints.get(parameter.name, parameter.annotation)
        schema = _annotation_schema(annotation)
        if parameter.default is not inspect.Parameter.empty:
            try:
                json.dumps(parameter.default)
            except (TypeError, ValueError):
                pass
            else:
                schema = {**schema, "default": parameter.default}
        else:
            required.append(parameter.name)
        properties[parameter.name] = schema
    result: dict[str, Any] = {
        "type": "object",
        "properties": properties,
        "additionalProperties": False,
    }
    if required:
        result["required"] = required
    return result


class Tool:
    """A Python callable exposed to an inline Locus Agent."""

    def __init__(
        self,
        function: Callable[..., Any],
        *,
        name: str | None = None,
        description: str | None = None,
        mutates_workspace: bool = False,
        timeout: float = 120.0,
    ) -> None:
        if not callable(function):
            raise TypeError("Python tool must be callable")
        if timeout <= 0:
            raise ValueError("Python tool timeout must be positive")
        self.function = function
        self.name = (name or function.__name__).strip()
        self.description = (
            description
            if description is not None
            else (inspect.getdoc(function) or f"Run Python function {self.name}.")
        ).strip()
        self.input_schema = _function_schema(function)
        self.mutates_workspace = bool(mutates_workspace)
        self.timeout = float(timeout)

    async def invoke(self, arguments: dict[str, Any]) -> Any:
        if not isinstance(arguments, dict):
            raise TypeError(f"Arguments for Python tool '{self.name}' must be an object")
        bound = inspect.signature(self.function).bind(**arguments)
        bound.apply_defaults()
        if inspect.iscoroutinefunction(self.function):
            return await self.function(*bound.args, **bound.kwargs)
        result = await asyncio.to_thread(self.function, *bound.args, **bound.kwargs)
        if inspect.isawaitable(result):
            return await result
        return result

    def callback_spec(self, callback_key: str) -> dict[str, Any]:
        return {
            "name": self.name,
            "callbackKey": callback_key,
            "description": self.description,
            "inputSchema": self.input_schema,
            "mutatesWorkspace": self.mutates_workspace,
            "timeoutMs": int(self.timeout * 1000),
        }


def tool(
    function: Callable[..., Any] | None = None,
    *,
    name: str | None = None,
    description: str | None = None,
    mutates_workspace: bool = False,
    timeout: float = 120.0,
) -> Tool | Callable[[Callable[..., Any]], Tool]:
    """Decorate a sync or async Python function as a Locus Agent tool."""

    def wrap(target: Callable[..., Any]) -> Tool:
        return Tool(
            target,
            name=name,
            description=description,
            mutates_workspace=mutates_workspace,
            timeout=timeout,
        )

    return wrap(function) if function is not None else wrap
