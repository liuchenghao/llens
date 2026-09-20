use std::path::PathBuf;
use chrono::{Local, NaiveDate};
use super::store::list_t10_range;

/// Day + its 10-min slice count (for calendar badge / auto-catchup).
#[derive(Debug, Clone, serde::Serialize)]
pub struct DaySliceCount {
    pub date: String,
    pub slices: usize,
    /// True if a diary already exists on disk (any status).
    pub has_diary: bool,
}
use serde::{Deserialize, Serialize};

/// Daily diary: second-order summary built from that day's 10-min slices.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Diary {
    pub date: String,
    pub brief: String,
    pub top3: Vec<Task>,
    pub highlights: Vec<String>,
    pub todos: Vec<Todo>,
    pub advice: Vec<String>,
    pub tip: String,
    pub status: String, // "done" | "pending"
    pub source: Vec<String>, // which 10-min files fed this
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Todo {
    pub text: String,
    #[serde(default)]
    pub done: bool,
}

/// 日报条目：工作任务描述 + 累计时长（分钟）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Task {
    pub text: String,
    /// 累计时长（分钟），0 表示未统计
    #[serde(default)]
    pub minutes: u32,
}

impl Diary {
    pub fn new_pending(date: NaiveDate) -> Self {
        Self {
            date: date.to_string(),
            brief: String::new(),
            top3: vec![],
            highlights: vec![],
            todos: vec![],
            advice: vec![],
            tip: String::new(),
            status: "pending".into(),
            source: vec![],
        }
    }
}

fn path(root: &PathBuf, d: NaiveDate) -> PathBuf {
    root.join("diary").join(d.format("%Y-%m").to_string())
        .join(format!("{d}.json"))
}

pub fn load(root: &PathBuf, d: NaiveDate) -> Option<Diary> {
    let p = path(root, d);
    std::fs::read_to_string(p)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
}

pub fn save(root: &PathBuf, diary: &Diary) -> Result<(), String> {
    let d = NaiveDate::parse_from_str(&diary.date, "%Y-%m-%d")
        .unwrap_or_else(|_| Local::now().date_naive());
    let p = path(root, d);
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let s = serde_json::to_string_pretty(diary).map_err(|e| e.to_string())?;
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, s).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &p).map_err(|e| e.to_string())?;
    Ok(())
}

/// Days (within lookback window) that have 10-min data. Each item carries the
/// slice count and whether a diary file already exists. Oldest-first so the UI
/// can show "待生成" markers on the calendar and trigger auto-catchup in order.
pub fn days_with_slices(
    root: &PathBuf,
    up_to: NaiveDate,
    lookback_days: u32,
) -> Vec<DaySliceCount> {
    let mut out: Vec<DaySliceCount> = Vec::new();
    let mut d = up_to;
    for i in 0..=lookback_days as i64 {
        let slices = list_t10_range(root, d, d + chrono::Duration::days(1));
        if !slices.is_empty() {
            out.push(DaySliceCount {
                date: d.to_string(),
                slices: slices.len(),
                has_diary: load(root, d).is_some(),
            });
        }
        if i == lookback_days as i64 {
            break;
        }
        d = d - chrono::Duration::days(1);
        if out.len() > 60 {
            break;
        }
    }
    out
}

/// A day is "pending" if it has 10-min data and either has no diary, or the
/// diary is stale (fewer source slices than currently available — the day's
/// data has grown since the diary was generated).
pub fn pending_days(root: &PathBuf, up_to: NaiveDate, lookback_days: u32) -> Vec<NaiveDate> {
    let mut out = Vec::new();
    let mut d = up_to;
    for i in 0..=lookback_days as i64 {
        let slices = super::store::list_t10_range(root, d, d + chrono::Duration::days(1));
        let has_data = !slices.is_empty();
        let stale = match load(root, d) {
            None => true, // no diary yet
            Some(diary) => {
                if diary.status != "done" {
                    true
                } else {
                    // done diary: stale if fewer source slices than current data
                    diary.source.len() < slices.len()
                }
            }
        };
        if has_data && stale {
            out.push(d);
        }
        if i == lookback_days as i64 { break; }
        d = d - chrono::Duration::days(1);
        if out.len() > 14 { break; }
    }
    out
}

/// Generate (or regenerate) the diary for a single day.
/// `min_task_minutes`: 主要事项统计阈值（分钟），0 表示不限制；
/// 只把累计时长 ≥ 该值的任务计入 top3，默认 30。
pub async fn generate_day(
    root: &PathBuf,
    cfg: &super::store::Config,
    day: NaiveDate,
    min_task_minutes: u32,
) -> Result<Diary, String> {
    regenerate_day(root, cfg, day, false, min_task_minutes).await
}

/// Regenerate the diary for a single day. If `force` is true, ignore
/// "already done" status and always re-run the LLM.
pub async fn regenerate_day(
    root: &PathBuf,
    cfg: &super::store::Config,
    day: NaiveDate,
    force: bool,
    min_task_minutes: u32,
) -> Result<Diary, String> {
    let slices =
        super::store::list_t10_range(root, day, day + chrono::Duration::days(1));
    if slices.is_empty() {
        if !force {
            let p = Diary::new_pending(day);
            save(root, &p)?;
            return Ok(p);
        }
        return Err(format!("no 10-min data for {day}"));
    }
    // Non-force fast path: if diary already exists and is fresh, skip.
    if !force {
        if let Some(existing) = load(root, day) {
            if existing.status == "done" && existing.source.len() >= slices.len() {
                return Ok(existing);
            }
        }
    }
    let mut corpus = Vec::new();
    for s in &slices {
        corpus.push(format!(
            "【{} - {}】叙述:{} 活动:{} 软件:{}",
            s.from,
            s.to,
            s.narrative,
            s.activities.join("、"),
            s.top_apps.join("、")
        ));
        let _ = s;
    }
    let corpus = corpus.join("\n");
    // 任务时长阈值：0 表示不限制（全部计入），否则只统计 ≥ 该分钟数的工作任务
    let threshold = if min_task_minutes > 0 { min_task_minutes } else { 0 };
    let threshold_desc = if threshold > 0 {
        format!("只统计累计时长 ≥ {threshold} 分钟的工作任务，不足 {threshold} 分钟的零散操作忽略不写")
    } else {
        "不按时长过滤，所有工作任务均计入".to_string()
    };

    let prompt = format!(
        r#"你是用户的屏幕活动记录分析助手。下面是 {day} 一天内按 10 分钟切片的汇总记录：
{corpus}

请基于这些记录生成当天的日记，严格按如下 JSON 输出（不要输出 JSON 以外的任何文字）：
{{
  "brief": "简短总结（不超过 80 字）",
  "top3": [
    {{"text": "日报条目一描述（约 50 字）", "minutes": 45}},
    {{"text": "日报条目二描述（约 50 字）", "minutes": 30}},
    "…按实际符合条件的工作任务数量，每条含 text 和 minutes 字段"
  ],
  "highlights": ["高光时刻一","高光时刻二"],
  "todos": ["待办一","待办二"],
  "advice": ["优化建议一","优化建议二"],
  "tip": "一句温馨提示"
}}

【top3 日报条目规则】
top3 字段为对象数组，每项格式为 {{"text": "描述", "minutes": 整数分钟数}}，要求：
- **{threshold_desc}**
- **每条标注累计时长**（minutes 字段，单位：分钟），根据切片记录估算该任务的实际持续时间
- 条数：按实际统计出的任务数量写（不要凑数）
- 每条 text 约 50 个中文字符，纯文本业务语言描述（不用变量名、文件路径、类名、方法名）
- 聚焦：做了什么、为什么、带来什么影响
- 语气：简洁、专业、工作总结口吻
- 归纳合并：将相邻切片中相同或高度相似的条目合并为一条，避免重复
- 示例（好）：{{"text":"修复未登录状态下短视频页打开报异常问题","minutes":50}} {{"text":"调整用户收藏接口，请求失败时返回空列表不再抛出错误","minutes":35}}
- 示例（坏，禁止）：{{"text":"将 Scroll 组件替换为 List，添加 SmartRefreshController","minutes":30}}（包含代码标识符）
- **休息时段不计入 top3**：标记为「休息」（熄屏/黑屏）的切片表示用户离开了电脑，这些时段**不得**作为工作任务计入 top3，也不要为它们估算 minutes；可在 brief 或 advice 中简短提及当天休息/离开的时间分布，但 top3 只写实际工作

其他字段：highlights/todos/advice 各 1-3 条；如果当天数据很少，也要尽量填写。所有字段都用中文。"#,
        day = day,
        corpus = corpus,
        threshold_desc = threshold_desc
    );

    let client = reqwest::Client::new();
    let text = super::llm::chat(
        &client,
        cfg,
        &[super::llm::text_msg(&prompt)],
    )
    .await
    .unwrap_or_default();

    let mut parsed = serde_json::Value::Null;
    if let Some(start) = text.find('{') {
        if let Some(end) = text.rfind('}') {
            if end > start {
                parsed =
                    serde_json::from_str(&text[start..=end]).unwrap_or(serde_json::Value::Null);
            }
        }
    }

    let str_arr = |k: &str| {
        parsed[k]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str())
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };

    let mut diary = Diary::new_pending(day);
    diary.brief = parsed["brief"].as_str().unwrap_or_default().trim().to_string();
    // 解析 top3：兼容对象数组 [{text,minutes}] 和旧版字符串数组 ["..."]
    diary.top3 = parsed["top3"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| {
                    if let Some(obj) = x.as_object() {
                        let text = obj["text"].as_str()?.trim().to_string();
                        let minutes = obj["minutes"].as_u64().unwrap_or(0) as u32;
                        if text.is_empty() { None } else { Some(Task { text, minutes }) }
                    } else {
                        x.as_str().map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty())
                            .map(|text| Task { text, minutes: 0 })
                    }
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    diary.highlights = str_arr("highlights");
    diary.todos = str_arr("todos")
        .into_iter()
        .map(|text| Todo { text, done: false })
        .collect();
    diary.advice = str_arr("advice");
    diary.tip = parsed["tip"].as_str().unwrap_or_default().trim().to_string();
    diary.source = slices.iter().map(|s| s.from.clone()).collect();
    diary.status = "done".into();
    save(root, &diary)?;
    Ok(diary)
}

/// Re-run all pending days within the lookback window (oldest first), up to `limit`.
pub async fn regenerate_pending(
    root: &PathBuf,
    cfg: &super::store::Config,
    limit: usize,
    min_task_minutes: u32,
) -> Vec<Diary> {
    let days = pending_days(root, Local::now().date_naive(), cfg.diary_lookback_days);
    let mut out = Vec::new();
    for d in days.into_iter().rev().take(limit) {
        match generate_day(root, cfg, d, min_task_minutes).await {
            Ok(d) => out.push(d),
            Err(e) => eprintln!("diary {d} failed: {e}"),
        }
    }
    out
}
