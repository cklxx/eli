#!/usr/bin/env bash
# deploy.sh — Deploy a directory to Cloudflare Pages
# Usage: deploy.sh <directory> <project-name>
# Returns the live URL on success, exits non-zero on failure.

set -euo pipefail

DEPLOY_DIR="${1:?Usage: deploy.sh <directory> <project-name>}"
PROJECT_NAME="${2:?Usage: deploy.sh <directory> <project-name>}"

if [ ! -f "$DEPLOY_DIR/index.html" ]; then
  echo "ERROR: $DEPLOY_DIR/index.html not found" >&2
  exit 1
fi

if ! command -v wrangler &>/dev/null; then
  echo "ERROR: wrangler not installed. Run: npm install -g wrangler" >&2
  exit 1
fi

# Create project if it doesn't exist (idempotent — suppress "already exists" error)
npx wrangler pages project create "$PROJECT_NAME" --production-branch=main 2>&1 | grep -v "already exists" >&2 || true

# Deploy
OUTPUT=$(npx wrangler pages deploy "$DEPLOY_DIR" --project-name="$PROJECT_NAME" --commit-dirty=true 2>&1)
echo "$OUTPUT" >&2

# Extract URL — wrangler prints: "Take a peek over at https://xxx.pages.dev"
URL=$(echo "$OUTPUT" | grep -oE 'https://[^ ]+\.pages\.dev' | tail -1)

if [ -z "$URL" ]; then
  echo "ERROR: Could not extract deployment URL from wrangler output" >&2
  exit 1
fi

# Also print the production URL
PROD_URL="https://${PROJECT_NAME}.pages.dev"
echo ""
echo "Deploy complete!"
echo "  Preview:    $URL"
echo "  Production: $PROD_URL"
