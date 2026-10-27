"""度量已安装 release wheel 的内存快照接口，不包含文件系统或宿主授权耗时。"""

import argparse
import hashlib
import json
import platform
import statistics
import time

from neobot_agent_tools import Snapshot, backend_info


def measure(operation, *, samples):
    """预热后记录中位数；保留真实的 Python 结果构造成本。"""
    operation()
    timings = []
    for _ in range(samples):
        started = time.perf_counter()
        result = operation()
        timings.append((time.perf_counter() - started) * 1000)
        del result
    return {"median_ms": statistics.median(timings), "min_ms": min(timings), "max_ms": max(timings)}


def read_all(snapshot):
    """沿真实续读行列获取全文，用于验证分页性能与完整性。"""
    args = {"limit": 2000}
    parts = []
    while True:
        page = snapshot.read_page(**args)
        parts.append(page.as_dict()["content"])
        if page.next_offset is None:
            return "".join(parts)
        args.update(offset=page.next_offset, column=page.next_column)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--samples", type=int, default=21)
    args = parser.parse_args()
    if args.samples < 3:
        parser.error("至少使用 3 次采样")
    info = backend_info()
    if info["backend"] != "rust":
        raise SystemExit(f"此基准要求实际加载 Rust 后端：{info}")
    text = "".join(f"def function_0_{n:05d}(value): return value + {n}  # 汉字注释\r\n" for n in range(60000))
    data = text.encode()
    dense_data = b"line\n" * 800000
    long_text = "汉😀" * 300000
    long_data = long_text.encode()
    source = Snapshot.from_bytes(data, "benchmark", backend="rust")
    dense = Snapshot.from_bytes(dense_data, "benchmark", backend="rust")
    long_line = Snapshot.from_bytes(long_data, "benchmark", backend="rust")
    assert read_all(source) == text
    assert read_all(long_line) == long_text
    normalized = ("\n".join(text.splitlines()) + "\n").encode()
    assert source.for_search().search_bytes == normalized
    assert source.shares_storage(source.share())
    operations = {
        "source_create": lambda: Snapshot.from_bytes(data, "benchmark", backend="rust"),
        "dense_ascii_create": lambda: Snapshot.from_bytes(dense_data, "benchmark", backend="rust"),
        "long_unicode_create": lambda: Snapshot.from_bytes(long_data, "benchmark", backend="rust"),
        "source_full_pagination": lambda: read_all(source),
        "long_unicode_full_pagination": lambda: read_all(long_line),
        "source_search_preparation": source.for_search,
    }
    result = {
        "python": platform.python_version(),
        "backend": info,
        "samples": args.samples,
        "source_bytes": len(data),
        "source_sha256": hashlib.sha256(data).hexdigest(),
        "scope": "已安装 release wheel 的内存接口，含 Python 结果构造，不含磁盘及宿主授权",
        "cost_bytes": {"source": source.cost, "dense_ascii": dense.cost, "long_unicode": long_line.cost},
        "benchmarks": {name: measure(operation, samples=args.samples) for name, operation in operations.items()},
    }
    print(json.dumps(result, ensure_ascii=False))


if __name__ == "__main__":
    main()
