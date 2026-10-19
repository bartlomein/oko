# Cursor: 54 sessions (October 2026)

`sessions.json` holds every session of the Cursor run described in
[docs/benchmark-results.md](../../../docs/benchmark-results.md#cursor-54-sessions-october-2026):
nine tasks, three repetitions, Cursor alone (`native`) and Cursor with Oko as
`oko setup --client cursor` leaves the project (`oko`).

Per session: task, condition, repetition, wall seconds from sending the prompt
to the end of the turn in Cursor's transcript, the tool calls in that
transcript, the grade from the public runner's graders, the final answer, and
Cursor's request trace (time to first token, thinking steps). `tokensK` and
`requests` come from Cursor's usage export (one row per model request, to the
minute), matched to sessions by time overlap; each session's share of a minute
it shared with another is split by overlap. All 7.9 million exported tokens
for the run were matched. No transcripts or local paths are included.
