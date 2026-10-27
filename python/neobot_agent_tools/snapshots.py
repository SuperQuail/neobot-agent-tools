"""Stable buffer API. Authorization, file locks and observations belong to the host."""

import sys
from dataclasses import dataclass, replace

from ._fallback import SnapshotCore
from ._version import API_VERSION, __version__

MAX_FILE_BYTES = 16 * 1024 * 1024
MAX_TEXT_BYTES = 65536
_NATIVE_ERROR = None
try:
    from . import _native

    if _native.API_VERSION != API_VERSION or _native.VERSION != __version__:
        raise ImportError("Native extension version does not match its Python facade")
except (ImportError, OSError) as exc:
    _native, _NATIVE_ERROR = None, str(exc)


def backend_info() -> dict:
    """Describe the available backend without importing NeoBot or accessing files."""
    return {
        "backend": "rust" if _native is not None else "python",
        "version": __version__,
        "api_version": API_VERSION,
        "native_error": _NATIVE_ERROR,
    }


def _integer(value, name, minimum, maximum):
    if isinstance(value, bool) or not isinstance(value, int) or not minimum <= value <= maximum:
        raise ValueError(f"{name} must be {minimum}..{maximum}")


@dataclass(frozen=True)
class ReadPage:
    """Bounded output plus character positions for host-owned observation tracking."""

    rows: tuple
    start: int
    end: int
    next_offset: int | None
    next_column: int | None
    offset: int
    column: int
    total_lines: int
    total_chars: int
    size: int
    version: str

    def as_dict(self) -> dict:
        return {
            "ok": True,
            "version": self.version,
            "size": self.size,
            "offset": self.offset,
            "column": self.column,
            "lines": [{"number": number, "text": text, "column": column} for number, text, column in self.rows],
            "content": "".join(row[1] for row in self.rows),
            "totalLines": self.total_lines,
            "truncated": self.end < self.total_chars,
            "next_offset": self.next_offset,
            "next_column": self.next_column,
        }


@dataclass(frozen=True)
class Snapshot:
    """Immutable UTF-8 buffer and line index shared by reference between agents.

    The host reserves working memory before construction and retains cache quota
    for ``cost``. This low-level buffer API does not grant filesystem authority.
    """

    _core: object
    version: str
    backend: str
    search_bytes: bytes | None = None

    @classmethod
    def from_bytes(cls, data: bytes, version: str, *, backend="auto"):
        if not isinstance(data, bytes) or not isinstance(version, str):
            raise TypeError("data must be bytes and version must be a string")
        if len(data) > MAX_FILE_BYTES:
            raise ValueError(f"Maximum input size is {MAX_FILE_BYTES} bytes")
        if backend not in {"auto", "rust", "python"}:
            raise ValueError("backend must be auto, rust or python")
        if backend == "rust" and _native is None:
            raise RuntimeError(f"Native backend unavailable: {_NATIVE_ERROR}")
        use_native = backend != "python" and _native is not None
        core = _native.Snapshot(data) if use_native else SnapshotCore(data)
        return cls(core, version, "rust" if use_native else "python")

    @property
    def text(self):
        return self._core.text

    @property
    def size(self):
        return self._core.size

    @property
    def char_len(self):
        return self._core.char_len

    @property
    def line_count(self):
        return self._core.line_count

    @property
    def cost(self):
        return (
            sys.getsizeof(self)
            + self._core.memory_size
            + sys.getsizeof(self.version)
            + sys.getsizeof(self.search_bytes)
        )

    def start(self, index):
        _integer(index, "index", 0, MAX_FILE_BYTES + 1)
        return self._core.start(index)

    def line(self, index, *, keepends=True):
        _integer(index, "index", 0, max(0, self.line_count - 1))
        if index >= self.line_count:
            raise IndexError("Line index is outside the snapshot")
        return self._core.line(index, keepends=keepends)

    def read_fragment(self, index, column, char_limit):
        _integer(index, "index", 0, max(0, self.line_count - 1))
        _integer(column, "column", 0, MAX_FILE_BYTES)
        _integer(char_limit, "char_limit", 0, MAX_TEXT_BYTES)
        if index >= self.line_count:
            raise IndexError("Line index is outside the snapshot")
        return self._core.read_fragment(index, column, char_limit)

    def read_page(self, *, offset=1, limit=2000, column=0, max_bytes=MAX_TEXT_BYTES):
        _integer(offset, "offset", 1, MAX_FILE_BYTES + 1)
        _integer(limit, "limit", 1, 2000)
        _integer(column, "column", 0, MAX_FILE_BYTES)
        _integer(max_bytes, "max_bytes", 0, MAX_TEXT_BYTES)
        rows, start, end, following, next_column = self._core.read_page(offset, limit, column, max_bytes)
        return ReadPage(
            tuple(rows),
            start,
            end,
            following,
            next_column,
            offset,
            column,
            self.line_count,
            self.char_len,
            self.size,
            self.version,
        )

    def for_search(self):
        return self if self.search_bytes is not None else replace(self, search_bytes=self._core.normalized())

    def share(self):
        return replace(self, _core=self._core.share())

    def shares_storage(self, other):
        return isinstance(other, Snapshot) and self.backend == other.backend and self._core.shares_storage(other._core)
