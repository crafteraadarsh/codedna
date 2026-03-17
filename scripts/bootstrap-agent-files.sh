#!/usr/bin/env bash

set -euo pipefail

usage() {
  echo "Usage: $0 <target-repo-path> [codex] [opencode] [claude]" >&2
  echo "Default agents: opencode claude" >&2
}

if [[ $# -lt 1 ]]; then
  usage
  exit 1
fi

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
target_repo="$1"
shift || true

if [[ ! -d "$target_repo" ]]; then
  echo "Target repo not found: $target_repo" >&2
  exit 1
fi

declare -a agents
if [[ $# -eq 0 ]]; then
  agents=("opencode" "claude")
else
  agents=("$@")
fi

copy_file() {
  local src="$1"
  local dst="$2"
  cp "$src" "$dst"
  echo "Wrote $dst"
}

for agent in "${agents[@]}"; do
  case "$agent" in
    codex)
      mkdir -p "${CODEX_HOME:-$HOME/.codex}/skills"
      rm -rf "${CODEX_HOME:-$HOME/.codex}/skills/codedna"
      cp -R "$repo_root/skill/codedna" "${CODEX_HOME:-$HOME/.codex}/skills/codedna"
      echo "Installed Codex skill to ${CODEX_HOME:-$HOME/.codex}/skills/codedna"
      ;;
    opencode)
      copy_file "$repo_root/templates/AGENTS.md" "$target_repo/AGENTS.md"
      ;;
    claude)
      copy_file "$repo_root/templates/CLAUDE.md" "$target_repo/CLAUDE.md"
      ;;
    *)
      echo "Unknown agent: $agent" >&2
      usage
      exit 1
      ;;
  esac
done
