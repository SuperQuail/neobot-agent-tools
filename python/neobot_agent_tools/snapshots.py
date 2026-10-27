"""提供稳定的字节快照接口；授权、文件锁和 owner 观察记录仍由宿主维护。"""

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
    _native = None
    _NATIVE_ERROR = str(exc)


def backend_info() -> dict:
    """报告后端状态，不导入 NeoBot，也不访问文件。"""
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
    """保存有界分页结果和字符位置，供宿主累计自己的读取观察范围。"""

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
    """在 Agent 间按引用共享不可变的 UTF-8 内容和行索引。

    宿主须在构建前预留工作集，并按 ``cost`` 登记缓存额度。
    本接口只处理内存字节，不授予文件系统访问权限。
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
        if backend == "python" or _native is None:
            return cls(SnapshotCore(data), version, "python")
        return cls(_native.Snapshot(data), version, "rust")

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
            rows=tuple(rows),
            start=start,
            end=end,
            next_offset=following,
            next_column=next_column,
            offset=offset,
            column=column,
            total_lines=self.line_count,
            total_chars=self.char_len,
            size=self.size,
            version=self.version,
        )

    def for_search(self):
        if self.search_bytes is not None:
            return self
        return replace(self, search_bytes=self._core.normalized())

    def share(self):
        return replace(self, _core=self._core.share())

    def shares_storage(self, other):
        if not isinstance(other, Snapshot) or self.backend != other.backend:
            return False
        return self._core.shares_storage(other._core)
