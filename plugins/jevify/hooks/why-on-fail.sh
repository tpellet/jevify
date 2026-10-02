#!/bin/sh
# Claude Code PostToolUseFailure hook for the Bash tool (plugins/jevify/hooks/hooks.json).
# `jevify why --hook claude` reads the hook payload on stdin, judges the failed output when
# it has at least JEVIFY_HOOK_MIN_LINES lines (default 80), and prints the hook JSON with the
# line it points at, or nothing. It always exits 0, so it never blocks a tool call, and it
# stops after 20 seconds. It never reads the API key: jevify reads TYPESAFE_API_KEY or
# TYPESAFE_API_KEY_FILE from the session's environment, or runs keyless.
command -v jevify >/dev/null 2>&1 || exit 0
jevify why --hook claude 2>/dev/null
exit 0
