"""Portable implementation of the native snapshot contract."""

import sys
from array import array
from itertools import accumulate, islice

BREAKS = "\n\r\v\f\x1c\x1d\x1e\x85\u2028\u2029"
ASCII_BREAKS = bytes.maketrans(b"\r\v\f\x1c\x1d\x1e", b"\n" * 6)


class SnapshotCore:
    def __init__(self, data: bytes):
        self.text = data.decode("utf-8")
        self.size, self.char_len = len(data), len(self.text)
        self._starts, position = array("I", [0]), 0
        while position < self.char_len:
            end = min(position + 65536, self.char_len)
            if end < self.char_len and self.text[end - 1 : end + 1] == "\r\n":
                end += 1
            chunk = self.text[position:end]
            self._starts.extend(
                islice(accumulate(map(len, chunk.splitlines(keepends=True)), initial=position), 1, None)
            )
            if end < self.char_len and chunk[-1] not in BREAKS:
                self._starts.pop()
            position = end

    @property
    def line_count(self):
        return len(self._starts) - 1

    @property
    def memory_size(self):
        return sys.getsizeof(self) + sys.getsizeof(self.text) + sys.getsizeof(self._starts)

    def start(self, index):
        return self._starts[min(index, self.line_count)]

    def line(self, index, *, keepends=True):
        value = self.text[self._starts[index] : self._starts[index + 1]]
        if not keepends:
            if value.endswith("\r\n"):
                return value[:-2]
            if value and value[-1] in BREAKS:
                return value[:-1]
        return value

    def read_fragment(self, index, column, char_limit):
        start, end = self._starts[index] + column, self._starts[index + 1]
        if start > end:
            raise ValueError("column exceeds line length")
        return self.text[start : min(end, start + char_limit)], end - start

    def read_page(self, offset, limit, column, budget):
        if column and (offset > self.line_count or column > self.start(offset) - self.start(offset - 1)):
            raise ValueError("column exceeds line length")
        start = self.start(offset - 1) + column
        position, remaining, rows = start, budget, []
        next_offset = next_column = None
        for index in range(offset - 1, min(self.line_count, offset - 1 + limit)):
            begin = column if index == offset - 1 else 0
            fragment, length = self.read_fragment(index, begin, remaining)
            chunk = fragment.encode("utf-8")[:remaining].decode("utf-8", errors="ignore")
            if not chunk and length:
                next_offset, next_column = index + 1, begin
                break
            rows.append((index + 1, chunk, begin))
            position += len(chunk)
            remaining -= len(chunk.encode("utf-8"))
            if len(chunk) < length:
                next_offset, next_column = index + 1, begin + len(chunk)
                break
            if index + 1 < self.line_count:
                next_offset, next_column = index + 2, 0
            else:
                next_offset = next_column = None
        return rows, start, position, next_offset, next_column

    def normalized(self):
        data = self.text.encode("utf-8").replace(b"\r\n", b"\n").translate(ASCII_BREAKS)
        for separator in (b"\xc2\x85", b"\xe2\x80\xa8", b"\xe2\x80\xa9"):
            data = data.replace(separator, b"\n")
        return data + b"\n" if data and not data.endswith(b"\n") else data

    def share(self):
        return self

    def shares_storage(self, other):
        return self is other
