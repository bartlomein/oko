# Rank documents and records

[← Back to Oko](../README.md)

## Rank documents or database results

Provide a JSON array of items:

```json
[{"id":"ticket-42","text":"Charged twice for the same order.","source":"tickets/42"}]
```

```sh
oko rank --input examples/items.json "Who needs a refund?" --json
```

Files are limited to 1 MiB and 30 items. IDs must be unique, nonempty strings of
at most 200 UTF-16 code units (the existing JavaScript contract); text must be
nonempty. `source` is optional. Unknown fields are discarded.

Results preserve IDs and sources, and return up to five items with relevance
scores above `0.5`. Each candidate gets an independent yes/no relevance judgment
using TypeSafe's Noul primitive; all judgments share one request. Several items
can score highly at once, and the scores do not need to sum to one. The `0.5`
cutoff is an initial decision threshold, not a calibrated accuracy guarantee.
These scores are not comparable with older versions' Choice scores.

`omittedCount` counts items dropped by the 32,000-byte request budget, not items
outside the top five or below the relevance cutoff. Supply candidates in your
existing search order. Independent questions share that budget with the item
text, so large inputs may lose more trailing items than before. Normal code
search fits its previews to the available space. `--no-jev` preserves the input
order with zero scores; it is an input-order baseline, not keyword search.
Oko does not fetch URLs or connect to databases.

See [independent relevance validation](../benchmarks/independent-relevance.md) for
the before/after comparison, request-budget tradeoffs, and remaining limitations.

Any programming language can invoke the CLI and consume its JSON output.
Rust applications can use the library directly:

```rust,no_run
use oko::{rank_items, RankItem, RankOptions};

fn main() -> anyhow::Result<()> {
    let items = vec![RankItem {
        id: "ticket-42".into(),
        text: "Charged twice for the same order.".into(),
        source: Some("tickets/42".into()),
    }];
    let result = rank_items("Who needs a refund?", &items, &RankOptions {
        api_key: std::env::var("TYPESAFE_API_KEY").ok(),
        ..Default::default()
    })?;
    println!("{:?}", result.results);
    Ok(())
}
```

The library is synchronous and returns `Result`; async callers should run it on
a blocking worker. It does not load `.env` or the OS credential store itself; pass `api_key` explicitly. Use `no_jev: true` to preserve
input order without a request. `parse_items` validates unknown JSON first.
