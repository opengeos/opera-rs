"""Smoke tests for the opera-rs Python package."""

import opera_rs


def test_version() -> None:
    assert isinstance(opera_rs.__version__, str)
    assert opera_rs.__version__ == opera_rs.version()
