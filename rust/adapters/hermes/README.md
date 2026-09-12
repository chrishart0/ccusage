# ccusage-adapter-hermes

The Hermes adapter: it turns the Hermes SQLite state database
into the usage entries the reports render.

## Owns

- `loader.rs` — reading the source, dedupe, and date filtering.
- `parser.rs` — raw record parsing, token mapping, and model naming.
- `remote.rs` — read-only usage collection over SSH using remote Python 3.
- `paths.rs` — environment variables, default directories, and file discovery.
- `report.rs` — the JSON and table shapes where they differ from the shared ones.

Anything that is not specific to this source belongs in `ccusage-core` or
`ccusage-adapter-common` instead.

## Data source

- `${HERMES_HOME:-~/.hermes}/state.db`

Reads SQLite with the bundled `sqlite` crate, which is why this crate declares it and
most adapters do not.

## Public surface

- `loader::load_entries`
- `report::report_from_rows`
- `report::summarize_entries`
- `run`

## Depends on

- `ccusage-adapter-common`
- `ccusage-core`
- `jiff`
- `serde_json`
- `sqlite`

## Build layer

Built in the `adapters` Crane artifact layer; the layer compiles all adapters in one Cargo invocation, so they build concurrently.

SSH destinations come from `SharedArgs.ssh` (CLI `--ssh` or config `ssh`). Remote
rows use the same SQLite token mapping as local rows, with host-prefixed session
IDs. No remote logs are persisted locally. Collection errors fail the report.

Remote discovery includes `state.db` and `profiles/*/state.db` under each Hermes
home, never `state-snapshots` backups. A host with no databases fails collection.
