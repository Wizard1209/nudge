#!/usr/bin/env bash
#
# Run the #[ignore]'d DESKTOP native tests (tests/native_render.rs) — including
# the LLM-as-judge vision test — against the real nudge.exe window.
#
# These tests pop a real window and screen-capture it, so they only work on a
# Windows desktop (run from WSL). The LLM judge additionally needs an OpenAI
# key. WSL does NOT forward env vars to a windows-gnu child process unless they
# are named in WSLENV, so this script loads .env and re-exports the relevant
# vars through WSLENV — that's the whole reason the test couldn't see
# OPENAI_API_KEY when passed inline.
#
# Usage:
#   scripts/native-tests.sh                 # run all ignored native tests
#   scripts/native-tests.sh llm             # filter by test-name substring
#   NUDGE_DUMP_PNG='C:\Users\me\cap.png' scripts/native-tests.sh   # also dump
#
# Provide OPENAI_API_KEY via .env (gitignored) or the environment. See
# .env.example.
set -euo pipefail

cd "$(dirname "$0")/.."

# Load .env (KEY=VALUE lines) if present; -a auto-exports everything sourced.
if [ -f .env ]; then
    set -a
    # shellcheck disable=SC1091
    . ./.env
    set +a
fi

if [ -z "${OPENAI_API_KEY:-}" ]; then
    echo "warning: OPENAI_API_KEY not set (.env or env) — the LLM-judge test will SKIP." >&2
fi

# Forward to the Windows test process via WSLENV (WSL won't otherwise). Each
# var is appended only when set: OPENAI_JUDGE_MODEL (e.g. gpt-4o) and
# NUDGE_DUMP_PNG (a Windows path, e.g. C:\...\cap.png, to dump the judged image).
WSLENV="OPENAI_API_KEY"
WSLENV="${WSLENV}${OPENAI_JUDGE_MODEL:+:OPENAI_JUDGE_MODEL}"
WSLENV="${WSLENV}${OPENAI_JUDGE_SCALE:+:OPENAI_JUDGE_SCALE}"
WSLENV="${WSLENV}${OPENAI_JUDGE_DETAIL:+:OPENAI_JUDGE_DETAIL}"
WSLENV="${WSLENV}${NUDGE_DUMP_PNG:+:NUDGE_DUMP_PNG}"
export WSLENV

exec cargo test --target x86_64-pc-windows-gnu --test native_render \
    -- --ignored --nocapture "$@"
