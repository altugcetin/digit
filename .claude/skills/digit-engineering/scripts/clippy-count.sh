#!/usr/bin/env bash
# Counts unique clippy/rustc warnings across the workspace (bin and test targets
# report the same warning twice; this counts it once).
#   clippy-count.sh            # total, then the files with the most warnings
#   clippy-count.sh FILE...    # only warnings in these files, with messages
set -euo pipefail
cd "$(git -C "$(dirname "$0")" rev-parse --show-toplevel)"
json=$(cargo clippy --workspace --all-targets --offline --message-format=json 2>/dev/null)
rows=$(jq -r 'select(.reason=="compiler-message") | .message | select(.level=="warning") | select(.spans|length>0)
  | (.spans[]|select(.is_primary)) as $s | "\($s.file_name):\($s.line_start)\t\(.code.code // "-")\t\(.message)"' <<<"$json" | sort -u)
if [[ $# -gt 0 ]]; then
    pattern=$(printf '%s\n' "$@" | sed 's/[.[\*^$]/\\&/g' | paste -sd'|')
    grep -E "^($pattern):" <<<"$rows" || echo "no warnings in: $*"
    exit 0
fi
echo "unique warnings: $(grep -c . <<<"$rows" || true)"
cut -d: -f1 <<<"$rows" | sort | uniq -c | sort -rn | head -12
