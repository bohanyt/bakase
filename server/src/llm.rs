//! Klien model lewat Azure OpenAI. Chat memakai Responses API dengan streaming, karena
//! model adalah model reasoning dan hanya bisa memakai tool bersama reasoning lewat jalur ini.
//!
//! Request dan event dikirim sebagai JSON mentah (fitur `byot`), supaya tidak terikat
//! tipe library.

use crate::config::AzureConfig;
use async_openai::{
    Client,
    config::{AzureConfig as OaAzure, OpenAIConfig},
};
use futures::{Stream, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone)]
pub struct Llm {
    /// Chat Completions, hanya untuk cek info di `cek-vision`.
    client: Client<OaAzure>,
    /// Responses API lewat endpoint v1 Azure (`{endpoint}/openai/v1`), yang menerima
    /// API key sebagai Bearer.
    responses: Client<OpenAIConfig>,
    deployment: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
    /// Lampiran user di pesan ini. Isinya dibaca server saat giliran dikirim ke model.
    #[serde(skip)]
    pub attachments: Vec<crate::files::FileRef>,
}

impl Llm {
    pub(crate) fn deployment(&self) -> &str {
        &self.deployment
    }

    pub fn new(cfg: &AzureConfig) -> Self {
        let oa = OaAzure::new()
            .with_api_base(cfg.endpoint.trim_end_matches('/'))
            .with_deployment_id(&cfg.deployment)
            .with_api_version(&cfg.api_version)
            .with_api_key(&cfg.api_key);
        // Hanya menyambung yang dibatasi di sini; jeda di antara event dibatasi loop agen.
        let http = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .build()
            .unwrap_or_default();
        let v1 = OpenAIConfig::new()
            .with_api_base(format!("{}/openai/v1", cfg.endpoint.trim_end_matches('/')))
            .with_api_key(&cfg.api_key);
        Self {
            client: Client::build(http.clone(), oa),
            responses: Client::build(http, v1),
            deployment: cfg.deployment.clone(),
        }
    }

    /// Satu permintaan Chat Completions tanpa streaming. `extra` digabung ke request,
    /// misalnya `tools` atau `reasoning_effort`.
    pub async fn complete(&self, messages: Value, extra: Value) -> anyhow::Result<Value> {
        let request = with_fixed(
            extra,
            json!({ "model": self.deployment, "messages": messages }),
        );
        Ok(self
            .client
            .chat()
            .create_byot::<Value, Value>(request)
            .await?)
    }

    /// Satu permintaan Responses API tanpa streaming, tanpa menyimpan apa pun di Azure
    /// (`store: false`). Reasoning dikembalikan terenkripsi supaya bisa dikirim lagi
    /// di giliran berikutnya, seperti cara openai/codex.
    pub async fn respond(&self, input: Value, extra: Value) -> anyhow::Result<Value> {
        self.respond_request(self.request(input, extra, false))
            .await
    }

    pub(crate) async fn respond_request(&self, request: Value) -> anyhow::Result<Value> {
        Ok(self
            .responses
            .responses()
            .create_byot::<Value, Value>(request)
            .await?)
    }

    /// Sama seperti `respond`, tapi mengalirkan event Responses API satu per satu
    /// (`response.output_text.delta`, `response.completed`, dan seterusnya).
    pub async fn respond_stream(
        &self,
        request: &Value,
    ) -> anyhow::Result<impl Stream<Item = anyhow::Result<Value>> + use<>> {
        let stream = self
            .responses
            .responses()
            .create_stream_byot::<&Value, Value>(request)
            .await?;
        Ok(stream.map(|event| event.map_err(|e| anyhow::anyhow!(e))))
    }

    pub(crate) fn request(&self, input: Value, extra: Value, stream: bool) -> Value {
        with_fixed(
            extra,
            json!({
                "model": self.deployment,
                "input": input,
                "store": false,
                "include": ["reasoning.encrypted_content"],
                "stream": stream,
            }),
        )
    }
}

/// Ringkasan error model untuk log, tanpa isi respons. Pesan error mentah dari library bisa
/// memuat potongan jawaban (JSON yang gagal dibaca) atau pesan dari penyedia.
pub fn error_summary(e: &anyhow::Error) -> String {
    use async_openai::error::{OpenAIError, StreamError};
    match e.downcast_ref::<OpenAIError>() {
        Some(OpenAIError::Reqwest(err)) => {
            if err.is_timeout() {
                "waktu habis".into()
            } else if err.is_connect() {
                "tidak bisa terhubung".into()
            } else if let Some(status) = err.status() {
                format!("HTTP {status}")
            } else {
                "kesalahan jaringan".into()
            }
        }
        Some(OpenAIError::ApiError(api)) => {
            let code = api.api_error.code.as_deref();
            format!(
                "API {} ({})",
                api.status_code,
                code.or(api.api_error.r#type.as_deref()).unwrap_or("-")
            )
        }
        Some(OpenAIError::JSONDeserialize(..)) => "jawaban model tidak bisa dibaca".into(),
        Some(OpenAIError::StreamError(stream)) => match stream.as_ref() {
            StreamError::UnknownEvent(_) => "event stream tidak dikenal".into(),
            StreamError::EventStream(_) => "stream terputus".into(),
        },
        Some(OpenAIError::InvalidArgument(_)) => "argumen tidak valid".into(),
        Some(_) => "kesalahan lain dari klien model".into(),
        None => "kesalahan lain".into(),
    }
}

/// Opsi tambahan (`tools`, `tool_choice`, ...) digabung dengan field wajib. Field wajib
/// selalu menang, jadi opsi tambahan tidak bisa mengganti model, input, atau `store: false`.
fn with_fixed(extra: Value, fixed: Value) -> Value {
    let mut request = match extra {
        Value::Object(map) => map,
        _ => serde_json::Map::new(),
    };
    if let Value::Object(fixed) = fixed {
        request.extend(fixed);
    }
    Value::Object(request)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_fields_win_over_extra_options() {
        let request = with_fixed(
            json!({ "store": true, "model": "lain", "tools": [1] }),
            json!({ "model": "vision", "store": false }),
        );
        assert_eq!(
            request,
            json!({ "model": "vision", "store": false, "tools": [1] })
        );
        assert_eq!(
            with_fixed(json!(null), json!({ "store": false })),
            json!({ "store": false })
        );
    }
}
