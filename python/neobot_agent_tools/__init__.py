"""提供可复用的有界 Agent 快照，以及进程内资源协调。"""

from ._version import API_VERSION, __version__
from .resources import FILE_IO_RESOURCES, FileChangedDuringAdmission, FileIoResources, FileResourceError
from .snapshots import ReadPage, Snapshot, backend_info

__all__ = [
    "API_VERSION",
    "FILE_IO_RESOURCES",
    "FileChangedDuringAdmission",
    "FileIoResources",
    "FileResourceError",
    "ReadPage",
    "Snapshot",
    "__version__",
    "backend_info",
]
