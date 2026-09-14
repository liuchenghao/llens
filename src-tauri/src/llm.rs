use serde::{Deserialize, Serialize};
use super::store::Config;

/// One chat message (OpenAI-compatible).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmMsg {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<LlmContent>,
    pub images: Vec<String>, // base64 data URLs or file paths handled by caller
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum LlmContent {
    Text(String),
    Multi(Vec<LlmPart>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmPart {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_url: Option<LlmImageUrl>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmImageUrl {
    pub url: String,
}

pub fn text_msg(content: &str) -> LlmMsg {
    LlmMsg {
        role: "user".into(),
        content: Some(LlmContent::Text(content.into())),
        images: vec![],
    }
}

#[allow(dead_code)]
pub fn system_msg(content: &str) -> LlmMsg {
    LlmMsg {
        role: "system".into(),
        content: Some(LlmContent::Text(content.into())),
        images: vec![],
    }
}

pub fn image_msg(base64_data_url: &str, prompt: &str) -> LlmMsg {
    LlmMsg {
        role: "user".into(),
        content: Some(LlmContent::Multi(vec![
            LlmPart {
                kind: "text".into(),
                text: Some(prompt.into()),
                image_url: None,
            },
            LlmPart {
                kind: "image_url".into(),
                text: None,
                image_url: Some(LlmImageUrl {
                    url: base64_data_url.into(),
                }),
            },
        ])),
        images: vec![],
    }
}

pub async fn chat(
    client: &reqwest::Client,
    cfg: &Config,
    messages: &[LlmMsg],
) -> Result<String, String> {
    let url = format!("{}/chat/completions", cfg.base_url.trim_end_matches('/'));
    let body = serde_json::json!({
        "model": cfg.model,
        "messages": messages,
        "temperature": 0.3,
    });
    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", cfg.api_key))
        .header("Content-Type", "application/json")
        .timeout(std::time::Duration::from_secs(60))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("request failed: {e}"))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let t = resp.text().await.unwrap_or_default();
        return Err(format!("llm http {status}: {t}"));
    }
    let v: serde_json::Value = resp.json().await.map_err(|e| format!("bad json: {e}"))?;
    Ok(v["choices"][0]["message"]["content"]
        .as_str()
        .or_else(|| v["choices"][0]["message"]["text"].as_str())
        .unwrap_or_default()
        .to_string())
}
