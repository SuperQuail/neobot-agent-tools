import gc
import threading
import time
import weakref

import pytest
from neobot_agent_tools import FileIoResources, FileResourceError


def test_shared_working_budget_times_out_and_is_released_after_exception():
    budget = FileIoResources(working_bytes=100, operations=2)
    with budget.operation(60, deadline=time.monotonic() + 1):
        with pytest.raises(FileResourceError), budget.operation(50, deadline=time.monotonic() + 0.01):
            pytest.fail("over-budget operation admitted")
        with budget.operation(40, deadline=time.monotonic() + 1):
            assert budget.status()["working_bytes"] == 100
    assert budget.status()["working_bytes"] == 0
    with pytest.raises(RuntimeError), budget.operation(90, deadline=time.monotonic() + 1):
        raise RuntimeError("operation failed")
    assert budget.status()["active_operations"] == 0


def test_one_sweeper_reclaims_idle_entries_from_multiple_caches():
    budget = FileIoResources(sweep_seconds=0.01)

    class Cache:
        def __init__(self):
            self.token = budget.register(self)
            self.expired = threading.Event()
            self.retained = False

        def expire(self):
            if self.retained:
                budget.release(self.token, 100)
                self.retained = False
                self.expired.set()

    caches = [Cache(), Cache()]
    for cache in caches:
        cache.retained = True
        assert budget.retain(cache.token, 100)
    assert all(cache.expired.wait(1) for cache in caches)
    assert budget.status()["cached_bytes"] == 0


def test_cache_gc_releases_shared_quota():
    budget = FileIoResources(sweep_seconds=60)

    class Cache:
        def expire(self):
            pass

    cache = Cache()
    token = budget.register(cache)
    assert budget.retain(token, 100)
    reference = weakref.ref(cache)
    del cache
    gc.collect()
    assert reference() is None
    assert budget.status()["cached_bytes"] == budget.status()["cached_entries"] == 0
