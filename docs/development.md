# 开发与分发

## 环境与目录

需要 uv、Rust 工具链和普通 CPython 3.13。当前 CI 固定使用 Rust 1.96.0、uv 0.11.24、maturin 1.15.0、Ruff 0.16.9；测试 wheel 时也会使用 CPython 3.14。

```text
src/
  lib.rs                 Python 绑定和协议转换
  core.rs                快照与分页
  core/index.rs          行索引与 Unicode 检查点
  core/lines.rs          行分隔符扫描及检索归一化
  ptc_values/            有界 JSON 遍历、字节计费和深复制
python/neobot_agent_tools/
  snapshots.py           稳定快照接口与后端选择
  _fallback.py           Python 快照实现
  resources.py           进程内工作集及缓存协调
  ptc_values.py          PTC 原生能力检测及错误转换
scripts/
  test_wheel.py          Python 3.13／3.14 隔离 wheel 验证
  benchmark.py           快照创建、全文分页和检索输入基准
  benchmark_ptc.py       PTC 值内核的独立基准
tests/                   接口、类型、额度、隔离和并发测试
docs/                    API、接入约定和性能记录
```

## 构建与验证 wheel

在仓库根目录执行；`dist/` 应只保留本次待验证的一个平台 wheel。

```powershell
uvx --from maturin==1.15.0 maturin build --release --locked --out dist
uv run --no-project --python 3.13 python scripts/test_wheel.py dist
```

`test_wheel.py` 通过 uv 把 wheel 和 pytest 装进隔离环境，默认依次使用 Python 3.13／3.14。它同时要求快照后端为 Rust、PTC 原生能力可用；不能用静默回退冒充原生验证。可用 `--python 3.13` 只验证一个解释器。

联调期间重建同一版本的 wheel 后，可用 `uv pip install --reinstall` 加对应 wheel 路径更新开发环境，避免继续加载已安装的旧模块；已启动的 Python 进程需要重启。

Windows 上需要显式选择解释器时，可先创建虚拟环境：

```powershell
uv venv --python 3.13
$env:PYO3_PYTHON = (Resolve-Path .venv/Scripts/python.exe).Path
uvx --from maturin==1.15.0 maturin build --release --locked --out dist
```

## 代码质量

```powershell
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --locked
uvx --from ruff==0.16.9 ruff check .
uvx --from ruff==0.16.9 ruff format --check .
```

可读性、中文注释和兼容行为遵循 [AGENTS.md](../AGENTS.md)。只重排自己改动的代码，不为通过检查修改门禁、断言或失败用例。

### 验证快照 Python 回退

源码目录中没有已编译扩展时，下面的命令直接导入 `python/`。在 PowerShell 执行：

```powershell
$env:PYTHONPATH = "python"
uv run --no-project --python 3.13 --with "pytest>=9,<10" python -B -m pytest tests -q
Remove-Item Env:PYTHONPATH
```

Linux 对应命令：

```bash
PYTHONPATH=python uv run --no-project --python 3.13 --with 'pytest>=9,<10' python -B -m pytest tests -q
```

这条路径验证快照回退、资源协调和 PTC 能力不可用时的明确失败。依赖原生 PTC 的用例会按设计跳过，因为库内没有 PTC Python 回退；它们必须在 wheel 测试中运行。2026-10-27 的记录为源码环境 45 passed／41 skipped，原生 wheel 在两个 Python 版本各 86 passed。

## 性能复现

先安装 release wheel，再在仓库根目录执行。下面是 Windows 虚拟环境示例：

```powershell
uv run --no-project --python .venv/Scripts/python.exe python scripts/benchmark.py --samples 21
uv run --no-project --python .venv/Scripts/python.exe python scripts/benchmark_ptc.py --samples 21 --iterations 30
```

两个脚本均要求真实原生后端，不包含文件、网络、模型或授权耗时。比较版本时固定机器、解释器、输入和 release 构建方式，交替运行新旧 wheel，记录中位数和原始采样。主仓完整 PTC 解释器的基准另见 [性能记录](performance.md)。

## CI 与发布

[Wheels 工作流](../.github/workflows/wheels.yml) 在分支推送、PR 和手动触发时运行：

1. 质量任务执行 Rust 检查和 Python 快照回退测试。
2. Windows x64、Linux x64 和 Linux ARM64 分别构建 release wheel。
3. 每个 wheel 在 CPython 3.13／3.14 隔离环境验证，再上传 `wheels-目标三元组` artifact。

普通分支推送只构建和测试。只有 `v*` 标签进入发布步骤，且标签必须等于 `pyproject.toml` 中的 `v` 加版本号；发布依赖 PyPI Trusted Publishing 与 GitHub 的 `pypi` environment 配置。

版本提升、发行标签和 PyPI 上传单独处理，不能把一次 main 推送或 CI artifact 当作已发布的包版本。Python facade 与 Rust 的包版本、快照 `API_VERSION` 必须兼容；PTC 使用独立的 `PTC_VALUE_API_VERSION=1` 能力标记。
