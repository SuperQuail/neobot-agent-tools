"""提供无状态的原生 JSON 额度检查与隔离复制，不访问工具或宿主环境。"""

from typing import Any

try:
    from . import _native
except (ImportError, OSError):
    _native = None


def native_available() -> bool:
    """旧版或缺失扩展交由宿主选择兼容回退。"""
    return getattr(_native, "PTC_VALUE_API_VERSION", None) == 1


class PtcValueError(ValueError):
    """携带稳定错误码，供宿主转换成自己的解释器异常。"""

    def __init__(self, code: str, message: str) -> None:
        super().__init__(message)
        self.code = code


class PtcValueKernel:
    """只保存不可变额度；每次遍历的计数、祖先集合和副本独立。"""

    def __init__(self, **limits: int) -> None:
        if not native_available():
            raise RuntimeError("Native PTC value kernel is unavailable")
        self._kernel = _native.PtcValueKernel(**limits)

    def copy(self, value: Any, *, byte_limit: int | None = None, timeout_seconds: float = 30.0) -> Any:
        """保持标量值，逐次复制容器，重复别名也分别复制。"""
        try:
            return self._kernel.copy(value, byte_limit=byte_limit, timeout_seconds=timeout_seconds)
        except _native.PtcValueError as exc:
            raise PtcValueError(*exc.args) from exc

    def validate(self, value: Any, *, byte_limit: int | None = None, timeout_seconds: float = 30.0) -> None:
        """完整计费但不分配容器副本。"""
        try:
            self._kernel.validate(value, byte_limit=byte_limit, timeout_seconds=timeout_seconds)
        except _native.PtcValueError as exc:
            raise PtcValueError(*exc.args) from exc
