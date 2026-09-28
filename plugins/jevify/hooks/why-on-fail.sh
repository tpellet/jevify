#!/bin/sh
# Claude Code PostToolUseFailure hook for the Bash tool (plugins/jevify/hooks/hooks.json).
#
# A failed Bash call reaches this hook as JSON on stdin; its `error` field holds
# "Exit code N" and the output Claude sees. When that output has at least
# JEVIFY_HOOK_MIN_LINES lines (default 80), `jevify why` reads it and the hook
# prints hookSpecificOutput.additionalContext: the line it points at, with
# context, and the path of the saved output.
#
# It prints nothing and exits 0 when jq or jevify is not on PATH, the output is
# shorter, the call was interrupted, jevify abstains (3), is unavailable (4),
# fails authentication (5) or any other way, or takes longer than 20 seconds.
# It always exits 0, so it never blocks. It never reads the API key: jevify
# reads TYPESAFE_API_KEY or TYPESAFE_API_KEY_FILE from the session's
# environment, or runs keyless. jq is required and not replaced by a fallback.
set -u
exec 2>/dev/null

command -v jq >/dev/null || exit 0
command -v jevify >/dev/null || exit 0

min=${JEVIFY_HOOK_MIN_LINES:-80}
case $min in '' | *[!0-9]*) min=80 ;; esac

log=$(jq -r 'select(.tool_name == "Bash" and ((.is_interrupt // false) | not))
  | .error // empty | select(type == "string") | sub("^Exit code [0-9]+\n"; "")') || exit 0
[ -n "$log" ] || exit 0
lines=$(printf '%s\n' "$log" | wc -l | tr -d ' ')
[ "$lines" -ge "$min" ] || exit 0

# jevify gets 20 seconds; the watchdog is stopped as soon as jevify exits.
answer=$(
  printf '%s\n' "$log" | jevify why --json &
  pid=$!
  (sleep 20 && kill "$pid") >/dev/null &
  dog=$!
  wait "$pid"
  rc=$?
  kill "$dog"
  exit "$rc"
) || exit 0

printf '%s' "$answer" | jq -c --arg lines "$lines" '
  select(.exit_code == 0 and ((.data.causes // []) | length) > 0) | .data as $d
  | $d.causes[0] as $c
  | ($c.context | map(
      (.line | tostring) as $n
      | (if .line == $c.line then ">" else " " end)
        + ((" " * (6 - ($n | length))) // "") + $n + " │ " + (.text | .[0:300])
    ) | join("\n")) as $excerpt
  | {hookSpecificOutput: {hookEventName: "PostToolUseFailure", additionalContext:
      ("jevify why points at line \($c.line) of the \($lines) output lines of this failed command as the cause"
       + " (" + (if $d.saved_input then "output saved at \($d.saved_input)" else "output not saved" end)
       + "):\n" + $excerpt)}}
'
exit 0
