use anyhow::{Context as _, Result, anyhow, bail};
use clap::Args;
use cloud_llm_client::predict_edits_v3::{PredictEditsV3Request, PredictEditsV3Response};
use futures::AsyncReadExt as _;
use http_client::{AsyncBody, Method as HttpMethod};
use reqwest_client::ReqwestClient;
use std::fs;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::Arc;
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

pub fn run_serve_stub(args: &ServeStubArgs) -> Result<()> {
    let configured_response_sources = [
        args.response_text.is_some(),
        args.response_file.is_some(),
        args.diff_file.is_some(),
        args.echo_editable_region,
        args.passthrough_url.is_some(),
    ]
    .into_iter()
    .filter(|configured| *configured)
    .count();

    if configured_response_sources > 1 {
        bail!(
            "choose at most one of --response-text, --response-file, --diff-file, --echo-editable-region, or --passthrough-url"
        );
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
            build_local_response_payload(args, request_count, parsed_request.as_ref())?
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
) -> Result<ResponsePayload> {
    let parsed_request =
        parsed_request.context("parsed predict-edits request is required for local stub mode")?;
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
