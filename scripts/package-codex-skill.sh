#!/usr/bin/env bash

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
skill_src="$repo_root/skill/codedna"
dist_dir="$repo_root/dist"
archive="$dist_dir/codedna-skill.tgz"

if [[ ! -f "$skill_src/SKILL.md" ]]; then
  echo "Skill source not found at $skill_src" >&2
  exit 1
fi

mkdir -p "$dist_dir"
tar -C "$repo_root/skill" -czf "$archive" codedna

echo "Packaged Codex skill at $archive"
