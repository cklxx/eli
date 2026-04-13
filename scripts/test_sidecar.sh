#!/usr/bin/env bash
set -euo pipefail

cd sidecar

npm install
npm test
npm run build
