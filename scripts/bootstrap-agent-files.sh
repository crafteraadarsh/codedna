#!/usr/bin/env bash
# Add the CodeDna skill to a repository (or to Codex globally).
#
#   scripts/bootstrap-agent-files.sh <target-repo> [agent...]
#
# Agents:
#   claude    .claude/skills/codedna and a CLAUDE.md (if the repo has none)
#   codex     ~/.codex/skills/codedna (global)
#   opencode  .opencode/skills/codedna
#   agents    .agents/skills/codedna (Antigravity, and Codex per repository)

set -euo pipefail

usage() {
  echo "Usage: $0 <target-repo-path> [claude] [codex] [opencode] [agents]" >&2
  echo "Default agents: claude opencode agents" >&2
}

if [[ $# -lt 1 ]]; then
  usage
  exit 1
fi

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
skill_src="$repo_root/skill/codedna"
target_repo="$1"
shift

if [[ ! -d "$target_repo" ]]; then
  echo "Target repo not found: $target_repo" >&2
  exit 1
fi

declare -a agents
if [[ $# -eq 0 ]]; then
  agents=("claude" "opencode" "agents")
else
  agents=("$@")
fi

install_skill() {
  local dst="$1/codedna"
  mkdir -p "$1"
  rm -rf "$dst"
  mkdir -p "$dst"
  cp "$skill_src/SKILL.md" "$dst/SKILL.md"
  echo "Installed skill to $dst"
}

for agent in "${agents[@]}"; do
  case "$agent" in
    claude)
      install_skill "$target_repo/.claude/skills"
      if [[ -e "$target_repo/CLAUDE.md" ]]; then
        echo "Kept existing $target_repo/CLAUDE.md"
      else
        cp "$repo_root/templates/CLAUDE.md" "$target_repo/CLAUDE.md"
        echo "Wrote $target_repo/CLAUDE.md"
      fi
      ;;
    codex)
      codex_skills="${CODEX_HOME:-$HOME/.codex}/skills"
      mkdir -p "$codex_skills"
      rm -rf "$codex_skills/codedna"
      cp -R "$skill_src" "$codex_skills/codedna"
      echo "Installed skill to $codex_skills/codedna"
      ;;
    opencode)
      install_skill "$target_repo/.opencode/skills"
      ;;
    agents | agy | antigravity)
      install_skill "$target_repo/.agents/skills"
      ;;
    *)
      echo "Unknown agent: $agent" >&2
      usage
      exit 1
      ;;
  esac
done
