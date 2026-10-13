## Sessions by client and condition

| Client (model) | Condition | Passed | Mean time | Median time | Mean agent tokens | Mean tool calls | Mean Oko calls | Ranker calls / session | Ranker input tokens / session | Median Oko search s |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| claude (claude-sonnet-5) | Without Oko | 26/27 | 18.7 s | 17.7 s | 65,518 | 5.3 | 0.0 | 0 | 0 | n/a |
| claude (claude-sonnet-5) | Oko + Jev | 24/27 | 7.0 s | 7.2 s | 23,982 | 0.8 | 0.1 | 1.1 | 15,327 | 0.25 |
| claude (claude-sonnet-5) | Oko + Nimble | 21/27 | 12.0 s | 8.1 s | 36,429 | 1.5 | 0.1 | 1.7 | 31,601 | 12.37 |
| codex (gpt-5.6-sol) | Without Oko | 27/27 | 23.4 s | 20.5 s | 73,195 | 3.2 | 0.0 | 0 | 0 | n/a |
| codex (gpt-5.6-sol) | Oko + Jev | 26/27 | 15.3 s | 13.9 s | 33,699 | 1.1 | 0.3 | 1.5 | 17,958 | 0.41 |
| codex (gpt-5.6-sol) | Oko + Nimble | 24/27 | 20.5 s | 20.9 s | 44,676 | 1.5 | 0.5 | 2.3 | 40,629 | 4.39 |
| opencode (claude-sonnet-5) | Without Oko | 22/27 | 13.6 s | 11.8 s | 43,051 | 4.4 | 0.0 | 0 | 0 | n/a |
| opencode (claude-sonnet-5) | Oko + Jev | 26/27 | 8.1 s | 7.4 s | 16,935 | 0.7 | 0.0 | 1.1 | 15,265 | 0.46 |
| opencode (claude-sonnet-5) | Oko + Nimble | 20/27 | 11.8 s | 10.6 s | 30,933 | 1.7 | 0.0 | 1.2 | 19,201 | 12.41 |
| pi (claude-sonnet-5) | Without Oko | 23/27 | 10.0 s | 9.7 s | 24,936 | 3.9 | 0.0 | 0 | 0 | n/a |
| pi (claude-sonnet-5) | Oko + Jev | 21/27 | 6.5 s | 6.2 s | 14,005 | 0.9 | 0.1 | 1.0 | 13,762 | 0.02 |
| pi (claude-sonnet-5) | Oko + Nimble | 20/27 | 9.3 s | 7.1 s | 20,589 | 1.5 | 0.1 | 1.6 | 29,076 | 12.59 |
| pi (gpt-5.6-sol) | Without Oko | 22/27 | 27.1 s | 27.3 s | 39,934 | 6.4 | 0.0 | 0 | 0 | n/a |
| pi (gpt-5.6-sol) | Oko + Jev | 25/27 | 15.7 s | 14.4 s | 11,310 | 1.3 | 0.4 | 2.0 | 27,002 | 0.47 |
| pi (gpt-5.6-sol) | Oko + Nimble | 23/27 | 20.8 s | 23.1 s | 13,673 | 1.9 | 0.5 | 2.1 | 41,286 | 6.70 |

## Paired changes

Same client, model, task and repetition. Negative is faster or fewer. Medians of per-pair percent changes; all attempts included.

| Client (model) | Change | Pairs | Time | Agent tokens | Faster in |
|---|---|---:|---:|---:|---:|
| claude (claude-sonnet-5) | Without Oko → Oko + Jev | 27 | -62% | -62% | 26/27 |
| claude (claude-sonnet-5) | Without Oko → Oko + Nimble | 27 | -39% | -45% | 21/27 |
| claude (claude-sonnet-5) | Oko + Jev → Oko + Nimble | 27 | 90% | 1% | 5/27 |
| codex (gpt-5.6-sol) | Without Oko → Oko + Jev | 27 | -38% | -53% | 21/27 |
| codex (gpt-5.6-sol) | Without Oko → Oko + Nimble | 27 | -11% | -38% | 15/27 |
| codex (gpt-5.6-sol) | Oko + Jev → Oko + Nimble | 27 | 48% | 11% | 7/27 |
| opencode (claude-sonnet-5) | Without Oko → Oko + Jev | 27 | -48% | -58% | 25/27 |
| opencode (claude-sonnet-5) | Without Oko → Oko + Nimble | 27 | -24% | -45% | 21/27 |
| opencode (claude-sonnet-5) | Oko + Jev → Oko + Nimble | 27 | 56% | 85% | 4/27 |
| pi (claude-sonnet-5) | Without Oko → Oko + Jev | 27 | -38% | -45% | 26/27 |
| pi (claude-sonnet-5) | Without Oko → Oko + Nimble | 27 | -18% | -22% | 16/27 |
| pi (claude-sonnet-5) | Oko + Jev → Oko + Nimble | 27 | 17% | 4% | 6/27 |
| pi (gpt-5.6-sol) | Without Oko → Oko + Jev | 27 | -47% | -67% | 23/27 |
| pi (gpt-5.6-sol) | Without Oko → Oko + Nimble | 27 | -20% | -60% | 23/27 |
| pi (gpt-5.6-sol) | Oko + Jev → Oko + Nimble | 27 | 54% | 23% | 8/27 |

## Prompt hook (prefetch) outcomes

| Client (model) | Condition | Sessions | Answer injected | Pointer only | Nothing | Why nothing | Median hook ms |
|---|---|---:|---:|---:|---:|---|---:|
| claude (claude-sonnet-5) | Oko + Jev | 27 | 27 | 0 | 0 | - | 410 |
| claude (claude-sonnet-5) | Oko + Nimble | 27 | 27 | 0 | 0 | - | 1,339 |
| codex (gpt-5.6-sol) | Oko + Jev | 27 | 27 | 0 | 0 | - | 409 |
| codex (gpt-5.6-sol) | Oko + Nimble | 27 | 27 | 0 | 0 | - | 1,326 |
| opencode (claude-sonnet-5) | Oko + Jev | 27 | 27 | 0 | 0 | - | 438 |
| opencode (claude-sonnet-5) | Oko + Nimble | 27 | 27 | 0 | 0 | - | 1,348 |
| pi (claude-sonnet-5) | Oko + Jev | 27 | 27 | 0 | 0 | - | 416 |
| pi (claude-sonnet-5) | Oko + Nimble | 27 | 27 | 0 | 0 | - | 1,360 |
| pi (gpt-5.6-sol) | Oko + Jev | 27 | 27 | 0 | 0 | - | 412 |
| pi (gpt-5.6-sol) | Oko + Nimble | 27 | 27 | 0 | 0 | - | 1,361 |

## Sessions that did not pass

| Client (model) | Condition | Task | Repetition | Reason |
|---|---|---|---:|---|
| claude (claude-sonnet-5) | Without Oko | ripgrep-capture-hyphen | 1 | Changes outside the requested implementation; review required |
| claude (claude-sonnet-5) | Oko + Jev | astro-forwarded-empty | 1 | Target missing/symlink or unrelated edits |
| claude (claude-sonnet-5) | Oko + Jev | astro-forwarded-empty | 2 | Target missing/symlink or unrelated edits |
| claude (claude-sonnet-5) | Oko + Jev | astro-forwarded-empty | 3 | Target missing/symlink or unrelated edits |
| claude (claude-sonnet-5) | Oko + Nimble | astro-forwarded-empty | 1 | Target missing/symlink or unrelated edits |
| claude (claude-sonnet-5) | Oko + Nimble | astro-forwarded-empty | 2 | Target missing/symlink or unrelated edits |
| claude (claude-sonnet-5) | Oko + Nimble | astro-forwarded-empty | 3 | Target missing/symlink or unrelated edits |
| claude (claude-sonnet-5) | Oko + Nimble | ripgrep-capture-hyphen | 1 | Target missing/symlink or unrelated edits |
| claude (claude-sonnet-5) | Oko + Nimble | ripgrep-capture-hyphen | 2 | Target missing/symlink or unrelated edits |
| claude (claude-sonnet-5) | Oko + Nimble | ripgrep-capture-hyphen | 3 | Target missing/symlink or unrelated edits |
| codex (gpt-5.6-sol) | Oko + Jev | astro-image-probe-authorization | 2 | grade failed |
| codex (gpt-5.6-sol) | Oko + Nimble | astro-forwarded-empty | 1 | Target missing/symlink or unrelated edits |
| codex (gpt-5.6-sol) | Oko + Nimble | astro-forwarded-empty | 2 | Target missing/symlink or unrelated edits |
| codex (gpt-5.6-sol) | Oko + Nimble | astro-forwarded-empty | 3 | Target missing/symlink or unrelated edits |
| opencode (claude-sonnet-5) | Without Oko | astro-forwarded-empty | 2 | Target missing/symlink or unrelated edits |
| opencode (claude-sonnet-5) | Without Oko | astro-forwarded-empty | 3 | Target missing/symlink or unrelated edits |
| opencode (claude-sonnet-5) | Without Oko | astro-image-probe-authorization | 1 | grade failed |
| opencode (claude-sonnet-5) | Without Oko | ripgrep-capture-hyphen | 2 | Target missing/symlink or unrelated edits |
| opencode (claude-sonnet-5) | Without Oko | ripgrep-printed-bytes | 2 | Extra data: line 2 column 1 (char 222) |
| opencode (claude-sonnet-5) | Oko + Jev | astro-forwarded-empty | 3 | Target missing/symlink or unrelated edits |
| opencode (claude-sonnet-5) | Oko + Nimble | astro-forwarded-empty | 1 | Target missing/symlink or unrelated edits |
| opencode (claude-sonnet-5) | Oko + Nimble | astro-forwarded-empty | 2 | Target missing/symlink or unrelated edits |
| opencode (claude-sonnet-5) | Oko + Nimble | astro-forwarded-empty | 3 | Target missing/symlink or unrelated edits |
| opencode (claude-sonnet-5) | Oko + Nimble | httpx-async-auth-body | 2 | grade failed |
| opencode (claude-sonnet-5) | Oko + Nimble | ripgrep-capture-hyphen | 1 | Target missing/symlink or unrelated edits |
| opencode (claude-sonnet-5) | Oko + Nimble | ripgrep-capture-hyphen | 2 | Target missing/symlink or unrelated edits |
| opencode (claude-sonnet-5) | Oko + Nimble | ripgrep-capture-hyphen | 3 | Target missing/symlink or unrelated edits |
| pi (claude-sonnet-5) | Without Oko | astro-forwarded-empty | 3 | Target missing/symlink or unrelated edits |
| pi (claude-sonnet-5) | Without Oko | astro-image-probe-authorization | 1 | grade failed |
| pi (claude-sonnet-5) | Without Oko | astro-image-probe-authorization | 2 | grade failed |
| pi (claude-sonnet-5) | Without Oko | astro-image-probe-authorization | 3 | grade failed |
| pi (claude-sonnet-5) | Oko + Jev | astro-forwarded-empty | 1 | Target missing/symlink or unrelated edits |
| pi (claude-sonnet-5) | Oko + Jev | astro-forwarded-empty | 2 | Target missing/symlink or unrelated edits |
| pi (claude-sonnet-5) | Oko + Jev | astro-forwarded-empty | 3 | Target missing/symlink or unrelated edits |
| pi (claude-sonnet-5) | Oko + Jev | astro-image-probe-authorization | 1 | grade failed |
| pi (claude-sonnet-5) | Oko + Jev | astro-image-probe-authorization | 2 | grade failed |
| pi (claude-sonnet-5) | Oko + Jev | astro-image-probe-authorization | 3 | grade failed |
| pi (claude-sonnet-5) | Oko + Nimble | astro-action-key-guards | 1 | grade failed |
| pi (claude-sonnet-5) | Oko + Nimble | astro-action-key-guards | 3 | grade failed |
| pi (claude-sonnet-5) | Oko + Nimble | astro-forwarded-empty | 1 | Target missing/symlink or unrelated edits |
| pi (claude-sonnet-5) | Oko + Nimble | astro-forwarded-empty | 2 | Target missing/symlink or unrelated edits |
| pi (claude-sonnet-5) | Oko + Nimble | astro-forwarded-empty | 3 | Target missing/symlink or unrelated edits |
| pi (claude-sonnet-5) | Oko + Nimble | httpx-async-auth-body | 1 | grade failed |
| pi (claude-sonnet-5) | Oko + Nimble | httpx-async-auth-body | 3 | grade failed |
| pi (gpt-5.6-sol) | Without Oko | httpx-decoder-chain | 1 | Result range exceeds file length: httpx/_decoders.py |
| pi (gpt-5.6-sol) | Without Oko | httpx-decoder-chain | 2 | Result range exceeds file length: httpx/_decoders.py |
| pi (gpt-5.6-sol) | Without Oko | ripgrep-printed-bytes | 1 | grade failed |
| pi (gpt-5.6-sol) | Without Oko | ripgrep-printed-bytes | 2 | grade failed |
| pi (gpt-5.6-sol) | Without Oko | ripgrep-printed-bytes | 3 | grade failed |
| pi (gpt-5.6-sol) | Oko + Jev | astro-image-probe-authorization | 1 | grade failed |
| pi (gpt-5.6-sol) | Oko + Jev | astro-image-probe-authorization | 3 | grade failed |
| pi (gpt-5.6-sol) | Oko + Nimble | astro-forwarded-empty | 1 | Target missing/symlink or unrelated edits |
| pi (gpt-5.6-sol) | Oko + Nimble | astro-forwarded-empty | 2 | Target missing/symlink or unrelated edits |
| pi (gpt-5.6-sol) | Oko + Nimble | astro-forwarded-empty | 3 | Target missing/symlink or unrelated edits |
| pi (gpt-5.6-sol) | Oko + Nimble | ripgrep-capture-expansion | 2 | grade failed |
