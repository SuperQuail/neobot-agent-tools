import itertools
import random
from concurrent.futures import ThreadPoolExecutor

import pytest
from neobot_agent_tools import Snapshot, backend_info

BACKENDS = ["python", "auto"]
SAMPLES = [
    "",
    "a",
    "\n",
    "\n\n",
    "a\r\nb\rc\n",
    "汉😀\v\f\x1c\x1d\x1e\x85\u2028\u2029tail",
    "汉" * 255 + "\r\n😀tail",
    "x" * 65535 + "\r\nlast",
    "\r\r",
    "\n\r\r\n汉😀tail",
]


@pytest.mark.parametrize("backend", BACKENDS)
@pytest.mark.parametrize(
    "text",
    SAMPLES,
    ids=[
        "empty",
        "plain",
        "newline",
        "empty-lines",
        "mixed",
        "unicode",
        "checkpoint",
        "boundary",
        "repeat-cr",
        "adjacent-breaks",
    ],
)
def test_line_index_and_normalization_match_python(backend, text):
    snapshot = Snapshot.from_bytes(text.encode(), "v", backend=backend)
    lines = text.splitlines(keepends=True)
    assert snapshot.line_count == len(lines)
    assert [snapshot.line(i) for i in range(len(lines))] == lines
    assert [snapshot.line(i, keepends=False) for i in range(len(lines))] == text.splitlines()
    assert [snapshot.start(i) for i in range(len(lines) + 1)] == list(itertools.accumulate(map(len, lines), initial=0))
    normalized = ("\n".join(text.splitlines()) + "\n").encode() if lines else b""
    assert snapshot.for_search().search_bytes == normalized


@pytest.mark.parametrize("backend", BACKENDS)
def test_long_unicode_line_paginates_without_loss(backend):
    text = "汉😀" * 50000 + "\r\nsecond\nend"
    snapshot = Snapshot.from_bytes(text.encode(), "version", backend=backend)
    args, pages, following = {"limit": 1}, [], []
    while True:
        page = snapshot.read_page(**args)
        output = page.as_dict()
        assert len(output["content"].encode()) <= 65536
        pages.append(output["content"])
        following.append((page.start, page.end))
        if output["next_offset"] is None:
            break
        args.update(offset=output["next_offset"], column=output["next_column"])
    assert "".join(pages) == text
    assert all(a[1] == b[0] for a, b in itertools.pairwise(following))
    assert following[-1][1] == len(text)


@pytest.mark.parametrize("alphabet", ["a汉😀\r\n", "a汉😀\r\n\v\f\x1c\x1d\x1e\x85\u2028\u2029"])
def test_random_pages_have_identical_rust_and_python_results(alphabet):
    rng = random.Random(20261027)
    for _ in range(80):
        text = "".join(rng.choice(alphabet) for _ in range(rng.randrange(1000)))
        old = Snapshot.from_bytes(text.encode(), "v", backend="python")
        new = Snapshot.from_bytes(text.encode(), "v")
        assert old.for_search().search_bytes == new.for_search().search_bytes
        for _ in range(10):
            offset = rng.randrange(old.line_count + 3) + 1
            length = len(old.line(offset - 1)) if offset <= old.line_count else 0
            args = {
                "offset": offset,
                "column": rng.randrange(length + 1),
                "limit": rng.randrange(1, 10),
                "max_bytes": rng.randrange(30),
            }
            assert old.read_page(**args) == new.read_page(**args)


@pytest.mark.parametrize("backend", BACKENDS)
@pytest.mark.parametrize("data", [b"\xff", b"a\xe2\x80", b"\xed\xa0\x80"])
def test_invalid_utf8_never_becomes_a_preview(backend, data):
    with pytest.raises(UnicodeDecodeError):
        Snapshot.from_bytes(data, "v", backend=backend)


@pytest.mark.parametrize(
    "args", [{"offset": 0}, {"limit": True}, {"limit": 2001}, {"column": 100}, {"max_bytes": 65537}]
)
def test_invalid_page_arguments_fail_in_both_backends(args):
    for backend in BACKENDS:
        with pytest.raises(ValueError):
            Snapshot.from_bytes(b"text", "v", backend=backend).read_page(**args)


def test_storage_is_shared_for_copies_and_search_preparation():
    snapshot = Snapshot.from_bytes("汉😀\r\n".encode(), "v")
    assert snapshot is not snapshot.share()
    assert snapshot.shares_storage(snapshot.share())
    assert snapshot.shares_storage(snapshot.for_search())


def test_shared_snapshot_can_be_read_from_eight_threads():
    snapshot = Snapshot.from_bytes(("汉😀\r\n" * 1000).encode(), "v")
    with ThreadPoolExecutor(max_workers=8) as pool:
        pages = list(pool.map(lambda _: snapshot.share().read_page(limit=2), range(40)))
    assert all(page == pages[0] for page in pages)


def test_input_size_limit_is_checked_before_snapshot_construction():
    with pytest.raises(ValueError, match="16777216"):
        Snapshot.from_bytes(b"x" * (16 * 1024 * 1024 + 1), "v")


def test_backend_status_is_explicit():
    info = backend_info()
    assert info["api_version"] == 1 and info["backend"] in {"rust", "python"}
