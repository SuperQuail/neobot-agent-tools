# API 参考

本库处理内存数据。文件路径、聊天身份、文件版本的生成与比较均由宿主提供。发行包为 `neobot-agent-tools`，导入包为 `neobot_agent_tools`。

## 快照与分页

从 `neobot_agent_tools` 导入 `Snapshot`、`ReadPage`、`backend_info`。

| 接口 | 输入 | 返回与用途 |
|---|---|---|
| `Snapshot.from_bytes(data, version, backend="auto")` | UTF-8 `bytes`、宿主版本字符串 | 不可变快照；最多 16 MiB |
| `snapshot.read_page(offset=1, limit=2000, column=0, max_bytes=65536)` | 行、列和输出预算 | `ReadPage`，最多 2000 行／64 KiB |
| `snapshot.read_fragment(index, column, char_limit)` | 从 0 开始的行、列和字符上限 | `(文本片段, 该列起剩余字符数)`，含原有换行 |
| `snapshot.line(index, keepends=True)` | 从 0 开始的行号 | 原行文本，可移除行末换行 |
| `snapshot.start(index)` | 从 0 开始的行边界 | 原文本中的字符位置；超出末尾的边界钳制到全文末尾 |
| `snapshot.for_search()` | 无 | 带 `search_bytes` 的新快照，复用原文本内核 |
| `snapshot.share()` | 无 | 共享同一底层内容的快照包装 |
| `snapshot.shares_storage(other)` | 另一个快照 | 是否使用相同底层存储 |

`size` 为源 UTF-8 字节数，`char_len` 为 Python 字符数，`line_count` 为行数。`cost` 为宿主登记缓存时使用的保守存储估算，包含已准备的检索字节。共享引用不会复制底层数据，但宿主仍需管理缓存条目及在途请求的归属。

### 分页坐标与返回值

- `read_page.offset` 从 1 开始，`column` 从 0 开始；低层 `line`／`read_fragment` 的行下标从 0 开始。
- 所有列和 `ReadPage.start`／`end` 按 Python Unicode 字符计数，输出预算按 UTF-8 字节计数。
- `rows` 是 `(行号, 文本片段, 起始列)` 的元组序列；原换行计入文本和字符范围。
- 行切分与 `str.splitlines()` 一致，空文件为 0 行，行末换行不会额外产生一行。
- `next_offset is None` 表示没有后续文本；否则把行列游标一起传入下一次读取。
- 太小的 `max_bytes` 可能连一个 UTF-8 字符也容纳不下，导致游标不前进；宿主须增加预算或结束本批，避免空转。

`ReadPage.as_dict()` 返回以下字段。示例对应 README 中 18 字节的两行文本的第一页：

```json
{
  "ok": true,
  "version": "example-v1",
  "size": 18,
  "offset": 1,
  "column": 0,
  "lines": [{"number": 1, "text": "第一行\r\n", "column": 0}],
  "content": "第一行\r\n",
  "totalLines": 2,
  "truncated": true,
  "next_offset": 2,
  "next_column": 0
}
```

`version` 原样来自宿主。字符范围用于宿主累计当前 owner 的读取覆盖，不代表已经获得覆盖写权限。

### 检索输入与后端

`for_search()` 把 CRLF 和其他受支持的行分隔符统一成 LF；非空内容补齐末尾 LF。保存返回的快照即可复用 `search_bytes`，反复对未保存结果的原对象调用会重新准备输入。文件枚举、正则兼容性判断、ripgrep 启动和进程超时均由宿主处理。

`backend_info()` 返回 `backend`、`version`、`api_version` 和 `native_error`。它表示快照原生扩展的可用性；单个对象实际使用的后端查看 `snapshot.backend`。原生版本或 `API_VERSION` 不匹配时关闭快照原生后端。

| 异常 | 常见原因 |
|---|---|
| `TypeError` | 输入不是字节或版本不是字符串 |
| `UnicodeDecodeError` | 输入不是合法 UTF-8 |
| `ValueError` | 输入过大，或分页行列／预算非法 |
| `IndexError` | `line()`／`read_fragment()` 的行下标越界 |
| `RuntimeError` | 要求 `backend="rust"`，但原生后端不可用 |

## PTC 值内核

从 `neobot_agent_tools.ptc_values` 导入 `PtcValueKernel`、`PtcValueError`、`native_available`。它只校验和复制数据，不解析或运行 PTC 程序。

构造函数必须显式传入下面六项正整数。示例值来自 NeoBot 的常用配置，除原生深度限制外，不是不可修改的固定常量。

| 参数 | 示例值 | 含义 |
|---|---:|---|
| `nodes` | 16000 | 每次遍历的总节点数，字典键也计入 |
| `depth` | 32 | 根为 0 的最大深度；原生配置范围 1–128 |
| `items` | 4000 | 每个列表或字典的最大元素数 |
| `string_chars` | 32768 | 单个字符串的最大 Python 字符数 |
| `integer_bits` | 256 | 单个整数 `bit_length()` 的最大值 |
| `bytes` | 262144 | 每次遍历的保守 JSON 字节预算 |

| 接口 | 返回值 | 适用边界 |
|---|---|---|
| `copy(value, byte_limit=None, timeout_seconds=30.0)` | 独立的列表／字典树，标量值保持 | 赋值、传入工具参数和接收工具结果 |
| `validate(value, byte_limit=None, timeout_seconds=30.0)` | `None` | 环境复查、完整输出预算检查 |
| `native_available()` | `bool` | 检查独立能力标记 `PTC_VALUE_API_VERSION=1` |

`byte_limit` 只覆盖本次字节预算，其他额度仍照常检查；`timeout_seconds` 是本次调用的非负有限秒数，0 表示立即到期。宿主若有整个程序的统一截止时间，必须每次传入剩余时间，不能让各次调用重新获得完整程序预算。

### 类型、别名与额度

仅接受精确的 `str`、`int`、有限 `float`、`bool`、`None`、`list`、字符串键 `dict`。不接受这些类型的子类、元组、任意对象、非有限数和循环引用，不调用宿主自定义属性或方法。整数保留 Python 精度，不缩减到 64 位；字符串支持孤立代理项，不强制转成 Rust UTF-8 字符串。

重复别名每次出现都计入额度，复制时各自创建容器。节点额度不是“不同对象的数量”，因此不能用重复引用放大结果或绕过预算。

字节计费与原 PTC 的保守规则一致：字符串按 `ensure_ascii=True` 转义后的 JSON 长度；整数和有限浮点数按 Python 表示计费；布尔／空值统一按 5 字节。列表成本为 `2 + 2 × 元素数`，字典成本为 `2 + 4 × 元素数`，再加子节点成本。它是资源预算，不是最紧凑 JSON 的精确输出长度。

### 错误与回退

| `PtcValueError.code` | 原因 | 宿主处理 |
|---|---|---|
| `PTC_LIMIT` | 节点、深度、集合、字符串、整数或字节超限 | 终止本次程序，不向工具发起后续调用 |
| `PTC_VALUE` | 类型不受支持、非字符串键、循环或非有限数 | 拒绝该输入／工具结果 |
| `PTC_TIMEOUT` | 遍历的时间预算耗尽 | 交给程序的超时处理 |

`str(error)` 是错误文案。非法配置和参数还可能抛出标准 `ValueError`、`TypeError` 或 `OverflowError`，不能一律当作 `PtcValueError`。

构造 `PtcValueKernel` 时若扩展缺失或能力标记不兼容，会明确抛出 `RuntimeError`。本包不提供该内核的 Python 回退；宿主应在构造前检查能力，并对深度超过 128 或无法用本机原生整数表示的配置使用自己的回退，不应静默降低额度。

遍历持有 GIL，定期与返回前检查时间，不构成操作系统级硬中断。内核只保存额度，不缓存容器、环境或上次遍历计数；完整 AST、权限、取消与程序生命周期仍由宿主管理。

## 资源协调

从 `neobot_agent_tools` 导入共享实例 `FILE_IO_RESOURCES`、可配置的 `FileIoResources` 和 `FileResourceError`；从 `neobot_agent_tools.resources` 导入 `capture_weight(size)`、`search_weight(size)`。

| 接口 | 参数 | 约定 |
|---|---|---|
| `operation(weight, search=False, deadline=...)` | 预估字节、检索标志、`time.monotonic()` 绝对截止时间 | 上下文管理器，退出时归还在途额度 |
| `register(cache)` | 可弱引用且提供 `expire()` 的宿主缓存 | 返回登记 token |
| `retain(token, size)` | 一个缓存条目的成本 | 成功返回 `True`；额度不足返回 `False` |
| `release(token, size)` | 与登记成本相同的字节数 | 释放一个条目的记账 |
| `unregister(token)` | 登记 token | 移除整个缓存并释放剩余记账 |
| `status()` | 无 | 工作集、操作数、缓存和清理线程状态 |

`capture_weight(size)` 为 `14 × size + 8 MiB`，`search_weight(size)` 为 `24 × size + 4 MiB`。宿主必须先限制源数据体积，再使用这些保守估算申请额度。

`deadline` 限制资源准入的等待；不在上下文体中自动中断工作。容量立即可用时宿主也须自行检查操作截止时间。过大的工作集或等待超时抛出 `FileResourceError`，`code="file_busy"`。`FileChangedDuringAdmission` 是供宿主表达准入期间文件变化的异常，`code="file_changed"`，协调器本身不读取文件。

缓存的 TTL 和淘汰由宿主实现。协调器每隔一段时间调用缓存的 `expire()`，不持协调器锁调用宿主清理逻辑；缓存没有条目后清理线程退出。宿主关闭缓存时应 `unregister()`，不能依靠协调器替代自己的缓存生命周期管理。
