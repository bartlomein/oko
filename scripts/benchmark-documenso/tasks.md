# Documenso: 3 searches + 2 edits

Pinned commit: `e658cc581878f52c03b3e6a8f7ffd613aead4e69`. Expected answers in tasks.json are never sent to the agents.

1. **search — pdf-page-count**: Where does the frontend read the PDF page count from the viewer DOM and return zero when the count is missing, non-integer, or below one?
2. **search — next-recipient**: Where does the backend choose the recipient immediately after the current one in signing order, excluding CC recipients and clearing the returned signing token?
3. **search — recipient-initials**: Where does shared code produce up to two uppercase initials for a recipient and fall back to the first email character when the name produces no initials?
4. **edit — document-search-delay**: In the frontend document-list search input, reduce the debounce delay from 500 ms to 300 ms. Keep template search, URL updates, and pagination-reset behavior unchanged.
5. **edit — first-reminder-delay**: Change the default envelope reminder settings so the first reminder is sent after 3 days instead of 5 days. Keep the default repeat interval, reminder limits, and explicitly configured schedules unchanged.
