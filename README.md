# neobot-agent-tools

NeoBot Agent 工具的可复用文件内核：UTF-8 快照、行索引、分页、检索输入准备和进程内资源预算。

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

这些额度是工作集估算和缓存记账，不是 OS 级 RSS 硬限制。多进程分别计数。宿主必须在文件锁之前申请额度，原生计算使用 `Python::detach` 释放 GIL，并在计算返回后才释放预留。原生快照为不可变共享存储，`share()` 不复制整份内容。

## 发布配置

CI 构建与测试通过后，标签发布工作流可以使用 PyPI Trusted Publishing。组织维护者需在 PyPI 配置本仓库、发布 workflow 和 `pypi` environment 的信任关系。初始化阶段只构建 wheel，不自动创建发布标签或上传 PyPI。

原生源码与 Python facade 使用同一版本和 `API_VERSION`。加载状态可由 `backend_info()` 查看；原生模块缺失或无法加载时，auto 模式使用 Python 后端并给出原因，`backend="rust"` 则明确失败。
