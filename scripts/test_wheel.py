"""在隔离的 uv 环境验证已构建 wheel，确保实际加载原生扩展。"""

import argparse
import subprocess
import sys
from pathlib import Path


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("directory", type=Path)
    parser.add_argument("--python", action="append", dest="pythons")
    args = parser.parse_args()
    wheels = list(args.directory.glob("*.whl"))
    if len(wheels) != 1:
        raise SystemExit("Expected exactly one platform wheel in the directory")
    for version in args.pythons or ["3.13", "3.14"]:
        subprocess.run(
            [
                "uv",
                "run",
                "--no-project",
                "--python",
                version,
                "--with",
                "pytest>=9,<10",
                "--with",
                str(wheels[0].resolve()),
                "python",
                "-B",
                "-m",
                "pytest",
                "tests",
                "--require-native",
                "-q",
            ],
            check=True,
            cwd=Path(__file__).resolve().parents[1],
        )
    return 0


if __name__ == "__main__":
    sys.exit(main())
