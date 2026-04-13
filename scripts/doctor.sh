#!/usr/bin/env bash
set -euo pipefail

status=0

check_cmd() {
  local name="$1"
  local cmd="$2"
  if command -v "$cmd" >/dev/null 2>&1; then
    printf '[ok] %s (%s)\n' "$name" "$cmd"
  else
    printf '[missing] %s (%s)\n' "$name" "$cmd"
    status=1
  fi
}

printf 'eli doctor\n\n'

check_cmd "rust toolchain" rustc
check_cmd "cargo" cargo
check_cmd "python" python3
check_cmd "node" node
check_cmd "npm" npm
check_cmd "bun (required for sidecar tests)" bun

if python3 -m pytest --version >/dev/null 2>&1; then
  printf '[ok] pytest\n'
else
  printf '[missing] pytest\n'
  status=1
fi

if [ -f sidecar/package.json ]; then
  printf '[ok] sidecar package.json\n'
else
  printf '[missing] sidecar package.json\n'
  status=1
fi

if [ -d sidecar/node_modules ]; then
  printf '[ok] sidecar dependencies installed\n'
else
  printf '[warn] sidecar dependencies not installed yet (run: cd sidecar && npm install)\n'
fi

if [ -f .env ]; then
  printf '[ok] .env present\n'
else
  printf '[warn] .env missing (copy env.example if needed)\n'
fi

exit "$status"
