"""独立度量 release wheel 的 PTC 值内核，不依赖 NeoBot 解释器。"""

import argparse
import json
import platform
import statistics
import time

from neobot_agent_tools import __version__
from neobot_agent_tools.ptc_values import PtcValueKernel, native_available

LIMITS = {"nodes": 16_000, "depth": 32, "items": 4_000, "string_chars": 32_768, "integer_bits": 256, "bytes": 262_144}


def measure(operation, data, *, samples, iterations):
    """按批次采样，保留真实 Python 容器构造与释放成本。"""
    operation(data)
    timings = []
    for _ in range(samples):
        started = time.perf_counter_ns()
        for _ in range(iterations):
            result = operation(data)
            del result
        timings.append((time.perf_counter_ns() - started) / iterations / 1_000_000)
    return {"median_ms": statistics.median(timings), "samples_ms": timings}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--samples", type=int, default=21)
    parser.add_argument("--iterations", type=int, default=30)
    args = parser.parse_args()
    if args.samples < 3 or args.iterations < 1:
        parser.error("至少采样 3 批，每批至少调用 1 次")
    if not native_available():
        raise SystemExit("此基准要求包含 PTC 值内核的原生 wheel")
    kernel = PtcValueKernel(**LIMITS)
    results = {}
    for size in (100, 400, 800):
        data = [{"id": index, "text": "示例文本" * 5} for index in range(size)]
        copied = kernel.copy(data)
        assert copied == data and copied[0] is not data[0]
        copied[0]["id"] = -1
        assert data[0]["id"] == 0
        assert kernel.validate(data) is None
        results[str(size)] = {
            name: measure(operation, data, samples=args.samples, iterations=args.iterations)
            for name, operation in (("copy", kernel.copy), ("validate", kernel.validate))
        }
    print(
        json.dumps(
            {
                "platform": platform.platform(),
                "python": platform.python_version(),
                "version": __version__,
                "backend": "rust",
                "limits": LIMITS,
                "samples": args.samples,
                "iterations": args.iterations,
                "warmup": 1,
                "scope": "PTC 值内核，含 Python 结果构造与释放，不含宿主解释器、权限、文件或网络",
                "rows": results,
            },
            ensure_ascii=True,
            indent=2,
        )
    )


if __name__ == "__main__":
    main()
