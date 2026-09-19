# Twenty benchmark questions

10 read-only code-location questions and 5 small edits. Expected answers are in tasks.json and never included in agent prompts.

1. **search — search-normalization**: In the frontend, where is search text normalized to ignore accents and transliterate characters such as ø, æ, and ß?
2. **search — search-filter**: In the frontend, where are items filtered by matching a normalized query against any of their searchable values, while preserving all items for an empty query?
3. **search — rich-text-preview**: Where does the frontend extract the first nonempty text or link from rich-text blocks, skipping whitespace-only text?
4. **search — webhook-empty-row**: In the webhook settings UI, where is the empty operation row removed when both record and metadata catch-all operations are selected, and otherwise kept at the end?
5. **search — metadata-pagination**: Where does the backend reject metadata REST pagination requests that combine forward and backward cursors, contain non-UUID cursors, or specify fractional limits?
6. **search — gmail-retry-after**: Where does the backend extract a retry-after timestamp from a Gmail error message and reject invalid or already elapsed timestamps?
7. **search — webhook-event-match**: Where does the backend select configured webhooks whose subscribed operations match an incoming object event name?
8. **search — send-slot-backoff**: Where is the retry delay for acquiring an email send slot calculated using exponential backoff, a window-based ceiling, and jitter?
9. **search — locale-direction**: Where does shared code determine right-to-left versus left-to-right text direction from a locale, including locales with region subtags?
10. **search — short-number**: Where does shared code format signed numbers with k, m, or b suffixes, remove trailing decimal zeros, and round across magnitude boundaries?
11. **edit — preview-debounce**: In the frontend side-panel record search preview, increase the preview debounce from 200 ms to 300 ms. Preserve the rest of the preview behavior.
12. **edit — chart-dimming**: In frontend chart legend highlighting, increase the opacity of dimmed series from 0.2 to 0.35. Leave the highlighting logic unchanged.
13. **edit — preview-fields**: In the frontend side-panel search record preview, show at most five fields when collapsed instead of seven. Keep the expanded view unchanged.
14. **edit — email-retry-delay**: In the backend email-send retry policy, increase the initial exponential backoff delay from 5 seconds to 10 seconds. Keep the strategy and jitter unchanged.
15. **edit — trash-batch**: In backend trash cleanup, reduce the cleanup batch size from 1,000 records to 500. Preserve the cleanup selection and deletion behavior.
