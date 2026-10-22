#!/usr/bin/env bash
set -euo pipefail

# The installer writes into a disposable project; SM owns the final installation.
[ "$#" -eq 3 ]
source_url=$1
skill_name=$2
output_dir=$3
mkdir -p "$output_dir"
cd "$output_dir"
export DISABLE_TELEMETRY=1 DO_NOT_TRACK=1
exec npx --yes skills@1.7.2 add "$source_url" --skill "$skill_name" --agent codex --yes --copy
