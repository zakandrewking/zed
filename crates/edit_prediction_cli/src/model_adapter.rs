use anyhow::{Context as _, Result, anyhow, bail};
use clap::Args;
use cloud_llm_client::predict_edits_v3::PredictEditsV3Request;
use futures::{AsyncReadExt as _, FutureExt as _, pin_mut, select};
use gpui::BackgroundExecutor;
use http_client::{AsyncBody, HttpClient as _, Method as HttpMethod};
use reqwest_client::ReqwestClient;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tiny_http::{Header, Method, Response, Server};
use zeta_prompt::format_zeta_prompt;

#[derive(Debug, Args, Clone)]
pub struct ServeModelAdapterArgs {
    /// Address to bind the local model adapter to.
    #[arg(long, default_value = "127.0.0.1:3297")]
    pub bind: String,
    /// Request path to accept model adapter traffic on.
    #[arg(long, default_value = "/predict")]
    pub path: String,
    /// OpenAI-compatible completions endpoint, such as http://127.0.0.1:8080/v1/completions.
    #[arg(long)]
    pub completions_url: String,
    /// Model name to send to the completions endpoint.
    #[arg(long, default_value = "local-zeta2")]
    pub model: String,
    /// Maximum tokens to generate.
    #[arg(long, default_value_t = 256)]
    pub max_tokens: u64,
    /// Sampling temperature.
    #[arg(long, default_value_t = 0.0)]
    pub temperature: f32,
    /// Optional top-p value.
    #[arg(long)]
    pub top_p: Option<f32>,
    /// Stop sequence to send to the completions endpoint. May be repeated.
    #[arg(long = "stop")]
    pub stop: Vec<String>,
    /// Optional bearer token for local servers that require one.
    #[arg(long)]
    pub bearer_token: Option<String>,
    /// Fail upstream completion calls if they do not respond within this many milliseconds.
    #[arg(long, default_value_t = 30_000)]
    pub timeout_ms: u64,
    /// Exit after serving one successful request.
    #[arg(long)]
    pub once: bool,
}

pub fn run_serve_model_adapter(
    args: &ServeModelAdapterArgs,
    background_executor: BackgroundExecutor,
) -> Result<()> {
    if args.timeout_ms == 0 {
        bail!("--timeout-ms must be greater than zero");
    }

    let server = Server::http(&args.bind)
        .map_err(|error| anyhow!(error).context("failed to bind model adapter"))?;
    let listening_address = server
        .server_addr()
        .to_ip()
        .map(|address| format!("http://127.0.0.1:{}{}", address.port(), args.path))
        .unwrap_or_else(|| format!("http://{}{}", args.bind, args.path));

    println!("Model adapter listening at {listening_address}");
    println!("Use with: ep serve-stub --model-http-url {listening_address}");

    let http_client = ReqwestClient::new();
    let mut request_count = 0u64;

    loop {
        let mut request = match server.recv() {
            Ok(request) => request,
            Err(error) => return Err(anyhow!(error).context("model adapter server error")),
        };

        let method = request.method().clone();
        let url = request.url().to_string();
        if method != Method::Post || url != args.path {
            let response = Response::from_string("Not Found").with_status_code(404);
            request
                .respond(response)
                .map_err(|error| anyhow!(error).context("failed to send 404 response"))?;
            continue;
        }

        let mut raw_body = Vec::new();
        request
            .as_reader()
            .read_to_end(&mut raw_body)
            .context("failed to read adapter request body")?;

        let response =
            match run_adapter_request(args, &http_client, &raw_body, &background_executor) {
                Ok(output) => {
                    request_count += 1;
                    println!(
                        "request #{request_count}: prompt_bytes={} output_bytes={}",
                        raw_body.len(),
                        output.len()
                    );
                    json_response(200, &ModelAdapterResponse { output })?
                }
                Err(error) => {
                    eprintln!("model adapter request failed: {error:#}");
                    Response::from_string(format!("{error:#}")).with_status_code(500)
                }
            };

        request
            .respond(response)
            .map_err(|error| anyhow!(error).context("failed to send model adapter response"))?;

        if args.once {
            return Ok(());
        }
    }
}

fn run_adapter_request(
    args: &ServeModelAdapterArgs,
    http_client: &ReqwestClient,
    raw_body: &[u8],
    background_executor: &BackgroundExecutor,
) -> Result<String> {
    let adapter_request =
        serde_json::from_slice::<ModelAdapterRequest>(raw_body).context("invalid adapter JSON")?;
    let prompt = adapter_request.into_prompt()?;

    smol::block_on(async {
        let request_body = OpenAiCompletionRequest {
            model: args.model.clone(),
            prompt,
            stream: false,
            max_tokens: args.max_tokens,
            temperature: args.temperature,
            top_p: args.top_p,
            stop: args.stop.clone(),
        };
        let request_body =
            serde_json::to_vec(&request_body).context("failed to serialize completion request")?;

        let mut request = http_client::Request::builder()
            .method(HttpMethod::POST)
            .uri(&args.completions_url)
            .header("Content-Type", "application/json");
        if let Some(bearer_token) = &args.bearer_token {
            request = request.header("Authorization", format!("Bearer {bearer_token}"));
        }

        let request = request
            .body(AsyncBody::from(request_body))
            .context("failed to build completion request")?;
        let send = http_client.send(request).fuse();
        let timeout = background_executor
            .timer(Duration::from_millis(args.timeout_ms))
            .fuse();
        pin_mut!(send, timeout);

        let mut response = select! {
            result = send => result.context("failed to send completion request")?,
            _ = timeout => bail!("{} timed out after {} ms", args.completions_url, args.timeout_ms),
        };

        let status = response.status();
        let mut body = Vec::new();
        response
            .body_mut()
            .read_to_end(&mut body)
            .await
            .context("failed to read completion response body")?;

        if !status.is_success() {
            bail!(
                "completion endpoint {} returned {}: {}",
                args.completions_url,
                status,
                String::from_utf8_lossy(&body).trim()
            );
        }

        parse_completion_output(&body)
    })
}

#[derive(Debug, Deserialize)]
struct ModelAdapterRequest {
    input: String,
    prompt: Option<String>,
    request: Option<PredictEditsV3Request>,
}

impl ModelAdapterRequest {
    fn into_prompt(self) -> Result<String> {
        match (self.input.as_str(), self.prompt, self.request) {
            ("prompt", Some(prompt), None) => Ok(prompt),
            ("request-json", None, Some(request)) => {
                format_zeta_prompt(&request.input, Default::default())
                    .context("failed to format prompt from request JSON")
            }
            ("prompt", None, _) => bail!("adapter request input=prompt is missing prompt"),
            ("request-json", _, None) => {
                bail!("adapter request input=request-json is missing request")
            }
            (input, _, _) => bail!("unsupported adapter request input `{input}`"),
        }
    }
}

#[derive(Debug, Serialize)]
struct ModelAdapterResponse {
    output: String,
}

#[derive(Debug, Serialize)]
struct OpenAiCompletionRequest {
    model: String,
    prompt: String,
    stream: bool,
    max_tokens: u64,
    temperature: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f32>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    stop: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAiCompletionResponse {
    choices: Vec<OpenAiCompletionChoice>,
}

#[derive(Debug, Deserialize)]
struct OpenAiCompletionChoice {
    text: Option<String>,
    message: Option<OpenAiCompletionMessage>,
}

#[derive(Debug, Deserialize)]
struct OpenAiCompletionMessage {
    content: Option<String>,
}

fn parse_completion_output(body: &[u8]) -> Result<String> {
    let response = serde_json::from_slice::<OpenAiCompletionResponse>(body)
        .context("invalid completion JSON")?;
    let choice = response
        .choices
        .into_iter()
        .next()
        .context("completion response had no choices")?;
    if let Some(text) = choice.text {
        return Ok(text);
    }
    if let Some(message) = choice.message
        && let Some(content) = message.content
    {
        return Ok(content);
    }
    bail!("completion choice did not include text or message.content")
}

fn json_response<T: Serialize>(
    status_code: u16,
    value: &T,
) -> Result<Response<std::io::Cursor<Vec<u8>>>> {
    let body = serde_json::to_vec(value).context("failed to serialize JSON response")?;
    let mut response = Response::from_data(body).with_status_code(status_code);
    if let Ok(header) = Header::from_bytes("Content-Type".as_bytes(), "application/json".as_bytes())
    {
        response = response.with_header(header);
    }
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn adapter_request_accepts_prompt_input() {
        let request = ModelAdapterRequest {
            input: "prompt".to_string(),
            prompt: Some("native prompt".to_string()),
            request: None,
        };

        assert_eq!(request.into_prompt().unwrap(), "native prompt");
    }

    #[test]
    fn completion_output_supports_completion_and_chat_shapes() {
        let completion = json!({
            "choices": [
                {
                    "text": "raw zeta output"
                }
            ]
        });
        assert_eq!(
            parse_completion_output(completion.to_string().as_bytes()).unwrap(),
            "raw zeta output"
        );

        let chat = json!({
            "choices": [
                {
                    "message": {
                        "content": "chat-shaped output"
                    }
                }
            ]
        });
        assert_eq!(
            parse_completion_output(chat.to_string().as_bytes()).unwrap(),
            "chat-shaped output"
        );
    }
}
