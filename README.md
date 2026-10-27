# neobot-agent-tools

为 Python Agent 提供可独立复用的 Rust 计算内核：文件文本快照、有界分页、检索输入准备，以及 PTC（程序化工具调用）的值校验与隔离复制。

Python 提供稳定接口，Rust 加速重复计算，平台 wheel 负责二进制分发。宿主继续负责文件访问、工具权限和执行策略。

**当前处于独立工具库整理与联调阶段。** 源码及平台 wheel 已有构建验证，NeoBot 主仓的正式依赖和工具入口整合尚未完成；当前安装方式为下载 CI wheel 或本地构建。普通 `git pull` 只更新源码，不会安装或更新原生模块。

[安装](#安装) · [快速上手](#快速上手) · [API 参考](docs/api.md) · [宿主接入](docs/integration.md) · [开发与分发](docs/development.md) · [性能记录](docs/performance.md)

## 已提供的能力

| 能力 | 主要接口 | 后端与边界 |
|---|---|---|
| 文本快照与行索引 | `Snapshot.from_bytes()`、`share()` | Rust／Python；只接收已读取的 UTF-8 字节 |
| 有界分页与长行续读 | `read_page()`、`ReadPage.as_dict()` | Rust／Python；单页行数和 UTF-8 字节受限 |
| 检索输入准备 | `for_search()` | Rust／Python；归一化换行，供宿主匹配器使用 |
| 进程内资源协调 | `FILE_IO_RESOURCES` | Python；共享工作集、操作数和缓存额度 |
| PTC 值复查与复制 | `PtcValueKernel.validate()`／`copy()` | Rust；无状态遍历，宿主自行提供缺失扩展时的回退 |

本库尚不提供独立的 MCP 服务或完整 Agent 工具运行时。批量文件读取与编辑、无变化写入跳过、ripgrep 进程调度，以及 PTC 的 AST 解释器，目前由 NeoBot 宿主实现；不会把宿主中的能力计作已发布的库接口。

## 安装

使用预编译 wheel 时，用户只需要 **uv 和兼容的 CPython**，不需要 Rust 工具链。发行包名为 `neobot-agent-tools`，Python 导入名为 `neobot_agent_tools`。

### 使用 CI wheel

从 [Wheels 工作流](https://github.com/SuperQuail/neobot-agent-tools/actions/workflows/wheels.yml) 中选择已成功完成的运行，在 Artifacts 下载对应平台的 `wheels-...` 压缩包并解压；GitHub 可能要求登录后下载。

以下为 Windows x64 示例，将 wheel 放在当前目录的 `dist/` 后运行：

```powershell
uv venv --python 3.13
uv pip install ./dist/neobot_agent_tools-0.1.0-cp313-abi3-win_amd64.whl
uv run --no-project --python .venv/Scripts/python.exe python -c "from neobot_agent_tools import backend_info; print(backend_info())"
```

Linux 选择对应的 x64／ARM64 wheel，安装到项目虚拟环境。CI artifact 目录名见下表，解压后的 `.whl` 才是 uv 的安装输入。

| 平台 | Artifact | 已验证的 Python |
|---|---|---|
| Windows x64 | `wheels-x86_64-pc-windows-msvc` | CPython 3.13、3.14 |
| Linux x64 | `wheels-x86_64-unknown-linux-gnu` | CPython 3.13、3.14，manylinux2014 |
| Linux ARM64 | `wheels-aarch64-unknown-linux-gnu` | CPython 3.13、3.14，manylinux2014 |

同一平台的 wheel 使用 `abi3-py313`，可供上述两个 Python 版本使用。macOS、Windows ARM64、musl 和 free-threaded Python 尚未列入已验证的支持范围。

### 从源码构建

开发者需准备 Rust、uv 和 Python 3.13，在仓库根目录执行：

```powershell
uvx --from maturin==1.15.0 maturin build --release --locked --out dist
uv run --no-project --python 3.13 python scripts/test_wheel.py dist
```

第二条命令会在隔离环境中分别用 Python 3.13／3.14 安装并测试 wheel，要求实际加载 Rust。构建后可按上面的 wheel 安装方式接入自己的项目。源码构建、Python 回退测试和正式发布步骤见 [开发与分发](docs/development.md)。

## 快速上手

### 读取内存文本并继续翻页

```python
import time

from neobot_agent_tools import FILE_IO_RESOURCES, Snapshot
from neobot_agent_tools.resources import capture_weight

data = "第一行\r\nsecond\n".encode("utf-8")
deadline = time.monotonic() + 5
with FILE_IO_RESOURCES.operation(capture_weight(len(data)), deadline=deadline):
    snapshot = Snapshot.from_bytes(data, version="example-v1")
    first = snapshot.read_page(offset=1, limit=1)
    second = snapshot.read_page(offset=first.next_offset, column=first.next_column, limit=1)
    assert first.as_dict()["content"] == "第一行\r\n"
    assert second.as_dict()["content"] == "second\n"
    assert second.next_offset is None
    assert snapshot.shares_storage(snapshot.share())
    print(snapshot.backend, snapshot.line_count)
```

`offset` 从 1 开始，`column` 从 0 开始，按 Unicode 字符计数。单次输入最多 16 MiB，单页最多 2000 行／64 KiB；长行可按 `next_offset` 和 `next_column` 继续读取。原始混合换行保留，`version` 是宿主提供的标签，本库不会计算文件版本或授予写入权限。

`snapshot.for_search()` 返回带有归一化 `search_bytes` 的快照。宿主保留该返回对象后可复用检索输入；本库不启动 ripgrep，也不实现文件发现或正则超时。

### 复查 PTC 数据并隔离赋值

```python
from neobot_agent_tools.ptc_values import PtcValueKernel, native_available

if not native_available():
    raise RuntimeError("请安装包含 PTC 值内核的兼容 wheel，或由宿主提供 Python 回退")

values = PtcValueKernel(
    nodes=16_000,
    depth=32,
    items=4_000,
    string_chars=32_768,
    integer_bits=256,
    bytes=262_144,
)
source = {"rows": [{"id": 1}]}
values.validate(source, timeout_seconds=1.0)
copied = values.copy(source, timeout_seconds=1.0)
copied["rows"][0]["id"] = 2
assert source["rows"][0]["id"] == 1
```

`validate()` 完整检查额度，不创建容器副本；`copy()` 逐次复制列表／字典，重复别名也分别复制并计费。只接受精确 JSON 类型，拒绝子类、循环、非有限浮点数和任意宿主对象。上面是常用额度配置，构造函数要求显式传入各项额度。错误码和字节计费规则见 [PTC API](docs/api.md#ptc-值内核)。

快照的 `backend="auto"` 在扩展不可用时自动回退 Python；`backend="rust"` 会明确失败。PTC 的 `native_available()` 独立检查 `PTC_VALUE_API_VERSION=1`，**本包不内置 PTC 的 Python 回退**，宿主应在能力不可用或自定义深度超过 128 时选择自己的回退实现。

## 多 agent 与资源边界

同一 Python 解释器通常复用一个 `FILE_IO_RESOURCES`，不要为每个 agent 创建独立额度池。默认限制如下：

| 资源 | 默认额度 |
|---|---:|
| 在途文件工作集估算 | 256 MiB |
| 同时进行的文件操作 | 4 |
| 其中同时进行的检索 | 1 |
| 所有已登记缓存的合计额度 | 64 MiB／2048 项 |
| 缓存清理检查间隔 | 1 秒，按需启动单一线程 |

`Snapshot.share()` 共享不可变底层数据，不复制全文。缓存仍由宿主实现，负责文件变化检查、TTL、`snapshot.cost` 登记与释放；缓存被回收或关闭时须归还额度。各 agent 的文件观察版本和授权状态不在这里共享。多进程各自计数，这些数字是工作集估算，不是操作系统 RSS 硬限制。

文件快照计算可释放 GIL；PTC 遍历可变 Python 容器时保持 GIL，每次调用独立持有计数和祖先集合，不跨调用缓存可变环境。PTC 时间预算定期及返回前检查，异步取消和整个程序的超时仍由宿主控制。接入顺序和锁约定见 [宿主接入](docs/integration.md)。

## 性能与验证

在 Windows x64、CPython 3.13.14 的一次本地 release 实测中：

| 场景 | 改动前 | Rust 路径 | 倍率 |
|---|---:|---:|---:|
| 单次复制 800 行 PTC 数据 | 1.604 ms | 0.245 ms | 6.5 倍 |
| 单次复查 800 行 PTC 数据 | 1.543 ms | 0.179 ms | 8.6 倍 |
| NeoBot PTC 中循环计数 800 行 | 2533.719 ms | 299.794 ms | 8.5 倍 |

前两项为值内核，最后一项包含本地 NeoBot 解释器及环境复查。数据不包含模型、网络或文件 I/O，不能直接换算为整个 Agent 的响应速度。[性能记录](docs/performance.md) 提供采样口径、原始数据和独立内核复现命令。

独立仓库已覆盖 Rust 单测、Python 快照回退、资源协调及原生 PTC；平台 wheel 在 CPython 3.13／3.14 验证。构建状态以 [Wheels 工作流](https://github.com/SuperQuail/neobot-agent-tools/actions/workflows/wheels.yml) 为准，开发检查与目录说明见 [开发与分发](docs/development.md)，中文注释及可读性约定见 [AGENTS.md](AGENTS.md)。

## 许可证

[MIT](LICENSE)
