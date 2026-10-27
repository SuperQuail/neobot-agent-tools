"""Process-wide admission and cache accounting; never an OS/RSS memory sandbox."""

from __future__ import annotations

import threading
import time
import weakref
from contextlib import contextmanager

MIB = 1024 * 1024
MAX_WORKING_BYTES = 256 * MIB
MAX_CACHED_BYTES = 64 * MIB
MAX_CACHED_ENTRIES = 2048
MAX_ACTIVE_OPERATIONS = 4
MAX_ACTIVE_SEARCHES = 1
ADMISSION_SECONDS = 5.0
SWEEP_SECONDS = 1.0


class FileResourceError(ValueError):
    code = "file_busy"


class FileChangedDuringAdmission(FileResourceError):
    code = "file_changed"


def capture_weight(size: int) -> int:
    # Raw input, widened Unicode, packed index, normalized input and bounded
    # indexing chunks. This deliberately reserves more than just source bytes.
    return 14 * size + 8 * MIB


def search_weight(size: int) -> int:
    # Includes snapshot capture, JSON fallback serialization and native input.
    return 24 * size + 4 * MIB


class FileIoResources:
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
        self.working_limit, self.cache_limit, self.entry_limit = working_bytes, cached_bytes, cached_entries
        self.operation_limit, self.search_limit = operations, searches
        self.sweep_seconds = sweep_seconds
        self._condition = threading.Condition()
        self._working = self._operations = self._searches = 0
        self._cached = self._entries = 0
        self._caches: dict[object, tuple[weakref.ReferenceType, int, int]] = {}
        self._sweeper: threading.Thread | None = None

    @contextmanager
    def operation(self, weight: int, *, search: bool = False, deadline: float):
        if weight < 0 or weight > self.working_limit:
            raise FileResourceError("File operation exceeds the working-memory budget")
        with self._condition:
            while (
                self._working + weight > self.working_limit
                or self._operations >= self.operation_limit
                or (search and self._searches >= self.search_limit)
            ):
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise FileResourceError("File tools are busy; retry or narrow the operation")
                self._condition.wait(remaining)
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

    def register(self, cache) -> object:
        token = object()
        reference = weakref.ref(cache, lambda _: self.unregister(token))
        with self._condition:
            self._caches[token] = (reference, 0, 0)
        return token

    def unregister(self, token) -> None:
        with self._condition:
            previous = self._caches.pop(token, None)
            if previous:
                self._cached -= previous[1]
                self._entries -= previous[2]
            self._condition.notify_all()

    def retain(self, token, size: int) -> bool:
        with self._condition:
            record = self._caches.get(token)
            if record is None or self._cached + size > self.cache_limit or self._entries >= self.entry_limit:
                return False
            self._cached += size
            self._entries += 1
            self._caches[token] = (record[0], record[1] + size, record[2] + 1)
            if self._sweeper is None:
                self._sweeper = threading.Thread(target=self._sweep, name="neobot-file-cache", daemon=True)
                self._sweeper.start()
            return True

    def release(self, token, size: int) -> None:
        with self._condition:
            record = self._caches.get(token)
            if record is not None:
                self._cached -= size
                self._entries -= 1
                self._caches[token] = (record[0], record[1] - size, record[2] - 1)

    def _sweep(self):
        while True:
            with self._condition:
                if not self._entries:
                    self._sweeper = None
                    return
                self._condition.wait(self.sweep_seconds)
                references = [record[0] for record in self._caches.values()]
            # Never hold the coordinator lock while taking an individual cache's
            # lock. Cache removals acquire these two locks in the reverse order.
            for reference in references:
                cache = reference()
                if cache is not None:
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
