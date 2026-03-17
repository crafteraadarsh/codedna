#!/usr/bin/env bash

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
skill_src="$repo_root/.opencode/skills/codedna"
config_dir="${OPENCODE_CONFIG_DIR:-$HOME/.config/opencode}"
skill_dst="$config_dir/skills/codedna"

if [[ ! -f "$skill_src/SKILL.md" ]]; then
  echo "OpenCode skill source not found at $skill_src" >&2
  exit 1
fi

mkdir -p "$config_dir/skills"
rm -rf "$skill_dst"
cp -R "$skill_src" "$skill_dst"

echo "Installed OpenCode skill to $skill_dst"
