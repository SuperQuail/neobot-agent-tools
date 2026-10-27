import pytest
from neobot_agent_tools import backend_info


def pytest_addoption(parser):
    parser.addoption("--require-native", action="store_true", help="Fail unless the installed wheel loads Rust")


@pytest.fixture(scope="session", autouse=True)
def check_required_backend(request):
    if request.config.getoption("--require-native"):
        assert backend_info()["backend"] == "rust", backend_info()
