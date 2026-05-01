# Graph Report - desktop  (2026-05-01)

## Corpus Check
- 44 files · ~82,967 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 311 nodes · 292 edges · 3 communities detected
- Extraction: 99% EXTRACTED · 1% INFERRED · 0% AMBIGUOUS · INFERRED: 3 edges (avg confidence: 0.8)
- Token cost: 0 input · 0 output

## Community Hubs (Navigation)
- [[_COMMUNITY_Community 3|Community 3]]
- [[_COMMUNITY_Community 12|Community 12]]
- [[_COMMUNITY_Community 29|Community 29]]

## God Nodes (most connected - your core abstractions)
1. `ApiClient` - 8 edges
2. `main()` - 6 edges
3. `parse_response()` - 5 edges
4. `load_config()` - 4 edges
5. `load_master_key()` - 3 edges
6. `upload_file()` - 3 edges
7. `sync_batch()` - 3 edges
8. `futures_task_noop_waker()` - 3 edges
9. `config_path()` - 3 edges
10. `b64()` - 2 edges

## Surprising Connections (you probably didn't know these)
- `main()` --calls--> `load_config()`  [INFERRED]
  windows/src/main.rs → windows/src/config.rs

## Communities

### Community 3 - "Community 3"
Cohesion: 0.27
Nodes (8): Args, b64(), futures_task_noop_waker(), is_ignored_file(), load_master_key(), main(), sync_batch(), upload_file()

### Community 12 - "Community 12"
Cohesion: 0.56
Nodes (2): ApiClient, parse_response()

### Community 29 - "Community 29"
Cohesion: 0.53
Nodes (4): Config, config_path(), load_config(), save_config()

## Knowledge Gaps
- **1 isolated node(s):** `Args`
  These have ≤1 connection - possible missing edges or undocumented components.
- **Thin community `Community 12`** (9 nodes): `ApiClient`, `.create_folder()`, `.list_files()`, `.require_auth()`, `.upload_encrypted()`, `.url()`, `.verify_session()`, `parse_response()`, `api.rs`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `main()` connect `Community 3` to `Community 29`?**
  _High betweenness centrality (0.004) - this node is a cross-community bridge._
- **Why does `load_config()` connect `Community 29` to `Community 3`?**
  _High betweenness centrality (0.002) - this node is a cross-community bridge._
- **Why does `ApiClient` connect `Community 12` to `Community 3`?**
  _High betweenness centrality (0.002) - this node is a cross-community bridge._
- **Are the 2 inferred relationships involving `main()` (e.g. with `.new()` and `load_config()`) actually correct?**
  _`main()` has 2 INFERRED edges - model-reasoned connections that need verification._
- **What connects `Args` to the rest of the system?**
  _1 weakly-connected nodes found - possible documentation gaps or missing edges._