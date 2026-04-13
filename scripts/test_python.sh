#!/usr/bin/env bash
set -euo pipefail

python3 -m pytest tests/test_basic.py -k 'version or status or help or invalid_subcommand or run_without_message' -v
