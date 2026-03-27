use std::{
    env, fs,
    future::Future,
    io,
    path::{Path, PathBuf},
    sync::{LazyLock, Mutex},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow};
use base64::Engine;
use bytes::Bytes;
use chrono::Utc;
use rig::http_client::{
    self, HeaderValue, HttpClientExt, LazyBody, MultipartForm, Request, ReqwestClient, Response,
    StreamingResponse,
};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use crate::config::{CodexAuthMode, CodexConfig};

pub(crate) const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const AUTH_ISSUER: &str = "https://auth.openai.com";
const AUTH_ACCOUNTS_API: &str = "https://auth.openai.com/api/accounts";
const OAUTH_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const MODEL_CACHE_FILE: &str = "codex_models_cache.json";
const REFRESH_INTERVAL_DAYS: i64 = 8;
static MODEL_CACHE: LazyLock<Mutex<Option<Vec<CachedCodexModel>>>> =
    LazyLock::new(|| Mutex::new(None));

#[derive(Debug, Clone)]
pub(crate) struct DeviceCodePrompt {
    pub(crate) verification_url: String,
    pub(crate) user_code: String,
}

#[derive(Debug, Clone)]
pub(crate) struct DeviceCodeSession {
    prompt: DeviceCodePrompt,
    device_auth_id: String,
    interval_seconds: u64,
}

impl DeviceCodeSession {
    pub(crate) fn prompt(&self) -> &DeviceCodePrompt {
        &self.prompt
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) struct CachedCodexModel {
    pub(crate) slug: String,
    pub(crate) display_name: String,
}

#[derive(Debug, Deserialize)]
struct UserCodeResponse {
    device_auth_id: String,
    #[serde(alias = "user_code", alias = "usercode")]
    user_code: String,
    #[serde(default, deserialize_with = "deserialize_interval")]
    interval: u64,
}

#[derive(Debug, Serialize)]
struct UserCodeRequest<'a> {
    client_id: &'a str,
}

#[derive(Debug, Serialize)]
struct TokenPollRequest<'a> {
    device_auth_id: &'a str,
    user_code: &'a str,
}

#[derive(Debug, Deserialize)]
struct CodeSuccessResponse {
    authorization_code: String,
    #[allow(dead_code)]
    code_challenge: String,
    code_verifier: String,
}

#[derive(Debug, Deserialize)]
struct TokenExchangeResponse {
    id_token: String,
    access_token: String,
    refresh_token: String,
}

#[derive(Debug, Serialize)]
struct RefreshRequest<'a> {
    client_id: &'a str,
    grant_type: &'a str,
    refresh_token: &'a str,
}

#[derive(Debug, Deserialize)]
struct RefreshResponse {
    id_token: Option<String>,
    access_token: Option<String>,
    refresh_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct IdClaims {
    #[serde(rename = "https://api.openai.com/auth", default)]
    auth: Option<AuthClaims>,
}

#[derive(Debug, Deserialize)]
struct AuthClaims {
    #[serde(default)]
    chatgpt_account_id: Option<String>,
}

fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .build()
        .expect("reqwest client builds")
}

#[derive(Clone, Debug, Default)]
pub(crate) struct CodexHttpClient {
    inner: ReqwestClient,
}

impl CodexHttpClient {
    pub(crate) fn new(inner: ReqwestClient) -> Self {
        Self { inner }
    }
}

impl HttpClientExt for CodexHttpClient {
    fn send<T, U>(
        &self,
        req: Request<T>,
    ) -> impl Future<Output = http_client::Result<Response<LazyBody<U>>>> + Send + 'static
    where
        T: Into<Bytes> + Send,
        U: From<Bytes> + Send + 'static,
    {
        let inner = self.inner.clone();
        let req = normalize_codex_request(req);
        async move {
            let req = req?;
            HttpClientExt::send(&inner, req).await
        }
    }

    fn send_multipart<U>(
        &self,
        req: Request<MultipartForm>,
    ) -> impl Future<Output = http_client::Result<Response<LazyBody<U>>>> + Send + 'static
    where
        U: From<Bytes> + Send + 'static,
    {
        let inner = self.inner.clone();
        async move { HttpClientExt::send_multipart(&inner, req).await }
    }

    fn send_streaming<T>(
        &self,
        req: Request<T>,
    ) -> impl Future<Output = http_client::Result<StreamingResponse>> + Send
    where
        T: Into<Bytes>,
    {
        let inner = self.inner.clone();
        let req = normalize_codex_request(req);
        async move {
            let req = req?;
            let mut response = HttpClientExt::send_streaming(&inner, req).await?;
            if !response.headers().contains_key("content-type") {
                response.headers_mut().insert(
                    "content-type",
                    HeaderValue::from_static("text/event-stream; charset=utf-8"),
                );
            }
            Ok(response)
        }
    }
}

fn normalize_codex_request<T>(req: Request<T>) -> http_client::Result<Request<Bytes>>
where
    T: Into<Bytes>,
{
    let (mut parts, body) = req.into_parts();
    let body = body.into();
    if parts.method != reqwest::Method::POST {
        return Ok(Request::from_parts(parts, body));
    }

    parts
        .headers
        .insert("accept", HeaderValue::from_static("text/event-stream"));

    let normalized = normalize_codex_request_body(&body).unwrap_or(body);
    Ok(Request::from_parts(parts, normalized))
}

fn normalize_codex_request_body(body: &Bytes) -> Option<Bytes> {
    let mut payload: Value = serde_json::from_slice(body).ok()?;
    let input = payload.get_mut("input")?.as_array_mut()?;

    let mut instructions = Vec::new();
    let mut filtered_input = Vec::with_capacity(input.len());

    for item in input.drain(..) {
        let Some(mut object) = item.as_object().cloned() else {
            filtered_input.push(item);
            continue;
        };

        if matches!(
            object.get("type").and_then(Value::as_str),
            Some("item_reference")
        ) {
            continue;
        }

        object.remove("id");

        let role = object.get("role").and_then(Value::as_str);
        if matches!(role, Some("system") | Some("developer")) {
            if let Some(text) = extract_instruction_text(&object) {
                instructions.push(text);
            }
            continue;
        }

        filtered_input.push(Value::Object(object));
    }

    payload["input"] = Value::Array(filtered_input);

    if payload.get("instructions").is_none_or(Value::is_null) && !instructions.is_empty() {
        payload["instructions"] = Value::String(instructions.join("\n\n"));
    }

    serde_json::to_vec(&payload).ok().map(Bytes::from)
}

fn extract_instruction_text(item: &serde_json::Map<String, Value>) -> Option<String> {
    let content = item.get("content")?;
    if let Some(text) = content.as_str() {
        let text = text.trim();
        return (!text.is_empty()).then(|| text.to_string());
    }

    let parts = content.as_array()?;
    let text = parts
        .iter()
        .filter_map(|part| {
            let object = part.as_object()?;
            match object.get("type").and_then(Value::as_str) {
                Some("input_text") | Some("text") => object.get("text").and_then(Value::as_str),
                _ => None,
            }
        })
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n");

    (!text.is_empty()).then_some(text)
}

fn deserialize_interval<'de, D>(deserializer: D) -> std::result::Result<u64, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum IntervalValue {
        Number(u64),
        String(String),
    }

    Ok(match Option::<IntervalValue>::deserialize(deserializer)? {
        Some(IntervalValue::Number(value)) => value,
        Some(IntervalValue::String(value)) => value.parse().map_err(serde::de::Error::custom)?,
        None => 0,
    })
}

pub(crate) async fn begin_device_code_login() -> io::Result<DeviceCodeSession> {
    let request_body = serde_json::to_string(&UserCodeRequest {
        client_id: CLIENT_ID,
    })
    .map_err(io::Error::other)?;
    let response = http_client()
        .post(format!("{AUTH_ACCOUNTS_API}/deviceauth/usercode"))
        .header("Content-Type", "application/json")
        .body(request_body)
        .send()
        .await
        .map_err(io::Error::other)?;

    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "device code login is not enabled for this Codex server",
        ));
    }
    if !response.status().is_success() {
        return Err(io::Error::other(format!(
            "device code request failed with status {}",
            response.status()
        )));
    }

    let body = response.text().await.map_err(io::Error::other)?;
    let payload: UserCodeResponse = serde_json::from_str(&body).map_err(io::Error::other)?;
    Ok(DeviceCodeSession {
        prompt: DeviceCodePrompt {
            verification_url: format!("{AUTH_ISSUER}/codex/device"),
            user_code: payload.user_code,
        },
        device_auth_id: payload.device_auth_id,
        interval_seconds: payload.interval.max(1),
    })
}

pub(crate) async fn complete_device_code_login(
    session: DeviceCodeSession,
) -> io::Result<CodexConfig> {
    let code_response = poll_for_authorization_code(&session).await?;
    let redirect_uri = format!("{AUTH_ISSUER}/deviceauth/callback");
    let exchange = http_client()
        .post(OAUTH_TOKEN_URL)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(format!(
            "grant_type=authorization_code&code={}&redirect_uri={}&client_id={}&code_verifier={}",
            urlencoding::encode(&code_response.authorization_code),
            urlencoding::encode(&redirect_uri),
            urlencoding::encode(CLIENT_ID),
            urlencoding::encode(&code_response.code_verifier)
        ))
        .send()
        .await
        .map_err(io::Error::other)?;

    let exchange_status = exchange.status();
    if !exchange_status.is_success() {
        let body = exchange.text().await.unwrap_or_default();
        return Err(io::Error::other(format!(
            "token endpoint returned status {exchange_status}: {body}"
        )));
    }

    let tokens: TokenExchangeResponse = exchange.json().await.map_err(io::Error::other)?;
    Ok(CodexConfig {
        auth_mode: Some(CodexAuthMode::Chatgpt),
        openai_api_key: None,
        access_token: Some(tokens.access_token),
        refresh_token: Some(tokens.refresh_token),
        id_token: Some(tokens.id_token.clone()),
        account_id: parse_account_id(&tokens.id_token).ok().flatten(),
        last_refresh: Some(Utc::now()),
    })
}

async fn poll_for_authorization_code(
    session: &DeviceCodeSession,
) -> io::Result<CodeSuccessResponse> {
    let start = Instant::now();
    let max_wait = Duration::from_secs(15 * 60);
    loop {
        let request_body = serde_json::to_string(&TokenPollRequest {
            device_auth_id: &session.device_auth_id,
            user_code: &session.prompt.user_code,
        })
        .map_err(io::Error::other)?;
        let response = http_client()
            .post(format!("{AUTH_ACCOUNTS_API}/deviceauth/token"))
            .header("Content-Type", "application/json")
            .body(request_body)
            .send()
            .await
            .map_err(io::Error::other)?;

        if response.status().is_success() {
            return response.json().await.map_err(io::Error::other);
        }

        if matches!(
            response.status(),
            reqwest::StatusCode::FORBIDDEN | reqwest::StatusCode::NOT_FOUND
        ) {
            if start.elapsed() >= max_wait {
                return Err(io::Error::other("device auth timed out after 15 minutes"));
            }
            tokio::time::sleep(Duration::from_secs(session.interval_seconds)).await;
            continue;
        }

        return Err(io::Error::other(format!(
            "device auth failed with status {}",
            response.status()
        )));
    }
}

pub(crate) fn parse_account_id(id_token: &str) -> Result<Option<String>> {
    let payload = decode_jwt_payload(id_token)?;
    let claims: IdClaims = serde_json::from_slice(&payload)?;
    Ok(claims.auth.and_then(|auth| auth.chatgpt_account_id))
}

fn decode_jwt_payload(jwt: &str) -> Result<Vec<u8>> {
    let mut parts = jwt.split('.');
    let (_header, payload, _sig) = match (parts.next(), parts.next(), parts.next()) {
        (Some(header), Some(payload), Some(sig))
            if !header.is_empty() && !payload.is_empty() && !sig.is_empty() =>
        {
            (header, payload, sig)
        }
        _ => return Err(anyhow!("invalid JWT format")),
    };
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .context("failed to decode JWT payload")?)
}

pub(crate) fn should_refresh(config: &CodexConfig) -> bool {
    matches!(config.resolved_auth_mode(), Some(CodexAuthMode::Chatgpt))
        && config
            .last_refresh
            .is_some_and(|value| value < Utc::now() - chrono::Duration::days(REFRESH_INTERVAL_DAYS))
        && config
            .refresh_token
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
}

pub(crate) async fn refresh_auth(config: &CodexConfig) -> io::Result<CodexConfig> {
    let refresh_token = config
        .refresh_token
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| io::Error::other("Codex refresh token is not available"))?;
    let request_body = serde_json::to_string(&RefreshRequest {
        client_id: CLIENT_ID,
        grant_type: "refresh_token",
        refresh_token,
    })
    .map_err(io::Error::other)?;
    let response = http_client()
        .post(OAUTH_TOKEN_URL)
        .header("Content-Type", "application/json")
        .body(request_body)
        .send()
        .await
        .map_err(io::Error::other)?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(io::Error::other(format!(
            "failed to refresh Codex auth: {status}: {body}"
        )));
    }

    let refreshed: RefreshResponse = response.json().await.map_err(io::Error::other)?;
    let id_token = refreshed
        .id_token
        .clone()
        .or_else(|| config.id_token.clone())
        .ok_or_else(|| io::Error::other("Codex refresh response did not include an id_token"))?;
    Ok(CodexConfig {
        auth_mode: Some(CodexAuthMode::Chatgpt),
        openai_api_key: None,
        access_token: refreshed
            .access_token
            .or_else(|| config.access_token.clone()),
        refresh_token: refreshed
            .refresh_token
            .or_else(|| config.refresh_token.clone()),
        account_id: parse_account_id(&id_token)
            .ok()
            .flatten()
            .or_else(|| config.account_id.clone()),
        id_token: Some(id_token),
        last_refresh: Some(Utc::now()),
    })
}

pub(crate) fn load_cached_models() -> io::Result<Vec<CachedCodexModel>> {
    if let Some(models) = MODEL_CACHE
        .lock()
        .expect("codex model cache lock")
        .as_ref()
        .cloned()
    {
        return Ok(models);
    }

    let path = default_cache_path()?;
    let raw = fs::read(&path)?;
    let models: Vec<CachedCodexModel> = serde_json::from_slice(&raw).map_err(io::Error::other)?;
    *MODEL_CACHE.lock().expect("codex model cache lock") = Some(models.clone());
    Ok(models)
}

fn default_cache_path() -> io::Result<PathBuf> {
    let cwd_path = Path::new("config.toml");
    if cwd_path.exists() {
        return Ok(PathBuf::from(MODEL_CACHE_FILE));
    }
    if let Some(home) = env::var_os("HOME") {
        return Ok(PathBuf::from(home)
            .join(".config/oat")
            .join(MODEL_CACHE_FILE));
    }
    Err(io::Error::other(
        "failed to determine a Codex model cache path",
    ))
}

pub(crate) fn api_model_name(model_name: &str) -> &str {
    model_name.strip_prefix("codex/").unwrap_or(model_name)
}

pub(crate) fn display_name(model_name: &str) -> String {
    if !model_name.starts_with("codex/") {
        return model_name.to_string();
    }

    cached_display_name(model_name).unwrap_or_else(|| api_model_name(model_name).to_string())
}

pub(crate) fn cached_display_name(model_name: &str) -> Option<String> {
    let slug = model_name.strip_prefix("codex/")?;
    load_cached_models()
        .ok()?
        .into_iter()
        .find(|model| model.slug == slug)
        .map(|model| model.display_name)
}

#[cfg(test)]
mod tests {
    use super::normalize_codex_request_body;
    use bytes::Bytes;
    use serde_json::{Value, json};

    #[test]
    fn display_name_defaults_to_api_model_name_for_bundled_models() {
        assert_eq!(super::display_name("codex/gpt-5.3-codex"), "gpt-5.3-codex");
        assert_eq!(super::display_name("codex/gpt-5.4"), "gpt-5.4");
        assert_eq!(super::display_name("codex/gpt-5.4-mini"), "gpt-5.4-mini");
        assert_eq!(super::display_name("codex/gpt-5.2"), "gpt-5.2");
    }

    #[test]
    fn api_model_name_strips_codex_namespace() {
        assert_eq!(
            super::api_model_name("codex/gpt-5.3-codex"),
            "gpt-5.3-codex"
        );
        assert_eq!(super::api_model_name("gpt-5.4"), "gpt-5.4");
    }

    #[test]
    fn codex_request_body_moves_system_input_to_instructions() {
        let body = Bytes::from(
            serde_json::to_vec(&json!({
                "model": "gpt-5.3-codex",
                "input": [
                    {
                        "role": "system",
                        "content": [
                            { "type": "input_text", "text": "be concise" }
                        ]
                    },
                    {
                        "role": "user",
                        "content": [
                            { "type": "input_text", "text": "hello" }
                        ]
                    }
                ]
            }))
            .expect("serialize"),
        );

        let normalized = normalize_codex_request_body(&body).expect("normalized");
        let payload: Value = serde_json::from_slice(&normalized).expect("payload");

        assert_eq!(payload["instructions"], "be concise");
        assert_eq!(payload["input"].as_array().expect("input").len(), 1);
        assert_eq!(payload["input"][0]["role"], "user");
    }

    #[test]
    fn codex_request_body_drops_item_reference_and_ids() {
        let body = Bytes::from(
            serde_json::to_vec(&json!({
                "model": "gpt-5.3-codex",
                "input": [
                    { "type": "item_reference", "id": "ref_123" },
                    {
                        "role": "user",
                        "id": "msg_123",
                        "content": [
                            { "type": "input_text", "text": "hello" }
                        ]
                    }
                ]
            }))
            .expect("serialize"),
        );

        let normalized = normalize_codex_request_body(&body).expect("normalized");
        let payload: Value = serde_json::from_slice(&normalized).expect("payload");
        let input = payload["input"].as_array().expect("input");

        assert_eq!(input.len(), 1);
        assert!(input[0].get("id").is_none());
        assert_eq!(input[0]["role"], "user");
    }
}
