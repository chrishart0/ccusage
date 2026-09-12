//! One aggregate covering the requested number of calendar days.
use super::{loader, report, types::AllRow};
use crate::{
    Result,
    cli::{AgentCommandArgs, AgentReportKind},
    print_json_or_jq, wants_json,
};

pub fn run(args: AgentCommandArgs) -> Result<()> {
    if args.shared.html.is_some() {
        return super::html::run(AgentReportKind::Daily, &args.shared);
    }
    let days = args.shared.last.unwrap_or(30);
    let loaded = loader::load_rows(AgentReportKind::Daily, &args.shared)?;
    let rows = summarize(loaded.rows, days);
    if wants_json(&args.shared) {
        let mut output =
            report::report_json_with_agents(&rows, AgentReportKind::Daily, args.by_agent);
        let periods = output
            .as_object_mut()
            .expect("report object")
            .remove("daily")
            .expect("daily rows");
        output["rolling"] = periods;
        output["window"] = serde_json::json!({
            "days": days,
            "since": args.shared.since,
            "until": args.shared.until,
        });
        return print_json_or_jq(output, args.shared.jq.as_deref(), args.shared.no_cost);
    }
    report::print_rolling_table(&rows, &args.shared, &loaded.detected_agents, days)
}

pub(super) fn summarize(rows: Vec<AllRow>, days: u32) -> Vec<AllRow> {
    let mut sources = Vec::new();
    for row in rows {
        sources.extend(match row.agent_breakdowns {
            Some(agents) => agents,
            None => vec![row],
        });
    }
    for source in &mut sources {
        source.period = format!("Last {days} days");
    }
    loader::aggregate_rows(sources, AgentReportKind::Daily)
}
