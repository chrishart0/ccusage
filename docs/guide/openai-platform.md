# OpenAI Platform accounts

This fork adds named OpenAI Platform accounts to unified daily, weekly, monthly,
and rolling reports. It reads the organization Usage and Costs APIs using admin
keys. Platform billing has no local session IDs, so it is excluded from session
reports and focused agent reports.

Save credentials in `~/.config/ccusage/openai.env` (or
`$XDG_CONFIG_HOME/ccusage/openai.env`) and restrict it with `chmod 600`:

```dotenv
OPENAI_ADMIN_KEY_1=your-first-admin-key
OPENAI_ADMIN_KEY_2=your-second-admin-key
```

Add this top-level section to your [configuration file](./config-files.md):

```json
{
	"openai": {
		"accounts": [
			{ "name": "Prouct 1", "keyEnv": "OPENAI_ADMIN_KEY_1" },
			{ "name": "product_2", "keyEnv": "OPENAI_ADMIN_KEY_2" }
		]
	}
}
```

The names appear as `OpenAI: Prouct 1` and `OpenAI: product_2`. `keyEnv` references
an environment variable; never put the key itself in JSON. Exported environment
variables take precedence over the dotenv file. Set `openai.envFile` to an
absolute path (or `~/...`) to use another dotenv file. Relative paths resolve
from the current working directory. A project's `.env` is not loaded implicitly.

```sh
ccusage monthly                  # All available history, grouped by month
ccusage rolling                  # Last 30 days, or configured rolling default
ccusage rolling 7                # Last seven days
ccusage daily --no-openai        # Skip Platform accounts
ccusage monthly --refresh-openai # Fetch fresh counters
ccusage monthly --by-agent --json
```

Normal period reports paginate from January 1, 2020 through now, covering the
Platform's history available through these APIs. Rolling reports and explicit
`--since`/`--until` bounds request only their window. Optional `openai.since` or
per-account `since` can restrict collection, using `YYYY-MM-DD` or `YYYYMMDD`.
There is no default last-30-days filter on daily or monthly reports.

Counters are cached for five minutes under `$XDG_CACHE_HOME/ccusage/openai`
(default `~/.cache/ccusage/openai`). The cache contains normalized counters and
costs, with owner-only directory/file permissions on Unix. Credentials are never
stored in the cache. `--refresh-openai` bypasses it. The first full-history report
can take longer because it follows every page for each account.

Tokens include completions and embeddings. Cached input is separated from
uncached input without double counting. Costs come from the organization Costs
API, including charges for products that do not report token counts, so a day
can have costs and zero tokens. Model cost breakdowns are not estimated from
these billing totals. All Platform days use UTC buckets, even when local agent
reports use another timezone. API reporting delays can affect recent days.
See OpenAI's [Usage API documentation](https://platform.openai.com/docs/api-reference/usage).

Optional per-account `projectIds` restricts both usage and costs to selected
projects. Configure each organization once, or use disjoint project lists when
splitting one organization into products. Distinct keys can belong to the same
organization; the collector cannot infer your intended allocation. Local and
SSH agent logs can describe the same requests billed by Platform accounts.
Combined totals are additive and do not deduplicate across those sources; use
`--by-agent` or [JSON output](./json-output.md) to inspect each source.

Authentication, pagination, and API failures fail the report instead of silently
omitting an account. `--offline` only controls model-pricing downloads; use
`--no-openai --no-ssh` when you want to avoid those network collectors too.
