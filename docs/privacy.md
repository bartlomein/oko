# Privacy

[← Back to Oko](../README.md)

- **Local:** file discovery, indexing and the first keyword and symbol search
  run on your machine. Prepared search data is cached outside your repository.
- **Sent to TypeSafe AI:** Jev ranking sends your question and selected source
  snippets. With setup's prompt hook, a prompt that reads as a code question is
  searched as you send it, so its first paragraph is the question. Deep mode
  (`oko ask --deep`) can send more snippets across several requests.
- **Local only:** `--no-jev` searches without any network request.
- **Update check:** once a day Oko asks GitHub for the latest release number,
  and nothing else. `--no-jev` and `OKO_NO_UPDATE_CHECK=1` turn it off.
- **Cost log:** each Jev request's time, workspace path, phase and token counts
  are logged locally for `oko stats`, never the question or code.
  `OKO_NO_JEV_LOG=1` turns it off.
- **Your files:** Oko does not edit your source files.

Key storage and cache controls are in [configuration](configuration.md).
