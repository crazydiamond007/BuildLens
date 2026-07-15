import io
import re
import zipfile
from dataclasses import dataclass
from typing import Any

SECRET_PATTERNS = (
    re.compile(r"(?i)(authorization\s*:\s*(?:bearer|basic)\s+)[^\s]+"),
    re.compile(r"(?i)\b(gh[pousr]_[A-Za-z0-9_]{20,})\b"),
    re.compile(r"(?i)\b(AKIA[0-9A-Z]{16})\b"),
    re.compile(
        r"(?i)\b(password|passwd|secret|token|api[_-]?key|access[_-]?key)"
        r"(\s*[:=]\s*)[^\s,;]+"
    ),
    re.compile(r"(?i)(https?://[^\s:/]+:)[^@\s]+(@)"),
    re.compile(r"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}\b"),
)
ERROR_MARKERS = re.compile(
    r"(?i)(\berror\b|\bfail(?:ed|ure)?\b|exception|traceback|panic|fatal|assertion)"
)
MAX_ARCHIVE_ENTRIES = 250
MAX_ENTRY_BYTES = 2 * 1024 * 1024
MAX_UNCOMPRESSED_BYTES = 8 * 1024 * 1024
CONTEXT_LINES = 2


@dataclass(frozen=True)
class ExtractedRange:
    source: str
    start_line: int
    end_line: int
    text: str


def redact(text: str) -> str:
    value = text
    for index, pattern in enumerate(SECRET_PATTERNS):
        if index == 0:
            value = pattern.sub(r"\1[REDACTED]", value)
        elif index == 3:
            value = pattern.sub(r"\1\2[REDACTED]", value)
        elif index == 4:
            value = pattern.sub(r"\1[REDACTED]\2", value)
        elif index == 5:
            value = pattern.sub("[REDACTED_EMAIL]", value)
        else:
            value = pattern.sub("[REDACTED]", value)
    return value


def redact_data(value: Any) -> Any:
    """Redact every string in a JSON-like value before it leaves the worker."""
    if isinstance(value, str):
        return redact(value)
    if isinstance(value, dict):
        return {key: redact_data(item) for key, item in value.items()}
    if isinstance(value, list):
        return [redact_data(item) for item in value]
    if isinstance(value, tuple):
        return tuple(redact_data(item) for item in value)
    return value


def extract_failure_ranges(
    archive_bytes: bytes,
    *,
    max_lines: int,
    max_prompt_bytes: int,
) -> list[ExtractedRange]:
    ranges: list[ExtractedRange] = []
    consumed_lines = 0
    consumed_bytes = 0
    uncompressed_bytes = 0
    with zipfile.ZipFile(io.BytesIO(archive_bytes)) as archive:
        infos = archive.infolist()
        if len(infos) > MAX_ARCHIVE_ENTRIES:
            raise ValueError("log archive contains too many entries")
        for info in infos:
            if info.is_dir() or info.file_size > MAX_ENTRY_BYTES:
                continue
            if _unsafe_path(info.filename):
                continue
            uncompressed_bytes += info.file_size
            if uncompressed_bytes > MAX_UNCOMPRESSED_BYTES:
                raise ValueError("log archive contains too much uncompressed data")
            raw = archive.read(info)
            lines = raw.decode("utf-8", errors="replace").splitlines()
            for start, end in _matching_windows(lines):
                selected = "\n".join(redact(line) for line in lines[start:end])
                selected_bytes = len(selected.encode("utf-8"))
                selected_lines = end - start
                if consumed_lines + selected_lines > max_lines:
                    return ranges
                if consumed_bytes + selected_bytes > max_prompt_bytes:
                    return ranges
                ranges.append(
                    ExtractedRange(
                        source=info.filename,
                        start_line=start + 1,
                        end_line=end,
                        text=selected,
                    )
                )
                consumed_lines += selected_lines
                consumed_bytes += selected_bytes
    return ranges


def _matching_windows(lines: list[str]) -> list[tuple[int, int]]:
    raw_windows = [
        (max(0, index - CONTEXT_LINES), min(len(lines), index + CONTEXT_LINES + 1))
        for index, line in enumerate(lines)
        if ERROR_MARKERS.search(line)
    ]
    merged: list[tuple[int, int]] = []
    for start, end in raw_windows:
        if merged and start <= merged[-1][1]:
            merged[-1] = (merged[-1][0], max(merged[-1][1], end))
        else:
            merged.append((start, end))
    return merged


def _unsafe_path(name: str) -> bool:
    normalized = name.replace("\\", "/")
    return normalized.startswith("/") or ".." in normalized.split("/")
