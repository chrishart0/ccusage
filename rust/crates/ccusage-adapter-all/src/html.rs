//! Portable HTML reports containing counters only, with no external assets.
use super::{loader, types::AllRow};
use crate::{
    Result,
    cli::{AgentReportKind, SharedArgs},
};
use serde_json::{Value, json};

pub(super) fn run(kind: AgentReportKind, shared: &SharedArgs) -> Result<()> {
    let rows = loader::load_rows(AgentReportKind::Daily, shared)?.rows;
    let data = report_data(&rows, shared, kind);
    let path = shared.html.as_ref().expect("HTML output path");
    std::fs::write(path, render(&data)?)?;
    eprintln!("HTML report saved to {}", path.display());
    Ok(())
}
fn report_data(rows: &[AllRow], shared: &SharedArgs, kind: AgentReportKind) -> Value {
    let mut records = Vec::new();
    for period in rows {
        for row in period
            .agent_breakdowns
            .as_deref()
            .unwrap_or(std::slice::from_ref(period))
        {
            let models: Vec<_> = row.model_breakdowns.iter().map(|model| {
                let tokens = model.input_tokens.saturating_add(model.output_tokens)
                    .saturating_add(model.cache_creation_tokens).saturating_add(model.cache_read_tokens)
                    .saturating_add(model.extra_total_tokens);
                json!({"name":model.model_name,"tokens":tokens,"cost":if shared.no_cost || row.agent.starts_with("OpenAI:") {None} else {Some(model.cost)}})
            }).collect();
            records.push(
                json!({"date":period.period,"agent":row.agent,"tokens":row.total_tokens,
                "cost":if shared.no_cost {None} else {Some(row.total_cost)},"models":models}),
            );
        }
    }
    json!({"records":records,"costs":!shared.no_cost,"timezone":shared.timezone.as_deref().unwrap_or("local"),
        "group":match kind {AgentReportKind::Monthly=>"month",AgentReportKind::Weekly=>"week",_=>"day"}})
}
fn render(data: &Value) -> Result<String> {
    // A JSON string containing </script> must never close the inert data element.
    let data = serde_json::to_string(data)?
        .replace('&', "\\u0026")
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029");
    Ok(include_str!("usage-report.html").replace("__CCUSAGE_DATA__", &data))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn embedded_labels_cannot_escape_the_json_element() {
        let html =
            render(&json!({"records":[{"agent":"</script><script>alert(1)</script>"}]})).unwrap();
        assert!(!html.contains("<script>alert(1)"));
        let start = html.find("id=\"report-data\">").unwrap() + "id=\"report-data\">".len();
        let end = html[start..].find("</script>").unwrap() + start;
        let parsed: Value = serde_json::from_str(&html[start..end]).unwrap();
        assert_eq!(
            parsed["records"][0]["agent"],
            "</script><script>alert(1)</script>"
        );
    }
    #[test]
    fn export_keeps_source_totals_and_model_tokens_without_invented_costs() {
        let row = AllRow {
            period: "2026-09-12".into(),
            agent: "OpenAI: Example",
            models_used: vec!["gpt".into()],
            input_tokens: 70,
            output_tokens: 20,
            cache_creation_tokens: 0,
            cache_read_tokens: 30,
            total_tokens: 120,
            total_cost: 1.25,
            metadata: None,
            metadata_agents: None,
            agent_breakdowns: None,
            model_breakdowns: vec![crate::ModelBreakdown {
                model_name: "gpt".into(),
                input_tokens: 70,
                output_tokens: 20,
                cache_read_tokens: 30,
                ..Default::default()
            }],
        };
        let data = report_data(
            std::slice::from_ref(&row),
            &SharedArgs::default(),
            AgentReportKind::Daily,
        );
        assert_eq!(data["records"][0]["tokens"], 120);
        assert_eq!(data["records"][0]["models"][0]["tokens"], 120);
        assert!(data["records"][0]["models"][0]["cost"].is_null());
        let hidden = report_data(
            &[row],
            &SharedArgs {
                no_cost: true,
                ..Default::default()
            },
            AgentReportKind::Daily,
        );
        assert!(hidden["records"][0]["cost"].is_null());
    }
}
