#!/usr/bin/env bash

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
skill_src="$repo_root/skill/codedna"
codex_home="${CODEX_HOME:-$HOME/.codex}"
skill_dst="$codex_home/skills/codedna"

if [[ ! -f "$skill_src/SKILL.md" ]]; then
  echo "Skill source not found at $skill_src" >&2
  exit 1
fi

mkdir -p "$codex_home/skills"
rm -rf "$skill_dst"
cp -R "$skill_src" "$skill_dst"

echo "Installed Codex skill to $skill_dst"
