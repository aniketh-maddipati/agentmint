//! LM Studio chat-completions streaming client for turn.
//! Used by: the turn gear loop.

use std::collections::VecDeque;

use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EngineConfig {
    pub model: String,
    pub base_url: String,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub seed: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModelSnapshot {
    pub id: Option<String>,
    pub owned_by: Option<String>,
    pub raw: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EngineChunk {
    pub text_delta: String,
    pub token_logprob: Option<f32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EngineCompletion {
    pub text: String,
    pub chunks: Vec<EngineChunk>,
}

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error(transparent)]
    Http(#[from] reqwest::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("engine returned no model id")]
    MissingModel,
}

pub struct Engine {
    client: Client,
    warned_without_logprobs: bool,
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

impl Engine {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
            warned_without_logprobs: false,
        }
    }

    pub async fn fetch_model_snapshot(
        &self,
        base_url: &str,
        requested_model: Option<&str>,
    ) -> Result<ModelSnapshot, EngineError> {
        let url = format!("{}/v1/models", base_url.trim_end_matches('/'));
        let response = self.client.get(url).send().await?;
        let body = response.json::<ModelsResponse>().await?;
        let selected = match requested_model {
            Some(model) => body.data.into_iter().find(|entry| entry.id == model),
            None => body.data.into_iter().next(),
        };
        let selected = selected.ok_or(EngineError::MissingModel)?;
        let raw = serde_json::to_value(&selected)?;
        Ok(ModelSnapshot {
            id: Some(selected.id),
            owned_by: selected.owned_by,
            raw,
        })
    }

    pub async fn stream_completion<F>(
        &mut self,
        config: &EngineConfig,
        messages: &[ChatMessage],
        mut on_chunk: F,
    ) -> Result<EngineCompletion, EngineError>
    where
        F: FnMut(&EngineChunk),
    {
        match self
            .stream_completion_once(config, messages, true, &mut on_chunk)
            .await
        {
            Ok(completion) => Ok(completion),
            Err(error) if logprobs_unsupported(&error) => {
                if !self.warned_without_logprobs {
                    eprintln!(
                        "warning: model rejected logprobs; retrying without token confidence"
                    );
                    self.warned_without_logprobs = true;
                }
                self.stream_completion_once(config, messages, false, &mut on_chunk)
                    .await
            }
            Err(error) => Err(error),
        }
    }

    async fn stream_completion_once<F>(
        &self,
        config: &EngineConfig,
        messages: &[ChatMessage],
        include_logprobs: bool,
        on_chunk: &mut F,
    ) -> Result<EngineCompletion, EngineError>
    where
        F: FnMut(&EngineChunk),
    {
        let url = format!(
            "{}/v1/chat/completions",
            config.base_url.trim_end_matches('/')
        );
        let payload = json!({
            "model": config.model,
            "messages": messages.iter().map(|message| json!({
                "role": message.role,
                "content": message.content,
            })).collect::<Vec<_>>(),
            "temperature": config.temperature,
            "top_p": config.top_p,
            "seed": config.seed,
            "stream": true,
            "logprobs": include_logprobs,
            "top_logprobs": if include_logprobs { Some(1) } else { None::<u8> },
        });
        let mut response = self.client.post(url).json(&payload).send().await?;
        response.error_for_status_ref()?;

        let mut buffered = String::new();
        let mut pending_lines = VecDeque::new();
        let mut text = String::new();
        let mut chunks = Vec::new();

        while let Some(chunk) = response.chunk().await? {
            buffered.push_str(&String::from_utf8_lossy(&chunk));

            while let Some(index) = buffered.find('\n') {
                let line = buffered[..index].trim_end_matches('\r').to_owned();
                buffered = buffered[index + 1..].to_owned();
                pending_lines.push_back(line);
            }

            while let Some(line) = pending_lines.pop_front() {
                if !line.starts_with("data: ") {
                    continue;
                }

                let data = &line[6..];
                if data == "[DONE]" {
                    return Ok(EngineCompletion { text, chunks });
                }

                let event = serde_json::from_str::<ChatCompletionChunk>(data)?;
                let parsed_chunks = event.into_engine_chunks();
                for parsed in parsed_chunks {
                    text.push_str(&parsed.text_delta);
                    on_chunk(&parsed);
                    chunks.push(parsed);
                }
            }
        }

        Ok(EngineCompletion { text, chunks })
    }
}

fn logprobs_unsupported(error: &EngineError) -> bool {
    match error {
        EngineError::Http(inner) => inner
            .status()
            .map(|status| status.is_client_error())
            .unwrap_or(false),
        EngineError::Json(_) | EngineError::MissingModel => false,
    }
}

#[derive(Debug, Deserialize)]
struct ModelsResponse {
    data: Vec<ModelEntry>,
}

#[derive(Debug, Deserialize, Serialize)]
struct ModelEntry {
    id: String,
    #[serde(default)]
    owned_by: Option<String>,
    #[serde(flatten, default)]
    extra: Value,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionChunk {
    choices: Vec<Choice>,
}

impl ChatCompletionChunk {
    fn into_engine_chunks(self) -> Vec<EngineChunk> {
        let mut result = Vec::new();

        for choice in self.choices {
            if let Some(logprobs) = choice.logprobs {
                if let Some(tokens) = logprobs.content {
                    for token in tokens {
                        result.push(EngineChunk {
                            text_delta: token.token,
                            token_logprob: Some(token.logprob),
                        });
                    }
                    continue;
                }
            }

            if let Some(content) = choice.delta.content {
                result.push(EngineChunk {
                    text_delta: content,
                    token_logprob: None,
                });
            }
        }

        result
    }
}

#[derive(Debug, Deserialize)]
struct Choice {
    delta: Delta,
    #[serde(default)]
    logprobs: Option<Logprobs>,
}

#[derive(Debug, Deserialize)]
struct Delta {
    #[serde(default)]
    content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Logprobs {
    #[serde(default)]
    content: Option<Vec<TokenLogprob>>,
}

#[derive(Debug, Deserialize)]
struct TokenLogprob {
    token: String,
    logprob: f32,
}
