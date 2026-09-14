use std::path::PathBuf;
use serde::{Deserialize, Serialize};
use super::search::Hit;
use super::store::Config;

/// QA plan: what the model wants to search for.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Plan {
    pub keywords: Vec<String>,
    pub layer: String, // "frame" | "t10" | "diary"
    pub days: i64, // time window in days (<=31)
    pub need_more: bool,
}

impl Default for Plan {
    fn default() -> Self {
        Self {
            keywords: vec![],
            layer: "t10".into(),
            days: 7,
            need_more: false,
        }
    }
}

/// Run one QA round: plan -> local search (<=3 total, enforced by caller) -> hits.
pub fn search_round(
    root: &PathBuf,
    plan: &Plan,
) -> Vec<Hit> {
    let now = chrono::Local::now();
    let end = now.date_naive();
    let days = plan.days.clamp(1, 31);
    let start = end - chrono::Duration::days(days);
    let layer = match plan.layer.as_str() {
        "frame" => "frame",
        "diary" => "diary",
        _ => "t10",
    };
    let mut out: Vec<Hit> = Vec::new();
    for kw in plan.keywords.iter() {
        let hits = super::search::search(root, layer, start, end, kw, 20);
        for h in hits {
            if !out.iter().any(|x| x.time == h.time && x.layer == h.layer) {
                out.push(h);
            }
            if out.len() >= 20 {
                break;
            }
        }
        if out.len() >= 20 {
            break;
        }
    }
    out
}

/// Ask the LLM for the first plan.
pub async fn plan(
    cfg: &Config,
    question: &str,
) -> Result<Plan, String> {
    let prompt = format!(
        r#"你是屏幕活动记录检索器。用户问题：{question}
请输出检索计划 JSON（只输出 JSON）：
{{"keywords":["关键词1","关键词2"],"layer":"frame|t10|diary","days":7,"need_more":false}}
layer 含义：frame=20秒帧级总结；t10=10分钟汇总；diary=日级日记。
days 为检索的时间窗（天，最大 31）。keywords 2-4 个中文关键词。"#
    );
    let text = super::llm::chat(
        &reqwest::Client::new(),
        cfg,
        &[super::llm::text_msg(&prompt)],
    )
    .await?;
    Ok(parse_plan(&text))
}

pub fn parse_plan(text: &str) -> Plan {
    let mut p = Plan::default();
    if let Some(start) = text.find('{') {
        if let Some(end) = text.rfind('}') {
            if end > start {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text[start..=end]) {
                    p.keywords = v["keywords"]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .filter_map(|x| x.as_str())
                                .map(|s| s.to_string())
                                .collect()
                        })
                        .unwrap_or_default();
                    p.layer = v["layer"].as_str().unwrap_or("t10").to_string();
                    p.days = v["days"].as_i64().unwrap_or(7);
                    p.need_more = v["need_more"].as_bool().unwrap_or(false);
                }
            }
        }
    }
    p
}

/// Ask the LLM for a follow-up plan (after seeing first-round hits).
pub async fn refine(
    cfg: &Config,
    question: &str,
    previous_hits: &[Hit],
) -> Result<Plan, String> {
    let hit_desc: String = previous_hits
        .iter()
        .take(10)
        .map(|h| format!("[{}] {}", h.time, h.text))
        .collect::<Vec<_>>()
        .join("\n");
    let prompt = format!(
        r#"用户问题：{question}
第一轮检索结果：
{hit_desc}
如果上下文足够回答，输出 {{"keywords":[],"layer":"t10","days":0,"need_more":false}}；否则输出新的检索计划 JSON。"#
    );
    let text = super::llm::chat(
        &reqwest::Client::new(),
        cfg,
        &[super::llm::text_msg(&prompt)],
    )
    .await?;
    Ok(parse_plan(&text))
}

/// Final answer from accumulated context.
pub async fn answer(
    cfg: &Config,
    question: &str,
    context: &[Hit],
) -> Result<String, String> {
    let ctx: String = context
        .iter()
        .map(|h| format!("[{}][{}] {}", h.time, h.layer, h.text))
        .collect::<Vec<_>>()
        .join("\n");
    let prompt = format!(
        r#"基于以下检索到的屏幕活动记录片段回答用户问题。回答用中文，简洁（不超过 150 字），并在结尾用「来源:」列出引用片层的数量。如果片段不足以回答，说明缺少什么。
片段：
{ctx}

用户问题：{question}"#
    );
    let text = super::llm::chat(
        &reqwest::Client::new(),
        cfg,
        &[super::llm::text_msg(&prompt)],
    )
    .await?;
    Ok(text)
}
