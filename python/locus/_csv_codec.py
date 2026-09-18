"""Lossless CSV field replacement. Untouched records retain their original bytes."""
from __future__ import annotations

import csv
import io
import math
from datetime import date, datetime, time
from decimal import Decimal


def text_value(value):
    if value is None: return ""
    if isinstance(value, str): return value
    if isinstance(value, bool): return "TRUE" if value else "FALSE"
    if isinstance(value, (datetime, date, time)): return value.isoformat()
    if isinstance(value, (int, float, Decimal)):
        if not math.isfinite(value): raise ValueError("CSV cell values must be finite")
        return str(value)
    raise TypeError(f"Unsupported CSV cell value: {type(value).__name__}")


class CsvSource:
    def __init__(self, content: str, delimiter: str):
        self.content, self.delimiter = content, delimiter
        self.bom = "\ufeff" if content.startswith("\ufeff") else ""
        source = content[len(self.bom):]
        # The Rust reader has already validated quoting. Keep raw tokens / each record ending.
        self.records = []
        position = 0
        newline = None
        while position < len(source):
            fields = []
            while True:
                start = position
                if position < len(source) and source[position] == '"':
                    position += 1
                    while position < len(source):
                        if source[position] != '"': position += 1
                        elif source[position:position + 2] == '""': position += 2
                        else:
                            position += 1
                            break
                else:
                    while position < len(source) and source[position] not in delimiter + "\r\n": position += 1
                fields.append(source[start:position])
                if position < len(source) and source[position] == delimiter:
                    position += 1
                    continue
                ending = ""
                if source[position:position + 2] == "\r\n": ending = "\r\n"
                elif position < len(source) and source[position] in "\r\n": ending = source[position]
                position += len(ending)
                if ending and newline is None: newline = ending
                self.records.append((fields, ending))
                break
        self.newline = newline or "\n"
        self.rows = []
        for fields, _ in self.records:
            # csv.reader represents a blank physical line as []; Locus has one empty field.
            row = next(csv.reader(io.StringIO(delimiter.join(fields), newline=""), delimiter=delimiter), [])
            self.rows.append(row or [""])

    def edit(self, values: dict[tuple[int, int], str], *, minimum_rows: int = 0) -> str:
        changes = {key: value for key, value in values.items() if value != self.get(*key) or len(self.rows) < key[0] <= minimum_rows}
        if not changes and minimum_rows <= len(self.records): return self.content
        if any(r < 1 or c < 1 or r > 500000 or c > 10000 for r, c in changes):
            raise ValueError("CSV coordinates exceed worksheet limits")
        records = [(list(fields), ending) for fields, ending in self.records]
        maximum = max(minimum_rows, max((r for r, _ in changes), default=0))
        while len(records) < maximum:
            if records and not records[-1][1]: records[-1] = (records[-1][0], self.newline)
            records.append(([""], ""))
        for (row, column), value in changes.items():
            if "\0" in value: raise ValueError("CSV cannot contain NUL bytes")
            fields = records[row - 1][0]
            fields.extend([""] * max(0, column - len(fields)))
            # Retain explicit quoting when an existing quoted field changes.
            quoted = fields[column - 1].startswith('"') or any(ch in value for ch in self.delimiter + '\r\n"')
            fields[column - 1] = '"' + value.replace('"', '""') + '"' if quoted else value
        if len(records) > len(self.records) and records[-1] == ([""], ""):
            records[-1] = ([""], self.newline)
        return self.bom + "".join(self.delimiter.join(fields) + ending for fields, ending in records)

    def get(self, row: int, column: int) -> str:
        return self.rows[row - 1][column - 1] if row <= len(self.rows) and column <= len(self.rows[row - 1]) else ""
