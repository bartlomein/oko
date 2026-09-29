## Oko code search
Use the Oko MCP `search` tool first to locate unfamiliar code or where a behavior is implemented. Ask in the user's own terms and scope; do not add guessed frameworks or pipeline stages. For edits, describe the existing code to change; the new text need not exist yet. A function, class or method named in the question is always returned. When the task asks for every use of a name, or its tests, ask "who calls X" or "tests for X". When a task has several separate parts, put all of them in one call's `questions` (2–8) instead of one call each; to see named definitions you have not seen yet, pass their names in `symbols`. When the task asks for every file that depends on a definition, or its specs, ask "who uses X": each row is a path:line you can cite. For a deletion list, ask "unused definitions in <directory>". Use native grep only for literal text.

| Looking for | Use |
| --- | --- |
| Where something happens, how a flow works | Oko `search` |
| Where a function, type, or setting is defined or used | Oko `search` |
| An exact string: error message, log line, config key | grep |
When you hand code search to a subagent, tell it to locate code with the Oko MCP `search` tool first; subagents do not see these instructions.

Oko returns exact, current file contents with real line numbers: the same text a file read would print. Treat each excerpt as a file read you have already done. A `Files using X` or `Callers of X` listing is complete for the indexed code: cite its rows as they are and do not grep for the name again; it names what it cannot see.

After an Oko search, if you can name the exact code to cite or change, act: answer or edit. If the excerpts show more than one definition that could be meant, such as copies of a helper in different packages, decide which one the request names before editing; the first result is the best match, not always the intended one. Usually that is one Oko call and at most one follow-up read. Search again only for a part of the task no excerpt shows; do not re-ask shown code through `questions` or `symbols`. Read only what Oko did not show: the rest of a `partial excerpt`, a path under `Other candidates`, or code the excerpts reference but do not include. A `possible match` was rated below the relevance cutoff.

- Good: Oko search, then answer with the returned `path:line` ranges.
- Good: Oko search, then edit the returned lines (drop the line-number prefix).
- Bad: Oko search, then `sed`, `nl`, `cat`, or a read of the same range to verify it.
- Bad: Oko search, then grep for a name the excerpts already show.
- Bad: Oko search, then a second Oko call (`questions` or `symbols`) for code the first answer already shows.

When you answer in prose from search results, say inside the answer what you did not check: a path you did not follow, a caller you did not read, a file the excerpts only referenced. Do not present a partial trace as complete. When the user asks for a fixed format (JSON, a single value, a patch), return exactly that and nothing after it.

Start with normal search; use deep mode only if it was insufficient. If Oko is unavailable or returns nothing relevant, fall back to native search.
