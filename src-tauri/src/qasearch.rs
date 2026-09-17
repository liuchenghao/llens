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
    /// 单轮命中上限（可调）。默认 20；0 表示用默认 20。
    pub limit: i64,
}

impl Default for Plan {
    fn default() -> Self {
        Self {
            keywords: vec![],
            layer: "t10".into(),
            days: 7,
            need_more: false,
            limit: 20,
        }
    }
}

/// 多轮问答历史：上一轮的问题 + 回答，供追问时提供上下文。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ConvTurn {
    pub question: String,
    pub answer: String,
}

/// QA 最终回答：纯文本回答 + 可点来源引用（hit 下标）。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct QaAnswer {
    pub text: String,
    pub citations: Vec<usize>,
}

/// Run one QA round: plan -> local search -> hits.
/// `limit_override` 为 0 时用 plan.limit，否则用该值（前端可动态调整）。
pub fn search_round(root: &PathBuf, plan: &Plan, limit_override: i64) -> Vec<Hit> {
    let limit = {
        let v = if limit_override > 0 { limit_override } else { plan.limit };
        v.clamp(1, 100) // 至少 1，封顶 100，防超大上下文
    };
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
        let hits = super::search::search(root, layer, start, end, kw, limit as u32);
        for h in hits {
            if !out.iter().any(|x| x.time == h.time && x.layer == h.layer) {
                out.push(h);
            }
            if out.len() >= limit as usize {
                break;
            }
        }
        if out.len() >= limit as usize {
            break;
        }
    }
    out
}

/// 把多轮历史压缩成一段 prompt 上下文（最多取最近 3 轮，控制 token）。
fn history_block(history: Option<&[ConvTurn]>) -> String {
    let turns = history
        .unwrap_or(&[])
        .iter()
        .rev()
        .take(3)
        .map(|t| format!("Q: {}\nA: {}", t.question, t.answer));
    turns.collect::<Vec<_>>().join("\n---\n")
}

/// Ask the LLM for the first plan (with optional conversation history).
pub async fn plan(
    cfg: &Config,
    question: &str,
    history: Option<&[ConvTurn]>,
) -> Result<Plan, String> {
    let h = history_block(history);
    let prompt = format!(
        r#"你是屏幕活动记录检索器。用户问题：{question}
{hist_block}请输出检索计划 JSON（只输出 JSON）：
{{"keywords":["关键词1","关键词2"],"layer":"frame|t10|diary","days":7,"need_more":false}}
layer 含义：frame=20秒帧级总结；t10=10分钟汇总；diary=日级日记。
days 为检索的时间窗（天，最大 31）。keywords 2-4 个中文关键词。若当前问题是追问且上文已检索过相关内容，可沿用相似关键词或扩大时间窗。"#,
        question = question,
        hist_block = if h.is_empty() {
            String::new()
        } else {
            format!("以下是之前的问答历史（供参考，避免重复检索）：\n{h}\n")
        }
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
                    p.limit = v["limit"].as_i64().unwrap_or(20);
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
    history: Option<&[ConvTurn]>,
) -> Result<Plan, String> {
    let hit_desc: String = previous_hits
        .iter()
        .take(10)
        .map(|h| format!("[{}] {}", h.time, h.text))
        .collect::<Vec<_>>()
        .join("\n");
    let h = history_block(history);
    let prompt = format!(
        r#"用户问题：{question}
{hist_block}第一轮检索结果：
{hit_desc}
如果上下文足够回答，输出 {{"keywords":[],"layer":"t10","days":0,"need_more":false}}；否则输出新的检索计划 JSON。"#,
        question = question,
        hist_block = if h.is_empty() {
            String::new()
        } else {
            format!("之前的问答历史：\n{h}\n")
        }
    );
    let text = super::llm::chat(
        &reqwest::Client::new(),
        cfg,
        &[super::llm::text_msg(&prompt)],
    )
    .await?;
    Ok(parse_plan(&text))
}

/// Final answer from accumulated context. 在回答中用 [n] 标注引用了第 n 条片段。
pub async fn answer(
    cfg: &Config,
    question: &str,
    context: &[Hit],
    history: Option<&[ConvTurn]>,
) -> Result<String, String> {
    let ctx: String = context
        .iter()
        .enumerate()
        .map(|(i, h)| format!("[{}][{}][{}] {}", i + 1, h.time, h.layer, h.text))
        .collect::<Vec<_>>()
        .join("\n");
    let h = history_block(history);
    let prompt = format!(
        r#"基于以下检索到的屏幕活动记录片段回答用户问题。回答用中文，简洁（不超过 180 字）。
{hist_block}【引用规则】在回答正文中，凡引用了某条片段的信息，请在对应句子末尾用 [n] 标注（n 为片段编号，从 1 开始）；不要编造不存在的编号，最多标注 5 处。如果片段不足以回答，说明缺少什么并建议可用的时间范围。
片段：
{ctx}

用户问题：{question}"#,
        question = question,
        ctx = ctx,
        hist_block = if h.is_empty() {
            String::new()
        } else {
            format!("之前的问答历史（追问上下文）：\n{h}\n")
        }
    );
    let text = super::llm::chat(
        &reqwest::Client::new(),
        cfg,
        &[super::llm::text_msg(&prompt)],
    )
    .await?;
    Ok(text)
}

/// 从回答文本中提取 [n] 形式的引用，返回对应的 hit 下标（0-based）。
pub fn citations(text: &str, hits: &[Hit]) -> Vec<usize> {
    let mut seen: std::collections::HashSet<usize> = std::collections::HashSet::new();
    let mut out: Vec<usize> = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'[' {
            if let Some(close) = bytes[i..].iter().position(|&b| b == b']') {
                let inner = &bytes[i + 1..i + close];
                if let Ok(n_str) = std::str::from_utf8(inner) {
                    if let Ok(num) = n_str.parse::<usize>() {
                        let idx = num - 1;
                        if idx < hits.len() && seen.insert(idx) {
                            out.push(idx);
                        }
                    }
                }
                i += close + 1;
            } else {
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    out
}
