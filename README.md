# neobot-agent-tools

NeoBot Agent 工具的可复用计算内核：UTF-8 快照、行索引、分页、检索输入准备、进程内资源预算，以及 PTC 值校验与隔离复制。

Rust 负责纯字节计算，Python 提供稳定接口和可用性回退。包本身不解析聊天身份、不授予路径访问权限，也不直接读取文件。宿主应先完成授权、加锁和有界读取，再把字节交给内核；写入版本与 owner 观察仍由宿主维护。

## 使用快照

```python
import time

from neobot_agent_tools import FILE_IO_RESOURCES, Snapshot, backend_info
from neobot_agent_tools.resources import capture_weight

data = "第一行\r\nsecond\n".encode("utf-8")
with FILE_IO_RESOURCES.operation(capture_weight(len(data)), deadline=time.monotonic() + 5):
    snapshot = Snapshot.from_bytes(data, "host-owned-version")
    page = snapshot.read_page(offset=1, limit=1)
    assert page.as_dict()["content"] == "第一行\r\n"
    assert snapshot.shares_storage(snapshot.share())

print(backend_info())
```

单次输入限 16 MiB，分页最多 2000 行／64 KiB，offset 从 1 开始，column 按 Unicode 字符计数。换行语义与 Python `str.splitlines()` 一致，保留 CRLF 和混合换行。`read_page` 还返回用于宿主观察覆盖范围的字符位置，内核不会据此授予写入权限。

宿主需在捕获／检索期间持有工作集预留，并为长期缓存登记 `snapshot.cost`；示例中的小快照只展示接口，不构成缓存管理器。NeoBot 的 `FileSnapshotCache` 负责缓存登记、变化校验和按文件锁协调。

## PTC 值内核

```python
from neobot_agent_tools.ptc_values import PtcValueKernel, native_available

if native_available():
    values = PtcValueKernel(
        nodes=16_000,
        depth=32,
        items=4_000,
        string_chars=32_768,
        integer_bits=256,
        bytes=262_144,
    )
    source = {"rows": [{"id": 1}]}
    copied = values.copy(source, timeout_seconds=1.0)
    values.validate(source, timeout_seconds=1.0)
    assert copied["rows"] is not source["rows"]
```

只接受精确的 `str`、`int`、有限 `float`、`bool`、`None`、`list` 和字符串键 `dict`，拒绝子类、任意宿主对象和循环引用。重复别名每次都计入节点、深度和保守 JSON 字节额度，复制时分别创建容器，不能借共享引用绕过上限。字节口径包含 ASCII 转义、分隔符空格和原有布尔／空值的保守成本，支持 256 位整数及 Python 字符串中的孤立代理项。

`validate` 完整复查而不复制容器；`copy` 在赋值和工具边界创建独立数据。每次调用的计数和祖先集合独立，内核只保存不可变额度，不缓存可变对象或跨 agent 共享环境。遍历 Python 容器时持有 GIL，定期和返回前检查时间；宿主仍需保留异步取消、步数和工具调用预算。这里的时间预算不构成操作系统级硬中断。

原生深度配置限 1–128；宿主可对更深或超过本机整数表示范围的自定义额度使用自己的 Python 回退。NeoBot 同时兼容未安装包、旧版包和原生扩展不可用的情况，不降低既有配置上限。PTC 接口使用独立的 `PTC_VALUE_API_VERSION=1` 能力标记，扩展缺失时 `native_available()` 返回 `False`，构造内核则明确失败。权限、AST 白名单、工具调度和赋值时机均由宿主控制。

## 安装与发布

发布后由 `neobot-app` 声明版本依赖，用户仍可用 uv 安装 NeoBot。正常安装选择平台 wheel，Rust 编译发生在发布 CI。当前仓库的初始化版本尚不代表 PyPI 上已有可安装发行版。

源码子模块只用于联调，正常运行依赖使用包版本，不把子模块路径设为默认 `tool.uv.sources`，以免部署时触发 Rust 源码构建。

首版构建目标为 Windows x64、Linux x64／ARM64；使用 CPython 3.13 的 stable ABI，并在普通 CPython 3.13、3.14 验证同一平台 wheel。free-threaded Python 与其他平台在经过实际构建和测试前不列入支持范围。

## 本地构建与验证

需要 Rust、uv，以及 Python 3.13。以下命令在仓库根目录执行：

```powershell
uvx --from maturin==1.15.0 maturin build --release --locked --out dist
python scripts/test_wheel.py dist
```

`test_wheel.py` 使用 uv 创建隔离测试环境，在 Python 3.13／3.14 安装同一 wheel，且要求原生后端实际可用，不会用 Python 回退冒充原生验证。纯 Python 回退也由同一组兼容性测试覆盖。

```powershell
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --locked
```

Windows 使用 uv 的 Python 时，如需指定 Rust 绑定解释器，可设置 `PYO3_PYTHON` 为对应的 Python 可执行文件路径。

## 并发与内存边界

`FILE_IO_RESOURCES` 是同一 Python 解释器内的共享协调器。默认工作集预留 256 MiB、最多 4 个文件操作／1 个检索；所有注册缓存合计 64 MiB／2048 项，按需启动单一清理线程。缓存通过 `register/retain/release/unregister` 登记，宿主缓存需实现 `expire()`；关闭或回收后归还额度。

这些额度是工作集估算和缓存记账，不是 OS 级 RSS 硬限制。多进程分别计数。宿主必须在文件锁之前申请额度，文件快照的原生计算使用 `Python::detach` 释放 GIL，并在计算返回后才释放预留。原生快照为不可变共享存储，`share()` 不复制整份内容。PTC 值内核不使用文件缓存预算，仍受宿主每次运行的 JSON 额度限制。

## 发布配置

CI 构建与测试通过后，标签发布工作流可以使用 PyPI Trusted Publishing。组织维护者需在 PyPI 配置本仓库、发布 workflow 和 `pypi` environment 的信任关系。初始化阶段只构建 wheel，不自动创建发布标签或上传 PyPI。

原生源码与 Python facade 使用同一版本和 `API_VERSION`。加载状态可由 `backend_info()` 查看；原生模块缺失或无法加载时，auto 模式使用 Python 后端并给出原因，`backend="rust"` 则明确失败。

## 开发结构与规范

`src/core.rs` 维护快照与分页，`src/core/index.rs` 负责行索引和 Unicode 检查点，`src/core/lines.rs` 负责换行扫描与检索归一化；`src/ptc_values/` 负责有界 JSON 遍历与复制。`src/lib.rs` 仅处理 Python 绑定和协议转换。Python 快照 facade、回退与资源协调分别位于 `snapshots.py`、`_fallback.py`、`resources.py`；PTC facade 位于 `ptc_values.py`。

Rust 和 Python 均优先采用提前返回及职责明确的函数，内部多字段状态使用具名类型；仅在绑定边界保留兼容元组。解释性注释、Rust 文档注释和 Python docstring 使用中文，说明所有权、UTF-8 边界、预算和锁顺序。完整约定见 [AGENTS.md](AGENTS.md)。

`scripts/benchmark.py` 可针对已安装的 release wheel 度量快照创建、全文分页和检索输入准备，并核对最终内容和共享存储。它只度量内存接口，不包含文件系统与宿主授权耗时；比较时应交替运行新旧 wheel，并记录同机多次采样的中位数及缓存记账。
