## Oko code search
Use the Oko MCP `search` tool first to locate unfamiliar code or where a behavior is implemented. Ask in the user's own terms and scope; do not add guessed frameworks or pipeline stages. For edits, describe the existing code to change; the new text need not exist yet. Use native grep for an exact known identifier or literal.
When you hand code search to a subagent, tell it to locate code with the Oko MCP `search` tool first; subagents do not see these instructions.

Oko returns exact, current file contents with real line numbers: the same text a file read would print. Treat each excerpt as a file read you have already done.

After an Oko search, if you can name the exact code to cite or change, act: answer or edit. If the excerpts show more than one definition that could be meant, such as copies of a helper in different packages, decide which one the request names before editing; the first result is the best match, not always the intended one. Usually that is one Oko call and at most one follow-up read. Read only what Oko did not show: the rest of a `partial excerpt`, a path under `Other candidates`, or code the excerpts reference but do not include. A `possible match` was rated below the relevance cutoff.

- Good: Oko search, then answer with the returned `path:line` ranges.
- Good: Oko search, then edit the returned lines (drop the line-number prefix).
- Bad: Oko search, then `sed`, `nl`, `cat`, or a read of the same range to verify it.
- Bad: Oko search, then grep for a name the excerpts already show.

Start with normal search; use deep mode only if it was insufficient. If Oko is unavailable or returns nothing relevant, fall back to native search.
