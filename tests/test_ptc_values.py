"""验证原生值内核的精确类型、逻辑额度、隔离与无状态复用。"""

import json
from concurrent.futures import ThreadPoolExecutor

import pytest
from neobot_agent_tools import ptc_values


@pytest.fixture
def kernel():
    if not ptc_values.native_available():
        pytest.skip("Native PTC kernel is not installed; host fallback is tested in NeoBot")
    return make_kernel()


def make_kernel(**changes):
    limits = {
        "nodes": 16_000,
        "depth": 32,
        "items": 4_000,
        "string_chars": 32_768,
        "integer_bits": 256,
        "bytes": 262_144,
    }
    limits.update(changes)
    return ptc_values.PtcValueKernel(**limits)


def test_missing_extension_is_explicit(monkeypatch):
    monkeypatch.setattr(ptc_values, "_native", None)
    assert not ptc_values.native_available()
    with pytest.raises(RuntimeError, match="unavailable"):
        make_kernel()


def test_copy_isolates_aliases_and_validation_does_not_mutate(kernel):
    shared = {"nested": [1, 2]}
    source = [shared, shared]
    copied = kernel.copy(source)
    assert copied == source
    assert copied[0] is not copied[1]
    copied[0]["nested"][0] = 9
    assert copied[1]["nested"] == shared["nested"] == [1, 2]
    assert kernel.validate(source) is None
    assert source[0] is source[1] is shared


@pytest.mark.parametrize("value", [None, True, False, 0, -(2**255), 2**256 - 1, 1e20, 1e-8, -0.0])
def test_exact_scalars_and_byte_boundary(kernel, value):
    cost = 5 if value is None or type(value) is bool else len(json.dumps(value))
    assert kernel.copy(value, byte_limit=cost) == value
    for operation in (kernel.copy, kernel.validate):
        with pytest.raises(ptc_values.PtcValueError, match="JSON byte limit") as error:
            operation(value, byte_limit=cost - 1)
        assert error.value.code == "PTC_LIMIT"


@pytest.mark.parametrize("value", ["", "ASCII", '"\\\b\f\n\r\t', "\x00\x1f\x7f", "中文😀", "\ud800\udfff"])
def test_unicode_escape_accounting(kernel, value):
    cost = len(json.dumps(value, ensure_ascii=True))
    assert kernel.copy(value, byte_limit=cost) == value
    with pytest.raises(ptc_values.PtcValueError, match="JSON byte limit"):
        kernel.validate(value, byte_limit=cost - 1)


@pytest.mark.parametrize("method", ["copy", "validate"])
@pytest.mark.parametrize(
    ("limits", "value", "message"),
    [
        ({"nodes": 2}, {"a": 1}, "node/depth"),
        ({"depth": 1}, [[[]]], "node/depth"),
        ({"items": 1}, [1, 2], "Collection"),
        ({"string_chars": 1}, "😀😀", "String"),
        ({"integer_bits": 8}, 256, "Integer"),
        ({"bytes": 10}, [0, 0, 0], "JSON byte"),
    ],
)
def test_each_resource_limit(kernel, method, limits, value, message):
    with pytest.raises(ptc_values.PtcValueError, match=message) as error:
        getattr(make_kernel(**limits), method)(value)
    assert error.value.code == "PTC_LIMIT"


@pytest.mark.parametrize("method", ["copy", "validate"])
@pytest.mark.parametrize("value", [float("nan"), float("inf"), (1,), {1: 2}, object()])
def test_invalid_json(kernel, method, value):
    with pytest.raises(ptc_values.PtcValueError) as error:
        getattr(kernel, method)(value)
    assert error.value.code == "PTC_VALUE"


def test_cycles_repeated_alias_cost_and_error_recovery(kernel):
    cyclic = []
    cyclic.append(cyclic)
    for operation in (kernel.copy, kernel.validate):
        with pytest.raises(ptc_values.PtcValueError, match="Cyclic"):
            operation(cyclic)
        with pytest.raises(ptc_values.PtcValueError, match="node/depth"):
            getattr(make_kernel(nodes=4), operation.__name__)([[0], [0]])
        assert operation([1]) == ([1] if operation.__name__ == "copy" else None)


def test_host_methods_are_never_called(kernel):
    touched = []

    class HostDict(dict):
        def items(self):
            touched.append("items")
            return super().items()

    class HostString(str):
        def __len__(self):
            touched.append("len")
            return super().__len__()

    for source in (HostDict(a=1), HostString("hello")):
        for operation in (kernel.copy, kernel.validate):
            with pytest.raises(ptc_values.PtcValueError, match="Only exact"):
                operation(source)
    assert touched == []


def test_expired_timeout_and_shared_kernel(kernel):
    with pytest.raises(ptc_values.PtcValueError) as error:
        kernel.copy([1], timeout_seconds=0)
    assert error.value.code == "PTC_TIMEOUT"
    source = {"a": [1, 2]}
    with ThreadPoolExecutor(max_workers=4) as pool:
        copies = list(pool.map(kernel.copy, [source] * 32))
    copies[0]["a"][0] = 9
    assert source["a"] == copies[1]["a"] == [1, 2]
    assert all(item is not source for item in copies)
