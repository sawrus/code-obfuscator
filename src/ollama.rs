use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{AppError, AppResult};

#[derive(Debug, Clone)]
pub struct OllamaConfig {
    pub url: String,
    pub model: String,
    pub top_n: usize,
}

#[derive(Serialize)]
struct GenerateReq<'a> {
    model: &'a str,
    prompt: String,
    stream: bool,
}

#[derive(Deserialize)]
struct GenerateResp {
    response: String,
}

pub fn suggest_mapping(
    cfg: &OllamaConfig,
    terms: &[String],
) -> AppResult<BTreeMap<String, String>> {
    if terms.is_empty() {
        return Ok(BTreeMap::new());
    }
    let prompt = build_prompt(terms, cfg.top_n);
    let body = request(cfg, prompt)?;
    parse_body(&body)
}

fn build_prompt(terms: &[String], top_n: usize) -> String {
    let joined = terms
        .iter()
        .take(top_n)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    format!("Return only JSON object old->new random words for: {joined}")
}

fn request(cfg: &OllamaConfig, prompt: String) -> AppResult<String> {
    let req = GenerateReq {
        model: &cfg.model,
        prompt,
        stream: false,
    };
    let url = format!("{}/api/generate", cfg.url.trim_end_matches('/'));
    let resp: GenerateResp = ureq::post(&url)
        .send_json(req)
        .map_err(http_err)?
        .into_json()
        .map_err(http_err)?;
    Ok(resp.response)
}

fn http_err<E: std::fmt::Display>(err: E) -> AppError {
    AppError::Http(err.to_string())
}

fn parse_body(body: &str) -> AppResult<BTreeMap<String, String>> {
    let val: Value = serde_json::from_str(body)?;
    if let Some(obj) = val.as_object() {
        return Ok(from_obj(obj));
    }
    Err(AppError::InvalidArg(
        "ollama did not return json object".into(),
    ))
}

fn from_obj(obj: &serde_json::Map<String, Value>) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for (k, v) in obj {
        if let Some(s) = v.as_str() {
            out.insert(k.clone(), s.to_string());
        }
    }
    out
}
