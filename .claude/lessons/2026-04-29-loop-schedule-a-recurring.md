# # /loop — schedule a recurring

- **Date**: 2026-04-29
- **Trigger**: Keyword(`stop`)
- **Evidence Count**: 1
- **Status**: Observation

## Root Cause Depth

Layer 1/5 — 表面症状

## Context

User: # /loop — schedule a recurring or self-paced prompt

Parse the input below into `[interval] <prompt…>` and schedule it.

## Parsing (in priority order)

1. **Leading token**: if the first whitespace-delimited token matches `^\d+[smhd]$` (e.g. `5m`, `2h`), that's the interval; the rest is the prompt.
2. **Trailing "every" clause**: otherwise, if the input ends with `every <N><unit>` or `every <N> <unit-word>` (e.g. `every 20m`, `every 5 minutes`, `every 2 hours`), extract that as the interval and strip it from the prompt. Only match when what follows "every" is a time expression — `check every PR` has no interval.
3. **No interval**: otherwise, the entire input

[…截断]

## Why

<待人工补充>

## How to apply

<待人工补充>

## Takeaway

检测到 `stop` 信号。待人工补充结论。
