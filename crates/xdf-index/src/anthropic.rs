//! Anthropic Messages API の最小クライアント (blocking reqwest)
//!
//! - 環境変数 `ANTHROPIC_API_KEY` または `.env` ファイルから読む
//! - 入力: system prompt + user message
//! - 出力: テキスト + token 使用量

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};

/// 既知のモデル名
pub const MODEL_SONNET: &str = "claude-sonnet-4-6";
pub const MODEL_HAIKU: &str = "claude-haiku-4-5-20251001";
pub const MODEL_OPUS: &str = "claude-opus-4-7";

/// モデル名に応じた USD/M tokens 単価 (input, output)
/// 公開価格 (2026年初頭時点の概算)
pub fn pricing(model: &str) -> (f64, f64) {
    match model {
        MODEL_HAIKU => (1.0, 5.0),
        MODEL_SONNET => (3.0, 15.0),
        MODEL_OPUS => (15.0, 75.0),
        _ => (3.0, 15.0), // 未知のモデルは Sonnet 単価で近似
    }
}

/// 1 リクエストあたりのコスト計算 (USD)
pub fn estimate_cost(model: &str, input_tokens: u64, output_tokens: u64) -> f64 {
    let (in_price, out_price) = pricing(model);
    (input_tokens as f64 * in_price + output_tokens as f64 * out_price) / 1_000_000.0
}

/// API リクエスト構造体 (Anthropic Messages API)
#[derive(Debug, Serialize)]
struct MessagesRequest<'a> {
    model: &'a str,
    max_tokens: u32,
    system: &'a str,
    messages: Vec<MessageInput<'a>>,
}

#[derive(Debug, Serialize)]
struct MessageInput<'a> {
    role: &'a str,
    content: &'a str,
}

/// API レスポンスの必要部分のみ
#[derive(Debug, Deserialize)]
struct MessagesResponse {
    content: Vec<ContentBlock>,
    usage: UsageBlock,
    #[serde(default)]
    stop_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ContentBlock {
    #[serde(rename = "type")]
    block_type: String,
    #[serde(default)]
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UsageBlock {
    input_tokens: u64,
    output_tokens: u64,
}

/// API 呼び出し結果
#[derive(Debug)]
pub struct CompletionResult {
    pub text: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// Anthropic Messages API のシンプルなクライアント
pub struct AnthropicClient {
    api_key: String,
    client: reqwest::blocking::Client,
    base_url: String,
}

impl AnthropicClient {
    /// 環境変数 + `.env` から API key を取得して構築
    pub fn from_env() -> Result<Self> {
        // .env がカレントディレクトリにあれば読む (失敗は無視)
        let _ = dotenvy::dotenv();
        let api_key = std::env::var("ANTHROPIC_API_KEY").context(
            "ANTHROPIC_API_KEY not found. Set as env var or create .env with ANTHROPIC_API_KEY=...",
        )?;
        if api_key.is_empty() {
            bail!("ANTHROPIC_API_KEY is empty");
        }
        Self::new(api_key)
    }

    pub fn new(api_key: String) -> Result<Self> {
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()?;
        Ok(Self {
            api_key,
            client,
            base_url: "https://api.anthropic.com".to_string(),
        })
    }

    /// 1 回の Messages API 呼び出し
    pub fn complete(
        &self,
        model: &str,
        system: &str,
        user_message: &str,
        max_tokens: u32,
    ) -> Result<CompletionResult> {
        let req = MessagesRequest {
            model,
            max_tokens,
            system,
            messages: vec![MessageInput {
                role: "user",
                content: user_message,
            }],
        };
        let url = format!("{}/v1/messages", self.base_url);
        let resp = self
            .client
            .post(&url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&req)
            .send()
            .context("Anthropic API request failed")?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().unwrap_or_default();
            bail!(
                "Anthropic API returned {}: {}",
                status,
                body.chars().take(500).collect::<String>()
            );
        }
        let parsed: MessagesResponse = resp
            .json()
            .context("Cannot parse Anthropic API response as JSON")?;

        // text ブロックを連結 (通常 1 ブロックのみ)
        let text = parsed
            .content
            .iter()
            .filter(|c| c.block_type == "text")
            .filter_map(|c| c.text.as_deref())
            .collect::<Vec<_>>()
            .join("");

        if text.is_empty() {
            return Err(anyhow!(
                "Anthropic API returned no text content (stop_reason: {:?})",
                parsed.stop_reason
            ));
        }

        Ok(CompletionResult {
            text,
            input_tokens: parsed.usage.input_tokens,
            output_tokens: parsed.usage.output_tokens,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pricing_known_models() {
        let (i, o) = pricing(MODEL_SONNET);
        assert_eq!(i, 3.0);
        assert_eq!(o, 15.0);
    }

    #[test]
    fn pricing_unknown_falls_back_to_sonnet() {
        let (i, o) = pricing("foo");
        assert_eq!(i, 3.0);
        assert_eq!(o, 15.0);
    }

    #[test]
    fn cost_estimation() {
        // Sonnet で 1000 input + 500 output
        let cost = estimate_cost(MODEL_SONNET, 1000, 500);
        // = 1000 * 3 / 1M + 500 * 15 / 1M
        // = 0.003 + 0.0075 = 0.0105
        assert!((cost - 0.0105).abs() < 1e-9);
    }
}
