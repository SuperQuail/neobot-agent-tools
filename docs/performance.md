# 性能记录与复现

## 测量范围

本库的基准只测内存计算和 Python 结果构造。磁盘读取、权限校验、ripgrep 子进程、模型和网络等待都需要在宿主另外测量，不能用内核倍率代替整个 Agent 的收益。

先安装 release wheel。`scripts/benchmark.py` 度量快照创建、全文分页与检索输入准备；`scripts/benchmark_ptc.py` 独立度量 PTC 值复制和完整校验，两者都不依赖 NeoBot。

Windows 示例：

```powershell
uv run --no-project --python .venv/Scripts/python.exe python scripts/benchmark.py --samples 21
uv run --no-project --python .venv/Scripts/python.exe python scripts/benchmark_ptc.py --samples 21 --iterations 30
```

脚本要求实际加载 Rust。PTC 脚本每种操作先预热，再记录每批调用的平均耗时与批次中位数；源码回退不能用于此原生基准。

## 2026-10-27：PTC 环境复查与数据复制

环境为 Windows 11 x64、CPython 3.13.14，本地 release wheel。原实现、新 Python 路径和 Rust 路径交替执行，每个场景预热 1 次、采样 5 次。每行固定为 `{"id": index, "text": "示例文本" * 5}`。下面记录来自当时的 NeoBot 联调，不是每次安装都会达到的固定指标。

| 场景 | 原实现 | 新 Python 路径 | Rust 路径 | 原实现／Rust |
|---|---:|---:|---:|---:|
| 100 行循环计数 | 40.848 ms | 33.174 ms | 5.722 ms | 7.1 倍 |
| 400 行循环计数 | 632.107 ms | 514.103 ms | 79.138 ms | 8.0 倍 |
| 800 行循环计数 | 2533.719 ms | 2028.873 ms | 299.794 ms | 8.5 倍 |
| 400 行赋值复制并修改，20 次 | 120.032 ms | 96.325 ms | 14.516 ms | 8.3 倍 |
| 800 行工具参数／返回值往返 | 15.890 ms | 14.979 ms | 2.597 ms | 6.1 倍 |
| 单次复制 800 行 | 1.604 ms | 1.496 ms | 0.245 ms | 6.5 倍 |
| 单次复查 800 行 | 1.543 ms | 1.252 ms | 0.179 ms | 8.6 倍 |

原始数据为 [ptc-values-2026-10-27.json](benchmarks/ptc-values-2026-10-27.json)。前五项包含 NeoBot 的解析、解释执行、合成工具调度、复制和预算检查；后两项独立度量值计算，每批 30 次。旧“复查”仍会创建副本后丢弃，新路径只校验。

### 结果如何解释

收益来自减少环境副本、Python 路径在一次遍历内复用不可变字符串的编码成本，以及 Rust 的有界遍历与复制。每次出现的值仍独立计费，环境变化后仍完整复查，没有缓存可变容器的旧额度。

800 行循环场景中，新 Python 路径比原实现减少约 20% 耗时，Rust 在此基础上再快约 6.8 倍。Python PTC 回退属于 NeoBot 宿主，本库只发布原生值接口。

完整环境复查的复杂度仍随环境体积与复查次数相乘增长；没有以跳过权限、隔离或额度检查换取倍率。

### 复现范围

本仓库的 `scripts/benchmark_ptc.py` 可以独立复现同样数据形状的原生复制与校验，并输出平台、额度和采样。它不内置旧版 NeoBot 解释器，因此不声称复现表中的宿主循环或提供其 Python 回退。

重测完整宿主流程需要对应的 NeoBot 本地适配、改动前保存的 `ptc.py` 和宿主基准脚本；这些改动尚未作为本仓库接口分发。宿主接入流程见 [接入约定](integration.md)。

### 当时的验证证据

- 5000 组确定种子的输入，对照改动前实际实现执行 20000 次比较，覆盖复制／仅校验、错误码、文案、别名和循环。
- NeoBot PTC 专项 234 项，全量本地回归 6014 项通过；这两组数量属于宿主，不能算成本库测试数。
- 独立库 Rust 单测 7 项；原生 Windows wheel 在 Python 3.13、3.14 各 86 项通过。
- 内核提交 [2043df9](https://github.com/SuperQuail/neobot-agent-tools/commit/2043df9990a4fa646f1d03665e3bd2b111e02deb) 的 [构建记录](https://github.com/SuperQuail/neobot-agent-tools/actions/runs/40504653039) 覆盖 Windows x64、Linux x64／ARM64。
