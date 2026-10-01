## Scenario Evaluation

Results: 1 tools × 6 scenarios

**The headline is `cited_recall`; the blind `llm_quality`/`fairness` composite has been RETIRED** (it weighted 55% on omission-blind prose a frontier baseline aces, 0% on the objective axes Sense wins - it understated Sense ~16×, which silently favored the baseline). Each repo leads with a PRIMARY table of the axes that decompose Sense's value:

- **Cited recall (THE headline)** - share of the gold must-find set pinned to an exact location (`path:line`, `path (line N)`, a `"line": N` field, or an unambiguous basename+line). An agent can jump straight there. This is where Sense's structural advantage concentrates.
- **Mention recall** - share the answer named at all (completeness of the map), location optional.
- **Billed context** - `token_total_billed` with `token_input_uncached` alongside. Lower is better; reported, never traded against recall.

The aggregate adds the **B-score** = `0.55·cited_recall + 0.25·related + 0.20·grounded_precision` - one fair blended number, every term an objective/reference-aware axis Sense wins on merit (no efficiency: it dilutes and is not a correctness axis; efficiency is reported separately, gated at held recall). **Related** = correct-relation rate; **grounded_precision** = anti-fabrication (1 − contradictions/covered).

**Citations** are `file.ext:line`/`file.ext:Symbol` references the assistant printed. The scorer checks each against the repo at `run_meta.repo_commit`; `gold_f1` was dropped (it punished Sense for real beyond-gold finds). The ungrounded-citation list lives in [`citation-hallucinations.md`](citation-hallucinations.md).

### Reading the scores

| Metric | Best | Meaning |
|--------|------|---------|
| cited_recall | Higher | THE HEADLINE: objective cited-recall (location-pinned `path:line`) vs the authored must-find set. Ranks the report. The axis where Sense's structural advantage concentrates (mean margin +0.28 vs baseline). |
| b_score | Higher | Fair blended score = 0.55·cited_recall + 0.25·related + 0.20·grounded_precision. Replaces the retired blind composite. Every term is an objective/reference-aware axis Sense wins on merit; no efficiency (it dilutes and is not a correctness axis). |
| relationship_audit | Higher | Reference-aware audit - fraction of the must-find set the answer COVERED, graded vs the authored relations. Omission-proof. |
| related_recall | Higher | Relation-correctness: covered AND the answer states the CORRECT relation. Grep can name an endpoint; it cannot assert the relation. |
| grounded_precision | Higher | Anti-fabrication (Judging Contract rule 4): of the gold items characterised, the fraction characterised TRUTHFULLY (1 − contradictions/covered). Confident-FALSE relations are penalised here. |
| contradictions | Lower | Raw count of confident-FALSE relation claims on gold items. The fabrication smoking gun. Lower is better. |
| process_efficiency | Lower | Process cost at HELD recall (Judging Contract rule 5): reads / tool-calls / billed tokens, reported as a Sense win ONLY at recall parity or better. Never ranks a cheaper-but-less-complete answer over a complete one. |
| efficiency | Higher | Half token efficiency + half time efficiency, each calibrated per repo |
| tokens | Lower | Billed tokens (uncached) - lower is better (cheaper) |
| wall_time | Lower | Wall-clock time - lower is better, folded into efficiency |
| cost_usd | Lower | API cost in USD - lower is better |
| cites | Higher | Citations grounded against the repo checkout: `grounded/total`. A trailing **!N** marks line numbers beyond EOF - outright fabrication. Reported, not folded into the headline. |

### axum

> Multi-step Axum refactoring: trace Handler trait propagation, understand extractor chaining, add a request ID layer. Tests Rust trait analysis, Tower middleware comprehension, and layered modification.

| Tool | Mention recall | Cited recall (fixed) | Billed ctx | Uncached in | Cached read | Time |
|------|---------------:|---------------------:|-----------:|------------:|------------:|-----:|
| oko-dev | - | - | 17,699 | 13 | 384,171 | 235s |

### discourse

> Multi-step Discourse exploration: trace topic creation flow from controller to persistence, locate specs, understand Guardian authorization. Tests Rails service object tracing and test convention awareness.

| Tool | Mention recall | Cited recall (fixed) | Billed ctx | Uncached in | Cached read | Time |
|------|---------------:|---------------------:|-----------:|------------:|------------:|-----:|
| oko-dev | 100% (24/24) | 88% (21/24) | 33,855 | 12 | 343,327 | 379s |

### flask

> Multi-step Flask refactoring: trace WSGI dispatch, locate tests, add a debug parameter, verify the change. Tests call graph traversal, test-file mapping, and safe code modification awareness.

| Tool | Mention recall | Cited recall (fixed) | Billed ctx | Uncached in | Cached read | Time |
|------|---------------:|---------------------:|-----------:|------------:|------------:|-----:|
| oko-dev | - | - | 6,012 | 8 | 87,132 | 75s |

### gin

> Multi-step Gin exploration: understand middleware chaining, trace HTTP dispatch, find dead code, modify the recovery middleware. Tests data flow tracing, dead code detection, and structural editing awareness.

| Tool | Mention recall | Cited recall (fixed) | Billed ctx | Uncached in | Cached read | Time |
|------|---------------:|---------------------:|-----------:|------------:|------------:|-----:|
| oko-dev | - | - | 7,376 | 9 | 139,665 | 90s |

### javalin

> Multi-step Javalin exploration: understand servlet dispatch, trace routing table construction, add a custom error handler. Tests Java framework comprehension and handler registration patterns.

| Tool | Mention recall | Cited recall (fixed) | Billed ctx | Uncached in | Cached read | Time |
|------|---------------:|---------------------:|-----------:|------------:|------------:|-----:|
| oko-dev | - | - | 12,442 | 14 | 385,198 | 154s |

### nextjs

> Multi-step Next.js exploration: trace SSR render path, understand route matching, thread a request ID. Tests TypeScript monorepo navigation and complex server-side pipeline understanding.

| Tool | Mention recall | Cited recall (fixed) | Billed ctx | Uncached in | Cached read | Time |
|------|---------------:|---------------------:|-----------:|------------:|------------:|-----:|
| oko-dev | - | - | 9,219 | 10 | 207,209 | 124s |

### Aggregate

Ranked by **cited_recall** (the headline). The blind `fairness`/`llm_quality` composite is RETIRED - see the note above. **B-score** = `0.55·cited + 0.25·related + 0.20·grounded_precision`. The `Failures` column shows scenarios the tool could not complete. Costs marked `*` are estimated from partial-transcript token usage.

| Rank | Tool | Scenarios | Failures | **Cited Recall** | **B-score** | Rel Audit (cov) | Related | Grounded Prec. | Contradict. | Avg Efficiency | Avg Tokens | Avg Time | Total Cost | Avg Grounding |
|-----:|------|----------:|--------:|---------------:|-----------:|--------------:|--------:|---------------:|------------:|--------------:|-----------:|--------:|-----------:|--------------:|
| 1 | oko-dev :1st_place_medal: | 30 | 0 | 0.8833 | **0.9215** | 0.9905 | 0.9429 | 1.0000 | 0 | 0.5353 | 12,760 | 160.9s | $25.19 | 99.8% (2047/2052) **!1** |
