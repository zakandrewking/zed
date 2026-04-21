use anyhow::{Context as _, Result, anyhow, bail};
use clap::Args;
use cloud_llm_client::predict_edits_v3::{PredictEditsV3Request, PredictEditsV3Response};
use std::fs;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
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
    ]
    .into_iter()
    .filter(|configured| *configured)
    .count();

    if configured_response_sources > 1 {
        bail!("choose at most one of --response-text, --response-file, or --diff-file");
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

    loop {
        let mut request = match server.recv() {
            Ok(request) => request,
            Err(error) => return Err(anyhow!(error).context("predict-edits stub server error")),
        };

        let method = request.method().clone();
        let url = request.url().to_string();
        if method != Method::Post || url != args.path {
            let response = Response::from_string("Not Found")
                .with_status_code(404)
                .with_header(json_content_type_header()?);
            request
                .respond(response)
                .map_err(|error| anyhow!(error).context("failed to send 404 response"))?;
            continue;
        }

        let mut body = Vec::new();
        request
            .as_reader()
            .read_to_end(&mut body)
            .context("failed to read request body")?;

        let body = zstd::decode_all(&body[..]).unwrap_or(body);
        let predict_request: PredictEditsV3Request =
            serde_json::from_slice(&body).context("failed to parse predict-edits request")?;

        request_count += 1;

        println!(
            "request #{request_count}: path={} cursor_path={} excerpt_bytes={} events={} related_files={}",
            args.path,
            predict_request.input.cursor_path.display(),
            predict_request.input.cursor_excerpt.len(),
            predict_request.input.events.len(),
            predict_request
                .input
                .related_files
                .as_ref()
                .map(|related_files| related_files.len())
                .unwrap_or(0),
        );

        if args.print_json {
            println!("{}", serde_json::to_string_pretty(&predict_request)?);
        }

        let prompt = if args.print_prompt || args.artifact_dir.is_some() {
            format_prompt_safely(&predict_request)
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

        let editable_range =
            excerpt_range_for_format(Default::default(), &predict_request.input.excerpt_ranges).1;
        let old_editable = predict_request.input.cursor_excerpt[editable_range.clone()].to_string();
        let output = if let Some(response_text) = &args.response_text {
            response_text.clone()
        } else if let Some(response_file) = &args.response_file {
            fs::read_to_string(response_file).with_context(|| {
                format!("failed to read response file {}", response_file.display())
            })?
        } else if let Some(diff_file) = &args.diff_file {
            let diff = fs::read_to_string(diff_file)
                .with_context(|| format!("failed to read diff file {}", diff_file.display()))?;
            apply_diff_to_string(&diff, &old_editable)
                .context("failed to apply diff to editable region")?
        } else {
            String::new()
        };

        let response = PredictEditsV3Response {
            request_id: format!("stub-request-{request_count}"),
            editable_range,
            output,
            model_version: Some("local-stub".to_string()),
        };

        if let Some(artifact_dir) = &args.artifact_dir {
            write_request_artifacts(
                artifact_dir,
                request_count,
                &predict_request,
                prompt.as_deref(),
                &response,
            )?;
        }

        let response = Response::from_string(serde_json::to_string(&response)?)
            .with_status_code(200)
            .with_header(json_content_type_header()?);
        request
            .respond(response)
            .map_err(|error| anyhow!(error).context("failed to send stub response"))?;

        if args.once {
            return Ok(());
        }
    }
}

fn json_content_type_header() -> Result<Header> {
    Header::from_bytes("Content-Type", "application/json")
        .map_err(|_| anyhow!("failed to construct content-type header"))
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
    artifact_dir: &PathBuf,
    request_count: u64,
    request: &PredictEditsV3Request,
    prompt: Option<&str>,
    response: &PredictEditsV3Response,
) -> Result<()> {
    let request_dir = artifact_dir.join(format!("request-{request_count:04}"));
    fs::create_dir_all(&request_dir).with_context(|| {
        format!(
            "failed to create artifact directory {}",
            request_dir.display()
        )
    })?;

    let request_json =
        serde_json::to_string_pretty(request).context("failed to serialize request artifacts")?;
    fs::write(request_dir.join("request.json"), request_json).with_context(|| {
        format!(
            "failed to write request artifacts to {}",
            request_dir.join("request.json").display()
        )
    })?;

    if let Some(prompt) = prompt {
        fs::write(request_dir.join("prompt.txt"), prompt).with_context(|| {
            format!(
                "failed to write prompt artifacts to {}",
                request_dir.join("prompt.txt").display()
            )
        })?;
    }

    let response_json =
        serde_json::to_string_pretty(response).context("failed to serialize response artifacts")?;
    fs::write(request_dir.join("response.json"), response_json).with_context(|| {
        format!(
            "failed to write response artifacts to {}",
            request_dir.join("response.json").display()
        )
    })?;

    Ok(())
}
