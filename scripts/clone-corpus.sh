#!/usr/bin/env bash
# Clone the repositories of one or more calibration sets at their pinned
# commits, for `codedna calibration --corpus DIR`.
#
#   scripts/clone-corpus.sh DIR calibration/corpus.json calibration/fresh/corpus.json
#
# Each repository is fetched at depth 1 and keeps its .git folder. Existing
# clones are kept. Needs git and jq.
set -euo pipefail

if [ "$#" -lt 2 ]; then
  echo "usage: $0 DIR CORPUS.json..." >&2
  exit 2
fi
dir=$1
shift
mkdir -p "$dir"

clone_one() {
  local dir=$1 repo=$2 url=$3 commit=$4
  local target="$dir/$repo"
  if [ -d "$target/.git" ] && [ "$(git -C "$target" rev-parse HEAD 2>/dev/null)" = "$commit" ]; then
    return 0
  fi
  local attempt
  for attempt in 1 2 3; do
    rm -rf "$target"
    if git init -q "$target" &&
      git -C "$target" fetch -q --depth 1 "$url" "$commit" &&
      git -C "$target" -c advice.detachedHead=false checkout -q FETCH_HEAD; then
      return 0
    fi
    echo "retrying $repo ($attempt)" >&2
    sleep $((attempt * 5))
  done
  echo "could not clone $repo" >&2
  return 1
}
export -f clone_one

# The single-quoted arguments are expanded by the inner bash.
# shellcheck disable=SC2016
jq -r '.[] | [.repo, .url, .commit] | @tsv' "$@" |
  xargs -P 8 -L 1 bash -c 'clone_one "$0" "$1" "$2" "$3"' "$dir"
