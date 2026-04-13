#!/usr/bin/env bash
set -euo pipefail

./scripts/check.sh
./scripts/test_python.sh
./scripts/test_sidecar.sh
