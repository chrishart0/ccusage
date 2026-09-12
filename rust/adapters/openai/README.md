# OpenAI Platform adapter

Configured organization accounts participate in unified period and rolling reports:

```sh
ccusage monthly --by-agent
ccusage rolling 30 --json
```

See [account setup and data semantics](../../../docs/guide/openai-platform.md).
`credentials.rs` reads referenced environment variables or a private dotenv file;
`loader.rs` handles bounded, authenticated pagination against the fixed official API;
`parser.rs` merges UTC token and cost buckets; `cache.rs` stores normalized results
for five minutes. Completions and embeddings supply tokens, while Costs supplies
actual USD charges. Cached input is a subset of input, and session reports omit
organization billing. No request payloads or raw credentials enter report JSON.
