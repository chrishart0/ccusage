# HTML reports

Save a compact, interactive report that opens directly in a browser:

```sh
ccusage monthly --html usage.html
ccusage rolling 30 --html last-30-days.html
ccusage daily --since 2026-01-01 --html this-year.html
```

In this fork's local installation, use `ccusage-fork` in place of `ccusage`.
Open the generated file or share it as an attachment. It includes its data,
styles, and chart code; no server, internet connection, or external chart library
is needed. Running the same command again replaces the output file.

The report includes total tokens, USD cost, active days, and source/model counts.
Switch the chart between total usage, agents, and models; choose tokens or cost;
and group by day, week, or month. Weeks begin Monday. The source selector filters
both charts and totals. Agent and model tables show every source; charts group
smaller series into “Other” to stay compact. Hover or focus a bar for exact values.
Use **Print / save PDF** for a static copy.

HTML export keeps daily data even when generated from a monthly or weekly
command, so browser grouping works without collecting again. Long daily reports
initially use a monthly chart. Rolling exports retain only the requested window.
It works with unified daily, weekly, monthly, and rolling reports, including
[SSH sources](./config-files.md) and [OpenAI accounts](./openai-platform.md).
It cannot be combined with session reports, `--sections`, `--json`, or `--jq`.
Use `--no-cost` to omit costs from the embedded data and display.

Token totals include cached input. Agent costs retain their existing reported or
estimated meaning. OpenAI Platform costs come from billing; model token counts
come from usage, but billing costs cannot reliably be split by model. Those
charges appear under **Unallocated**, as do counters without model detail.
Platform days use UTC; agent logs use the configured timezone. Combined totals
are additive and can include overlapping usage from agent logs and Platform
billing.

The file contains usage counters and source/model names, but no admin keys or
conversation content. For machine-readable exports, see [JSON output](./json-output.md).
