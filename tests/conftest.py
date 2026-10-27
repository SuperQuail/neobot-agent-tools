import pytest
from neobot_agent_tools import backend_info
from neobot_agent_tools.ptc_values import native_available


def pytest_addoption(parser):
    parser.addoption("--require-native", action="store_true", help="Fail unless the installed wheel loads Rust")


@pytest.fixture(scope="session", autouse=True)
def check_required_backend(request):
    if request.config.getoption("--require-native"):
        assert backend_info()["backend"] == "rust", backend_info()
        assert native_available(), "The wheel must include the PTC value kernel"
