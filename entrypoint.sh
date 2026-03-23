#!/bin/bash

set -eo pipefail

if [ -f "/workspace/eli-reqs.txt" ]; then
    echo "Installing additional requirements from /workspace/eli-reqs.txt"
    uv pip install -r /workspace/eli-reqs.txt -p /app/.venv/bin/python
fi

source /app/.venv/bin/activate
if [ -f "/workspace/startup.sh" ]; then
    exec bash /workspace/startup.sh
else
    exec /app/.venv/bin/eli gateway
fi
