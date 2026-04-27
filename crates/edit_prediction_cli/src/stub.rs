use crate::capture_safety::{
    assess_zeta_model_output, expected_editable_range, expected_old_editable_region,
};
use anyhow::{Context as _, Result, anyhow, bail};
use clap::{Args, ValueEnum};
use cloud_llm_client::predict_edits_v3::{PredictEditsV3Request, PredictEditsV3Response};
use futures::{AsyncReadExt as _, AsyncWriteExt as _, FutureExt as _, pin_mut, select};
use gpui::BackgroundExecutor;
use http_client::{AsyncBody, Method as HttpMethod};
use reqwest_client::ReqwestClient;
use std::fs;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tiny_http::{Header, Method, Response, Server};
use zeta_prompt::{excerpt_range_for_format, format_zeta_prompt, udiff::apply_diff_to_string};

#[derive(Debug, Args, Clone)]
pub struct ServeStubArgs {
    /// Address to bind the local native predict-edits stub to.
    #[arg(long, default_value = "127.0.0.1:3000")]
    pub bind: String,
    /// Request path to accept for predict-edits traffic.
    #[arg(long, default_value = "/predict_edits/v3")]
    pub path: String,
    /// Return this exact editable-region text for every request.
    #[arg(long)]
    pub response_text: Option<String>,
    /// Read the editable-region response text from a file.
    #[arg(long)]
    pub response_file: Option<PathBuf>,
    /// Apply a unified diff to the request's editable region and return the result.
    #[arg(long)]
    pub diff_file: Option<PathBuf>,
    /// Parse this raw Zeta model output into a native V3 response.
    #[arg(long)]
    pub model_output_text: Option<String>,
    /// Read raw Zeta model output from a file and parse it into a native V3 response.
    #[arg(long)]
    pub model_output_file: Option<PathBuf>,
    /// Run this command per request and parse stdout as raw Zeta model output.
    #[arg(long)]
    pub model_command: Option<PathBuf>,
    /// Argument to pass to --model-command. May be repeated.
    #[arg(long = "model-command-arg")]
    pub model_command_args: Vec<String>,
    /// Input sent to --model-command stdin.
    #[arg(long, default_value = "prompt")]
    pub model_command_input: ModelCommandInput,
    /// Kill --model-command if it does not finish within this many milliseconds.
    #[arg(long, default_value_t = 30_000)]
    pub model_command_timeout_ms: u64,
    /// Send parsed raw model output even if local safety checks reject it.
    #[arg(long)]
    pub allow_unsafe_model_output: bool,
    /// Return the request's editable region unchanged to produce a local no-op response.
    #[arg(long)]
    pub echo_editable_region: bool,
    /// Forward the raw request to this upstream native predict-edits endpoint.
    #[arg(long)]
    pub passthrough_url: Option<String>,
    /// Inject this bearer token upstream when the incoming request has no Authorization header.
    #[arg(long)]
    pub upstream_bearer_token: Option<String>,
    /// Print the formatted default Zeta prompt for each request.
    #[arg(long)]
    pub print_prompt: bool,
    /// Print the parsed request JSON for each request.
    #[arg(long)]
    pub print_json: bool,
    /// Write parsed request, prompt, and response artifacts per request into this directory.
    #[arg(long)]
    pub artifact_dir: Option<PathBuf>,
    /// Exit after serving one successful request.
    #[arg(long)]
    pub once: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct ModelCommandConfig {
    pub(crate) command: PathBuf,
    pub(crate) args: Vec<String>,
    pub(crate) input: ModelCommandInput,
    pub(crate) timeout_ms: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
pub enum ModelCommandInput {
    /// Send the formatted default Zeta prompt on stdin.
    #[default]
    Prompt,
    /// Send the native PredictEditsV3Request JSON on stdin.
    RequestJson,
}

impl std::fmt::Display for ModelCommandInput {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ModelCommandInput::Prompt => write!(formatter, "prompt"),
            ModelCommandInput::RequestJson => write!(formatter, "request-json"),
        }
    }
}

pub fn run_serve_stub(args: &ServeStubArgs, background_executor: BackgroundExecutor) -> Result<()> {
    let configured_response_sources = [
        args.response_text.is_some(),
        args.response_file.is_some(),
        args.diff_file.is_some(),
        args.model_output_text.is_some(),
        args.model_output_file.is_some(),
        args.model_command.is_some(),
        args.echo_editable_region,
        args.passthrough_url.is_some(),
    ]
    .into_iter()
    .filter(|configured| *configured)
    .count();

    if configured_response_sources > 1 {
        bail!(
            "choose at most one of --response-text, --response-file, --diff-file, --model-output-text, --model-output-file, --model-command, --echo-editable-region, or --passthrough-url"
        );
    }
    if !args.model_command_args.is_empty() && args.model_command.is_none() {
        bail!("--model-command-arg requires --model-command");
    }
    if args.model_command_timeout_ms == 0 {
        bail!("--model-command-timeout-ms must be greater than zero");
    }

    let server = Server::http(&args.bind)
        .map_err(|error| anyhow!(error).context("failed to bind predict-edits stub"))?;
    let listening_address = server
        .server_addr()
        .to_ip()
        .map(|address| format!("http://127.0.0.1:{}{}", address.port(), args.path))
        .unwrap_or_else(|| format!("http://{}{}", args.bind, args.path));

    println!("Predict-edits stub listening at {listening_address}");
    println!("Set ZED_PREDICT_EDITS_URL={listening_address}");

    let mut request_count = 0u64;
    let http_client: Arc<dyn http_client::HttpClient> = Arc::new(ReqwestClient::new());

    loop {
        let mut request = match server.recv() {
            Ok(request) => request,
            Err(error) => return Err(anyhow!(error).context("predict-edits stub server error")),
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

        let request_headers = request
            .headers()
            .iter()
            .map(|header| {
                (
                    header.field.as_str().to_string(),
                    header.value.as_str().to_string(),
                )
            })
            .collect::<Vec<_>>();

        let mut raw_request_body = Vec::new();
        request
            .as_reader()
            .read_to_end(&mut raw_request_body)
            .context("failed to read request body")?;

        let decoded_request_body =
            zstd::decode_all(&raw_request_body[..]).unwrap_or_else(|_| raw_request_body.clone());
        let parsed_request =
            parse_predict_request(&decoded_request_body, args.passthrough_url.is_some())?;

        request_count += 1;

        if let Some(parsed_request) = &parsed_request {
            println!(
                "request #{request_count}: path={} cursor_path={} excerpt_bytes={} events={} related_files={}",
                args.path,
                parsed_request.input.cursor_path.display(),
                parsed_request.input.cursor_excerpt.len(),
                parsed_request.input.events.len(),
                parsed_request
                    .input
                    .related_files
                    .as_ref()
                    .map(|related_files| related_files.len())
                    .unwrap_or(0),
            );
        } else {
            println!(
                "request #{request_count}: path={} raw_bytes={} parse=failed",
                args.path,
                raw_request_body.len(),
            );
        }

        if args.print_json {
            if let Some(parsed_request) = &parsed_request {
                println!("{}", serde_json::to_string_pretty(parsed_request)?);
            }
        }

        let prompt = if args.print_prompt || args.artifact_dir.is_some() {
            parsed_request.as_ref().and_then(format_prompt_safely)
        } else {
            None
        };

        if args.print_prompt {
            if let Some(prompt) = &prompt {
                println!("--- prompt begin ---");
                println!("{prompt}");
                println!("--- prompt end ---");
            }
        }

        let response_payload = if let Some(passthrough_url) = &args.passthrough_url {
            forward_request(
                http_client.as_ref(),
                passthrough_url,
                &request_headers,
                &raw_request_body,
                args.upstream_bearer_token.as_deref(),
            )?
        } else {
            build_local_response_payload(
                args,
                request_count,
                parsed_request.as_ref(),
                &background_executor,
            )?
        };

        if let Some(artifact_dir) = &args.artifact_dir {
            write_request_artifacts(
                artifact_dir,
                request_count,
                &request_headers,
                &raw_request_body,
                parsed_request.as_ref(),
                prompt.as_deref(),
                &response_payload,
            )?;
        }

        let mut response = Response::from_data(response_payload.body.clone())
            .with_status_code(response_payload.status_code);
        for (name, value) in response_payload.headers.iter() {
            if header_is_hop_by_hop(name) || name.eq_ignore_ascii_case("content-length") {
                continue;
            }

            if let Ok(header) = Header::from_bytes(name.as_bytes(), value.as_bytes()) {
                response = response.with_header(header);
            }
        }

        request
            .respond(response)
            .map_err(|error| anyhow!(error).context("failed to send stub response"))?;

        if args.once {
            return Ok(());
        }
    }
}

#[derive(Debug)]
struct ResponsePayload {
    status_code: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    parsed_response: Option<PredictEditsV3Response>,
}

fn parse_predict_request(
    body: &[u8],
    allow_failure: bool,
) -> Result<Option<PredictEditsV3Request>> {
    match serde_json::from_slice::<PredictEditsV3Request>(body) {
        Ok(request) => Ok(Some(request)),
        Err(error) if allow_failure => {
            eprintln!("warning: failed to parse predict-edits request: {error:#}");
            Ok(None)
        }
        Err(error) => Err(error).context("failed to parse predict-edits request"),
    }
}

fn build_local_response_payload(
    args: &ServeStubArgs,
    request_count: u64,
    parsed_request: Option<&PredictEditsV3Request>,
    background_executor: &BackgroundExecutor,
) -> Result<ResponsePayload> {
    let parsed_request =
        parsed_request.context("parsed predict-edits request is required for local stub mode")?;
    if args.model_output_text.is_some()
        || args.model_output_file.is_some()
        || args.model_command.is_some()
    {
        return build_model_output_response_payload(
            args,
            request_count,
            parsed_request,
            background_executor,
        );
    }

    let editable_range =
        excerpt_range_for_format(Default::default(), &parsed_request.input.excerpt_ranges).1;
    let old_editable = parsed_request.input.cursor_excerpt[editable_range.clone()].to_string();
    let output = if let Some(response_text) = &args.response_text {
        response_text.clone()
    } else if let Some(response_file) = &args.response_file {
        fs::read_to_string(response_file)
            .with_context(|| format!("failed to read response file {}", response_file.display()))?
    } else if let Some(diff_file) = &args.diff_file {
        let diff = fs::read_to_string(diff_file)
            .with_context(|| format!("failed to read diff file {}", diff_file.display()))?;
        apply_diff_to_string(&diff, &old_editable)
            .context("failed to apply diff to editable region")?
    } else if args.echo_editable_region {
        old_editable
    } else {
        String::new()
    };

    let response = PredictEditsV3Response {
        request_id: format!("stub-request-{request_count}"),
        editable_range,
        output,
        model_version: Some("local-stub".to_string()),
    };
    let body = serde_json::to_vec(&response).context("failed to serialize stub response")?;

    Ok(ResponsePayload {
        status_code: 200,
        headers: vec![("Content-Type".to_string(), "application/json".to_string())],
        body,
        parsed_response: Some(response),
    })
}

fn build_model_output_response_payload(
    args: &ServeStubArgs,
    request_count: u64,
    parsed_request: &PredictEditsV3Request,
    background_executor: &BackgroundExecutor,
) -> Result<ResponsePayload> {
    let raw_output = read_raw_model_output(args, parsed_request, background_executor)?;

    let format = Default::default();
    let safety = assess_zeta_model_output(
        &format!("request-{request_count:04}"),
        "model-output",
        &raw_output,
        format,
        &parsed_request.input,
    );

    let (editable_range, output, model_version) = if (safety.safe_to_apply
        || args.allow_unsafe_model_output)
        && let Some(parsed_output) = safety.parsed_output
    {
        (
            parsed_output.range_in_excerpt,
            parsed_output.new_editable_region,
            if safety.safe_to_apply {
                "local-stub-model-output"
            } else {
                "local-stub-model-output-unsafe-allowed"
            },
        )
    } else {
        eprintln!(
            "warning: rejected unsafe model output for request #{request_count}: {}",
            if safety.reasons.is_empty() {
                "unknown safety failure".to_string()
            } else {
                safety.reasons.join("; ")
            }
        );
        (
            expected_editable_range(format, &parsed_request.input),
            expected_old_editable_region(format, &parsed_request.input)
                .unwrap_or_default()
                .to_string(),
            "local-stub-model-output-rejected",
        )
    };

    let response = PredictEditsV3Response {
        request_id: format!("stub-request-{request_count}"),
        editable_range,
        output,
        model_version: Some(model_version.to_string()),
    };
    let body = serde_json::to_vec(&response).context("failed to serialize stub response")?;

    Ok(ResponsePayload {
        status_code: 200,
        headers: vec![("Content-Type".to_string(), "application/json".to_string())],
        body,
        parsed_response: Some(response),
    })
}

fn read_raw_model_output(
    args: &ServeStubArgs,
    request: &PredictEditsV3Request,
    background_executor: &BackgroundExecutor,
) -> Result<String> {
    if let Some(model_output_text) = &args.model_output_text {
        return Ok(model_output_text.clone());
    }

    if let Some(model_output_file) = &args.model_output_file {
        return fs::read_to_string(model_output_file).with_context(|| {
            format!(
                "failed to read model output file {}",
                model_output_file.display()
            )
        });
    }

    if args.model_command.is_some() {
        let config = ModelCommandConfig {
            command: args.model_command.clone().expect("checked is_some"),
            args: args.model_command_args.clone(),
            input: args.model_command_input,
            timeout_ms: args.model_command_timeout_ms,
        };
        return run_model_command(&config, request, background_executor);
    }

    Ok(String::new())
}

pub(crate) fn run_model_command(
    config: &ModelCommandConfig,
    request: &PredictEditsV3Request,
    background_executor: &BackgroundExecutor,
) -> Result<String> {
    let input = match config.input {
        ModelCommandInput::Prompt => format_zeta_prompt(&request.input, Default::default())
            .context("failed to format prompt for model command")?,
        ModelCommandInput::RequestJson => serde_json::to_string(request)
            .context("failed to serialize request for model command")?,
    };
    let timeout = Duration::from_millis(config.timeout_ms);

    smol::block_on(async {
        let command = run_model_command_inner(config, &input).fuse();
        let timeout = background_executor.timer(timeout).fuse();
        pin_mut!(command, timeout);

        select! {
            result = command => result,
            _ = timeout => {
                bail!("{} timed out after {} ms", config.command.display(), config.timeout_ms)
            }
        }
    })
}

async fn run_model_command_inner(config: &ModelCommandConfig, input: &str) -> Result<String> {
    let command_path = &config.command;
    let mut child = KillOnDropChild::new(
        smol::process::Command::new(command_path)
            .args(&config.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("failed to spawn model command {}", command_path.display()))?,
    );

    let mut stdin = child.stdin.take().context("failed to open model stdin")?;
    stdin
        .write_all(input.as_bytes())
        .await
        .context("failed to write model command stdin")?;
    stdin
        .close()
        .await
        .context("failed to close model command stdin")?;
    drop(stdin);

    let mut stdout = child
        .stdout
        .take()
        .context("failed to open model command stdout")?;
    let mut stderr = child
        .stderr
        .take()
        .context("failed to open model command stderr")?;
    let read_stdout = async move {
        let mut stdout_bytes = Vec::new();
        stdout.read_to_end(&mut stdout_bytes).await?;
        std::io::Result::Ok(stdout_bytes)
    };
    let read_stderr = async move {
        let mut stderr_bytes = Vec::new();
        stderr.read_to_end(&mut stderr_bytes).await?;
        std::io::Result::Ok(stderr_bytes)
    };
    let wait_for_status = child.status();
    let (stdout, stderr, status) = futures::try_join!(read_stdout, read_stderr, wait_for_status)
        .with_context(|| format!("failed to run model command {}", command_path.display()))?;
    child.mark_finished();

    if !status.success() {
        bail!(
            "model command {} failed with status {}: {}",
            command_path.display(),
            status,
            String::from_utf8_lossy(&stderr).trim()
        );
    }

    String::from_utf8(stdout).context("model command stdout was not valid UTF-8")
}

struct KillOnDropChild {
    child: smol::process::Child,
    finished: bool,
}

impl KillOnDropChild {
    fn new(child: smol::process::Child) -> Self {
        Self {
            child,
            finished: false,
        }
    }

    fn mark_finished(&mut self) {
        self.finished = true;
    }
}

impl std::ops::Deref for KillOnDropChild {
    type Target = smol::process::Child;

    fn deref(&self) -> &Self::Target {
        &self.child
    }
}

impl std::ops::DerefMut for KillOnDropChild {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.child
    }
}

impl Drop for KillOnDropChild {
    fn drop(&mut self) {
        if !self.finished {
            _ = self.child.kill();
        }
    }
}

fn forward_request(
    http_client: &dyn http_client::HttpClient,
    passthrough_url: &str,
    request_headers: &[(String, String)],
    raw_request_body: &[u8],
    upstream_bearer_token: Option<&str>,
) -> Result<ResponsePayload> {
    smol::block_on(async {
        let mut request = http_client::Request::builder()
            .method(HttpMethod::POST)
            .uri(passthrough_url);
        let mut has_authorization_header = false;

        for (name, value) in request_headers {
            if header_is_hop_by_hop(name)
                || name.eq_ignore_ascii_case("host")
                || name.eq_ignore_ascii_case("content-length")
            {
                continue;
            }

            if name.eq_ignore_ascii_case("authorization") {
                has_authorization_header = true;
            }

            request = request.header(name, value);
        }

        if !has_authorization_header && let Some(upstream_bearer_token) = upstream_bearer_token {
            request = request.header("Authorization", format!("Bearer {upstream_bearer_token}"));
        }

        let request = request
            .body(AsyncBody::from(raw_request_body.to_vec()))
            .context("failed to build passthrough request")?;
        let mut response = http_client
            .send(request)
            .await
            .context("failed to forward predict-edits request upstream")?;
        let status_code = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .map(|(name, value)| {
                (
                    name.as_str().to_string(),
                    value.to_str().unwrap_or_default().to_string(),
                )
            })
            .collect::<Vec<_>>();
        let mut body = Vec::new();
        response
            .body_mut()
            .read_to_end(&mut body)
            .await
            .context("failed to read passthrough response body")?;
        let parsed_response = serde_json::from_slice(&body).ok();

        Ok(ResponsePayload {
            status_code,
            headers,
            body,
            parsed_response,
        })
    })
}

fn header_is_hop_by_hop(name: &str) -> bool {
    name.eq_ignore_ascii_case("connection")
        || name.eq_ignore_ascii_case("keep-alive")
        || name.eq_ignore_ascii_case("proxy-authenticate")
        || name.eq_ignore_ascii_case("proxy-authorization")
        || name.eq_ignore_ascii_case("te")
        || name.eq_ignore_ascii_case("trailers")
        || name.eq_ignore_ascii_case("transfer-encoding")
        || name.eq_ignore_ascii_case("upgrade")
}

fn redact_header_value(name: &str, value: &str) -> String {
    if name.eq_ignore_ascii_case("authorization") {
        "REDACTED".to_string()
    } else {
        value.to_string()
    }
}

fn write_headers(path: &Path, headers: &[(String, String)]) -> Result<()> {
    let mut header_lines = String::new();
    for (name, value) in headers {
        header_lines.push_str(name);
        header_lines.push_str(": ");
        header_lines.push_str(&redact_header_value(name, value));
        header_lines.push('\n');
    }

    fs::write(path, header_lines)
        .with_context(|| format!("failed to write header artifacts to {}", path.display()))
}

fn format_prompt_safely(request: &PredictEditsV3Request) -> Option<String> {
    let context_range =
        excerpt_range_for_format(Default::default(), &request.input.excerpt_ranges).0;
    if context_range.end > request.input.cursor_excerpt.len()
        || context_range.start > context_range.end
        || request.input.cursor_offset_in_excerpt < context_range.start
        || request.input.cursor_offset_in_excerpt > context_range.end
    {
        eprintln!(
            "warning: skipped prompt formatting for {} because excerpt ranges were not self-consistent",
            request.input.cursor_path.display()
        );
        return None;
    }

    match catch_unwind(AssertUnwindSafe(|| {
        format_zeta_prompt(&request.input, zeta_prompt::ZetaFormat::default())
    })) {
        Ok(prompt) => prompt,
        Err(_) => {
            eprintln!(
                "warning: failed to format prompt for {}; skipping prompt output",
                request.input.cursor_path.display()
            );
            None
        }
    }
}

fn write_request_artifacts(
    artifact_dir: &Path,
    request_count: u64,
    request_headers: &[(String, String)],
    raw_request_body: &[u8],
    parsed_request: Option<&PredictEditsV3Request>,
    prompt: Option<&str>,
    response_payload: &ResponsePayload,
) -> Result<()> {
    let request_dir = artifact_dir.join(format!("request-{request_count:04}"));
    fs::create_dir_all(&request_dir).with_context(|| {
        format!(
            "failed to create artifact directory {}",
            request_dir.display()
        )
    })?;

    write_headers(&request_dir.join("request_headers.txt"), request_headers)?;
    fs::write(request_dir.join("request_body.bin"), raw_request_body).with_context(|| {
        format!(
            "failed to write request body artifacts to {}",
            request_dir.join("request_body.bin").display()
        )
    })?;

    if let Some(parsed_request) = parsed_request {
        let request_json = serde_json::to_string_pretty(parsed_request)
            .context("failed to serialize request artifacts")?;
        fs::write(request_dir.join("request.json"), request_json).with_context(|| {
            format!(
                "failed to write request artifacts to {}",
                request_dir.join("request.json").display()
            )
        })?;
    }

    if let Some(prompt) = prompt {
        fs::write(request_dir.join("prompt.txt"), prompt).with_context(|| {
            format!(
                "failed to write prompt artifacts to {}",
                request_dir.join("prompt.txt").display()
            )
        })?;
    }

    fs::write(
        request_dir.join("response_status.txt"),
        response_payload.status_code.to_string(),
    )
    .with_context(|| {
        format!(
            "failed to write response status artifacts to {}",
            request_dir.join("response_status.txt").display()
        )
    })?;
    write_headers(
        &request_dir.join("response_headers.txt"),
        &response_payload.headers,
    )?;
    fs::write(
        request_dir.join("response_body.bin"),
        &response_payload.body,
    )
    .with_context(|| {
        format!(
            "failed to write response body artifacts to {}",
            request_dir.join("response_body.bin").display()
        )
    })?;

    if let Some(parsed_response) = &response_payload.parsed_response {
        let response_json = serde_json::to_string_pretty(parsed_response)
            .context("failed to serialize response artifacts")?;
        fs::write(request_dir.join("response.json"), response_json).with_context(|| {
            format!(
                "failed to write response artifacts to {}",
                request_dir.join("response.json").display()
            )
        })?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cloud_llm_client::predict_edits_v3::PredictEditsV3Request;
    use std::sync::Arc;
    use zeta_prompt::ExcerptRanges;

    #[gpui::test]
    async fn model_output_response_rejects_unsafe_output_to_no_op(cx: &mut gpui::TestAppContext) {
        let request = PredictEditsV3Request {
            input: test_prompt_input(),
            trigger: Default::default(),
        };
        let executor = cx.executor();
        let args = ServeStubArgs {
            bind: "127.0.0.1:0".to_string(),
            path: "/predict_edits/v3".to_string(),
            response_text: None,
            response_file: None,
            diff_file: None,
            model_output_text: Some("<|fim_prefix|>\n<<<<<<< CURRENT\nbad\n".to_string()),
            model_output_file: None,
            model_command: None,
            model_command_args: Vec::new(),
            model_command_input: ModelCommandInput::Prompt,
            model_command_timeout_ms: 30_000,
            allow_unsafe_model_output: false,
            echo_editable_region: false,
            passthrough_url: None,
            upstream_bearer_token: None,
            print_prompt: false,
            print_json: false,
            artifact_dir: None,
            once: false,
        };

        let response_payload =
            build_local_response_payload(&args, 1, Some(&request), &executor).unwrap();
        let response = response_payload.parsed_response.unwrap();

        assert_eq!(
            response.editable_range,
            expected_editable_range(Default::default(), &request.input)
        );
        assert_eq!(
            response.output,
            expected_old_editable_region(Default::default(), &request.input).unwrap()
        );
        assert_eq!(
            response.model_version.as_deref(),
            Some("local-stub-model-output-rejected")
        );
    }

    #[gpui::test]
    async fn model_output_response_normalizes_safe_output(cx: &mut gpui::TestAppContext) {
        let request = PredictEditsV3Request {
            input: test_prompt_input(),
            trigger: Default::default(),
        };
        let executor = cx.executor();
        let replacement = format!(
            "{}\n// added by model\n",
            expected_old_editable_region(Default::default(), &request.input).unwrap()
        );
        let args = ServeStubArgs {
            bind: "127.0.0.1:0".to_string(),
            path: "/predict_edits/v3".to_string(),
            response_text: None,
            response_file: None,
            diff_file: None,
            model_output_text: Some(replacement.clone()),
            model_output_file: None,
            model_command: None,
            model_command_args: Vec::new(),
            model_command_input: ModelCommandInput::Prompt,
            model_command_timeout_ms: 30_000,
            allow_unsafe_model_output: false,
            echo_editable_region: false,
            passthrough_url: None,
            upstream_bearer_token: None,
            print_prompt: false,
            print_json: false,
            artifact_dir: None,
            once: false,
        };

        let response_payload =
            build_local_response_payload(&args, 1, Some(&request), &executor).unwrap();
        let response = response_payload.parsed_response.unwrap();

        assert_eq!(
            response.editable_range,
            expected_editable_range(Default::default(), &request.input)
        );
        assert_eq!(response.output, replacement);
        assert_eq!(
            response.model_version.as_deref(),
            Some("local-stub-model-output")
        );
    }

    #[gpui::test]
    async fn model_command_stdout_is_normalized_as_model_output(cx: &mut gpui::TestAppContext) {
        let request = PredictEditsV3Request {
            input: test_prompt_input(),
            trigger: Default::default(),
        };
        let executor = cx.executor();
        let replacement = format!(
            "{}\n// generated by command\n",
            expected_old_editable_region(Default::default(), &request.input).unwrap()
        );
        let directory = tempfile::tempdir().unwrap();
        let script_path = directory.path().join("model.sh");
        let output_path = directory.path().join("output.txt");
        std::fs::write(&script_path, "cat >/dev/null\ncat \"$1\"\n").unwrap();
        std::fs::write(&output_path, &replacement).unwrap();

        let args = ServeStubArgs {
            bind: "127.0.0.1:0".to_string(),
            path: "/predict_edits/v3".to_string(),
            response_text: None,
            response_file: None,
            diff_file: None,
            model_output_text: None,
            model_output_file: None,
            model_command: Some(PathBuf::from("/bin/sh")),
            model_command_args: vec![
                script_path.display().to_string(),
                output_path.display().to_string(),
            ],
            model_command_input: ModelCommandInput::Prompt,
            model_command_timeout_ms: 30_000,
            allow_unsafe_model_output: false,
            echo_editable_region: false,
            passthrough_url: None,
            upstream_bearer_token: None,
            print_prompt: false,
            print_json: false,
            artifact_dir: None,
            once: false,
        };

        let response_payload =
            build_local_response_payload(&args, 1, Some(&request), &executor).unwrap();
        let response = response_payload.parsed_response.unwrap();

        assert_eq!(
            response.editable_range,
            expected_editable_range(Default::default(), &request.input)
        );
        assert_eq!(response.output, replacement);
        assert_eq!(
            response.model_version.as_deref(),
            Some("local-stub-model-output")
        );
    }

    fn test_prompt_input() -> zeta_prompt::ZetaPromptInput {
        let editable = concat!(
            "fn main() {\n",
            "    let message = \"hello\";\n",
            "    println!(\"{}\", message);\n",
            "}\n",
            "// This editable range is long enough for deletion safety checks.\n",
            "// It also keeps the model-output normalization tests realistic.\n",
            "// The exact contents are not important beyond stable byte ranges.\n",
        );
        let suffix = "\nfn helper() {\n    println!(\"helper\");\n}\n";
        let text = format!("{editable}{suffix}");
        let editable_end = editable.len();

        zeta_prompt::ZetaPromptInput {
            cursor_path: Arc::from(Path::new("src/main.rs")),
            cursor_excerpt: Arc::from(text.as_str()),
            cursor_offset_in_excerpt: 12,
            excerpt_start_row: Some(0),
            events: Vec::new(),
            related_files: Some(Vec::new()),
            active_buffer_diagnostics: Vec::new(),
            excerpt_ranges: ExcerptRanges {
                editable_150: 0..editable_end,
                editable_180: 0..editable_end,
                editable_350: 0..editable_end,
                editable_512: Some(0..editable_end),
                editable_150_context_350: 0..text.len(),
                editable_180_context_350: 0..text.len(),
                editable_350_context_150: 0..text.len(),
                editable_350_context_512: Some(0..text.len()),
                editable_350_context_1024: Some(0..text.len()),
                context_4096: Some(0..text.len()),
                context_8192: Some(0..text.len()),
            },
            syntax_ranges: None,
            in_open_source_repo: true,
            can_collect_data: false,
            repo_url: None,
        }
    }
}
