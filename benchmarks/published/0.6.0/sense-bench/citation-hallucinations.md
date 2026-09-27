# Citation hallucinations

Citations the assistant printed that did not resolve against the repo checked out at `run_meta.repo_commit`. **Hallucinated** = line number beyond EOF (made-up number). **Unresolved** = file not in repo, or symbol not within ±5 lines of the cited line.

Not yet folded into the fairness score.

## oko-dev

### oko-dev/discourse  - 210/211 grounded

**Unresolved**
- `Upload.rb:76` - file not found at Upload.rb

### oko-dev/discourse  - 260/262 grounded

**Unresolved**
- `.../tool_runner/upload.rb:6` - file not found at .../tool_runner/upload.rb
- `plugins/discourse-calendar/.../event.rb:346` - file not found at plugins/discourse-calendar/.../event.rb

### oko-dev/nextjs  - 36/37 grounded

**Unresolved**
- `edge-ssr-app.ts:267` - file not found at edge-ssr-app.ts

### oko-dev/nextjs  - 41/42 grounded

**Unresolved**
- `index.ts:class` - ambiguous: 99 files match `index.ts` (crates/next-custom-transforms/tests/fixture/server-actions/index.ts, crates/next-custom-transforms/tests/fixture/track-dynamic-imports/index.ts, test/production/typescript-basic/typechecking/index.ts...)

## sense

### sense/discourse  - 279/280 grounded

**Unresolved**
- `app/models/reports/top_uploads.rb:7` - file not found at app/models/reports/top_uploads.rb

### sense/discourse  - 259/260 grounded

**Unresolved**
- `.../update_message.rb:97` - file not found at .../update_message.rb

### sense/discourse  - 258/262 grounded

**Unresolved**
- `.../ai_tool.rb:3` - file not found at .../ai_tool.rb
- `.../rag_document_fragment.rb:3` - file not found at .../rag_document_fragment.rb
- `.../dialects/dialect.rb:85` - file not found at .../dialects/dialect.rb
- `.../endpoints/gemini.rb:315` - file not found at .../endpoints/gemini.rb

### sense/javalin  - 59/77 grounded

**Unresolved**
- `.../config/JavalinConfig.kt:24` - file not found at .../config/JavalinConfig.kt
- `.../config/JavalinState.kt:47` - file not found at .../config/JavalinState.kt
- `.../http/servlet/JavalinServlet.kt:31` - file not found at .../http/servlet/JavalinServlet.kt
- `.../router/InternalRouter.kt:26` - file not found at .../router/InternalRouter.kt
- `.../router/matcher/PathMatcher.kt:13` - file not found at .../router/matcher/PathMatcher.kt
- `.../http/Context.kt:58` - file not found at .../http/Context.kt
- `.../http/servlet/JavalinServletContext.kt:69` - file not found at .../http/servlet/JavalinServletContext.kt
- `.../http/Handler.java:19` - file not found at .../http/Handler.java
- `.../config/RoutesConfig.kt:24` - file not found at .../config/RoutesConfig.kt
- `.../router/JavalinDefaultRoutingApi.kt:38` - file not found at .../router/JavalinDefaultRoutingApi.kt
- `.../servlet/JavalinServletRequest.kt:7` - file not found at .../servlet/JavalinServletRequest.kt
- `.../servlet/JavalinServletContext.kt:69` - file not found at .../servlet/JavalinServletContext.kt
- `.../config/JavalinState.kt:83` - file not found at .../config/JavalinState.kt
- `.../router/JavalinDefaultRoutingApi.kt:104` - file not found at .../router/JavalinDefaultRoutingApi.kt
- `.../router/exception/ExceptionMapper.kt:24` - file not found at .../router/exception/ExceptionMapper.kt
- `.../servlet/DefaultTasks.kt:84` - file not found at .../servlet/DefaultTasks.kt
- `.../router/exception/ExceptionMapper.kt:26` - file not found at .../router/exception/ExceptionMapper.kt
- `.../config/RoutesConfig.kt:26` - file not found at .../config/RoutesConfig.kt

### sense/nextjs  - 71/76 grounded

**Hallucinated**
- `app-page.ts:1206` - line 1206 out of range (file only 297 lines) [via packages/next/src/export/routes/app-page.ts]
- `app-page.ts:707` - line 707 out of range (file only 297 lines) [via packages/next/src/export/routes/app-page.ts]

**Unresolved**
- `build/templates/app-page.ts:194` - file not found at build/templates/app-page.ts
- `build/templates/app-page.ts:203` - file not found at build/templates/app-page.ts
- `build/templates/app-page.ts:714` - file not found at build/templates/app-page.ts

### sense/nextjs  - 26/27 grounded

**Unresolved**
- `edge-ssr-app.ts:20` - file not found at edge-ssr-app.ts
