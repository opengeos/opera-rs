"""Command-line entry point for the ``opera`` CLI and MCP server via Python."""

from __future__ import annotations

import signal
import sys


def main() -> int:
    """Run the ``opera`` CLI (including ``opera mcp``) using the bundled Rust implementation."""
    from opera_rs._opera_rs import cli_main

    if hasattr(signal, "SIGPIPE"):
        signal.signal(signal.SIGPIPE, signal.SIG_DFL)
    signal.signal(signal.SIGINT, signal.SIG_DFL)
    # `opera mcp` invokes each tool as `python -m opera_rs <subcommand>`.
    return cli_main(["opera", *sys.argv[1:]], [sys.executable, "-m", "opera_rs"])


if __name__ == "__main__":
    sys.exit(main())
