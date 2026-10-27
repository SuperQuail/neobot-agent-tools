"""协调进程内工作集和缓存记账，不提供操作系统级内存硬限制。"""

from __future__ import annotations

import threading
import time
import weakref
from contextlib import contextmanager
from typing import NamedTuple

MIB = 1024 * 1024
MAX_WORKING_BYTES = 256 * MIB
MAX_CACHED_BYTES = 64 * MIB
MAX_CACHED_ENTRIES = 2048
MAX_ACTIVE_OPERATIONS = 4
MAX_ACTIVE_SEARCHES = 1
ADMISSION_SECONDS = 5.0
SWEEP_SECONDS = 1.0


class FileResourceError(ValueError):
    """资源额度或等待时间不足，调用方可缩小操作范围后重试。"""

    code = "file_busy"


class FileChangedDuringAdmission(FileResourceError):
    """文件在资源准入期间变化，须重新捕获。"""

    code = "file_changed"


def capture_weight(size: int) -> int:
    # 包括原始输入、Unicode 文本、行索引和检索字节，不能只按源文件大小预留额度。
    return 14 * size + 8 * MIB


def search_weight(size: int) -> int:
    # 同时计入快照捕获、回退进程的 JSON 序列化和原生检索输入。
    return 24 * size + 4 * MIB


class _CacheUsage(NamedTuple):
    """保存弱引用及其登记额度，避免用元组下标混淆字节数和条目数。"""

    reference: weakref.ReferenceType
    retained_bytes: int
    entry_count: int


class FileIoResources:
    """统一协调资源准入、缓存额度和唯一的空闲清理线程。"""

    def __init__(
        self,
        *,
        working_bytes=MAX_WORKING_BYTES,
        cached_bytes=MAX_CACHED_BYTES,
        cached_entries=MAX_CACHED_ENTRIES,
        operations=MAX_ACTIVE_OPERATIONS,
        searches=MAX_ACTIVE_SEARCHES,
        sweep_seconds=SWEEP_SECONDS,
    ):
        self.working_limit = working_bytes
        self.cache_limit = cached_bytes
        self.entry_limit = cached_entries
        self.operation_limit = operations
        self.search_limit = searches
        self.sweep_seconds = sweep_seconds
        self._condition = threading.Condition()
        self._working = 0
        self._operations = 0
        self._searches = 0
        self._cached = 0
        self._entries = 0
        self._caches: dict[object, _CacheUsage] = {}
        self._sweeper: threading.Thread | None = None

    @contextmanager
    def operation(self, weight: int, *, search: bool = False, deadline: float):
        if weight < 0 or weight > self.working_limit:
            raise FileResourceError("File operation exceeds the working-memory budget")
        with self._condition:
            self._wait_for_capacity(weight, search=search, deadline=deadline)
            self._working += weight
            self._operations += 1
            self._searches += int(search)
        try:
            yield
        finally:
            with self._condition:
                self._working -= weight
                self._operations -= 1
                self._searches -= int(search)
                self._condition.notify_all()

    def _wait_for_capacity(self, weight: int, *, search: bool, deadline: float) -> None:
        """调用方须持有条件锁；wait 会在等待期间释放锁，醒来后重新检查额度。"""
        while True:
            within_memory = self._working + weight <= self.working_limit
            operation_slot_available = self._operations < self.operation_limit
            search_slot_available = not search or self._searches < self.search_limit
            if within_memory and operation_slot_available and search_slot_available:
                return
            remaining_seconds = deadline - time.monotonic()
            if remaining_seconds <= 0:
                raise FileResourceError("File tools are busy; retry or narrow the operation")
            self._condition.wait(remaining_seconds)

    def register(self, cache) -> object:
        token = object()
        reference = weakref.ref(cache, lambda _: self.unregister(token))
        with self._condition:
            self._caches[token] = _CacheUsage(reference, 0, 0)
        return token

    def unregister(self, token) -> None:
        with self._condition:
            previous = self._caches.pop(token, None)
            if previous:
                self._cached -= previous.retained_bytes
                self._entries -= previous.entry_count
            self._condition.notify_all()

    def retain(self, token, size: int) -> bool:
        with self._condition:
            record = self._caches.get(token)
            if record is None or self._cached + size > self.cache_limit or self._entries >= self.entry_limit:
                return False
            self._cached += size
            self._entries += 1
            self._caches[token] = _CacheUsage(record.reference, record.retained_bytes + size, record.entry_count + 1)
            if self._sweeper is None:
                self._sweeper = threading.Thread(target=self._sweep, name="neobot-file-cache", daemon=True)
                self._sweeper.start()
            return True

    def release(self, token, size: int) -> None:
        with self._condition:
            record = self._caches.get(token)
            if record is None:
                return
            self._cached -= size
            self._entries -= 1
            self._caches[token] = _CacheUsage(record.reference, record.retained_bytes - size, record.entry_count - 1)

    def _sweep(self):
        while True:
            with self._condition:
                if not self._entries:
                    self._sweeper = None
                    return
                self._condition.wait(self.sweep_seconds)
                references = [record.reference for record in self._caches.values()]
            # 删除缓存时先拿缓存锁、再拿协调器锁；这里须先释放协调器锁，避免锁顺序反转。
            for reference in references:
                cache = reference()
                if cache is None:
                    continue
                cache.expire()
                del cache

    def status(self) -> dict:
        with self._condition:
            return {
                "working_bytes": self._working,
                "active_operations": self._operations,
                "active_searches": self._searches,
                "cached_bytes": self._cached,
                "cached_entries": self._entries,
                "registered_caches": len(self._caches),
                "sweeper_alive": self._sweeper is not None,
            }


FILE_IO_RESOURCES = FileIoResources()
