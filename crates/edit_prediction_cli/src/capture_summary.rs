use anyhow::{Context as _, Result, anyhow};
use clap::Args;
use cloud_llm_client::predict_edits_v3::{
    PredictEditsMode, PredictEditsV3Request, PredictEditsV3Response,
};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Args, Clone)]
pub struct SummarizeCapturesArgs {
    /// Capture artifact directory produced by `ep serve-stub --artifact-dir`.
    #[arg(long)]
    pub directory: PathBuf,
}

pub fn run_summarize_captures(
    args: &SummarizeCapturesArgs,
    output_path: Option<&PathBuf>,
) -> Result<()> {
    let capture_directories = capture_request_directories(&args.directory)?;

    let mut output = String::new();
    output.push_str("# Predict Edits Capture Summary\n\n");
    output.push_str(&format!("Directory: `{}`\n\n", args.directory.display()));
    output.push_str(&format!("Requests: {}\n\n", capture_directories.len()));

    for capture_directory in capture_directories {
        let capture = load_capture_summary(&capture_directory)?;
        output.push_str(&render_capture_summary(&capture));
    }

    if let Some(output_path) = output_path {
        fs::write(output_path, output)
            .with_context(|| format!("failed to write summary to {}", output_path.display()))?;
    } else {
        print!("{output}");
    }

    Ok(())
}

#[derive(Debug)]
pub(crate) struct CaptureSummary {
    pub(crate) name: String,
    pub(crate) request_headers: Vec<(String, String)>,
    pub(crate) response_headers: Vec<(String, String)>,
    pub(crate) response_status: Option<u16>,
    pub(crate) parsed_request: Option<PredictEditsV3Request>,
    pub(crate) parsed_response: Option<PredictEditsV3Response>,
    pub(crate) raw_request_bytes: Option<usize>,
    pub(crate) raw_response_bytes: Option<usize>,
    pub(crate) has_prompt: bool,
}

pub(crate) fn capture_request_directories(directory: &Path) -> Result<Vec<PathBuf>> {
    let capture_directories = fs::read_dir(directory)
        .with_context(|| format!("failed to read capture directory {}", directory.display()))?
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            entry
                .file_type()
                .ok()
                .is_some_and(|file_type| file_type.is_dir())
        })
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("request-"))
        })
        .collect::<Vec<_>>();

    let mut capture_directories = capture_directories;
    capture_directories.sort();
    Ok(capture_directories)
}

pub(crate) fn load_capture_summary(capture_directory: &Path) -> Result<CaptureSummary> {
    let request_headers = read_headers(&capture_directory.join("request_headers.txt"))?;
    let response_headers = read_headers(&capture_directory.join("response_headers.txt"))?;
    let response_status = read_optional_text(&capture_directory.join("response_status.txt"))?
        .map(|value| value.trim().parse::<u16>())
        .transpose()
        .context("failed to parse response status")?;
    let parsed_request = load_request(capture_directory)?;
    let parsed_response = load_response(capture_directory)?;
    let raw_request_bytes = file_len(&capture_directory.join("request_body.bin"))?;
    let raw_response_bytes = file_len(&capture_directory.join("response_body.bin"))?;
    let has_prompt = capture_directory.join("prompt.txt").exists();

    Ok(CaptureSummary {
        name: capture_directory
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("request")
            .to_string(),
        request_headers,
        response_headers,
        response_status,
        parsed_request,
        parsed_response,
        raw_request_bytes,
        raw_response_bytes,
        has_prompt,
    })
}

fn load_request(capture_directory: &Path) -> Result<Option<PredictEditsV3Request>> {
    if let Some(request_json) = read_optional_text(&capture_directory.join("request.json"))? {
        return Ok(Some(
            serde_json::from_str(&request_json).context("failed to parse request.json")?,
        ));
    }

    let Some(request_body) = read_optional_bytes(&capture_directory.join("request_body.bin"))?
    else {
        return Ok(None);
    };

    let decoded_request_body = zstd::decode_all(&request_body[..]).unwrap_or(request_body);
    match serde_json::from_slice(&decoded_request_body) {
        Ok(request) => Ok(Some(request)),
        Err(_) => Ok(None),
    }
}

fn load_response(capture_directory: &Path) -> Result<Option<PredictEditsV3Response>> {
    if let Some(response_json) = read_optional_text(&capture_directory.join("response.json"))? {
        return Ok(Some(
            serde_json::from_str(&response_json).context("failed to parse response.json")?,
        ));
    }

    let Some(response_body) = read_optional_bytes(&capture_directory.join("response_body.bin"))?
    else {
        return Ok(None);
    };

    match serde_json::from_slice(&response_body) {
        Ok(response) => Ok(Some(response)),
        Err(_) => Ok(None),
    }
}

pub(crate) fn read_headers(path: &Path) -> Result<Vec<(String, String)>> {
    let Some(contents) = read_optional_text(path)? else {
        return Ok(Vec::new());
    };

    let mut headers = Vec::new();
    for line in contents.lines() {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        headers.push((name.trim().to_string(), value.trim().to_string()));
    }

    Ok(headers)
}

pub(crate) fn read_optional_text(path: &Path) -> Result<Option<String>> {
    if !path.exists() {
        return Ok(None);
    }

    Ok(Some(fs::read_to_string(path).with_context(|| {
        format!("failed to read {}", path.display())
    })?))
}

pub(crate) fn read_optional_bytes(path: &Path) -> Result<Option<Vec<u8>>> {
    if !path.exists() {
        return Ok(None);
    }

    Ok(Some(fs::read(path).with_context(|| {
        format!("failed to read {}", path.display())
    })?))
}

fn file_len(path: &Path) -> Result<Option<usize>> {
    if !path.exists() {
        return Ok(None);
    }

    let metadata = fs::metadata(path)
        .with_context(|| format!("failed to stat capture artifact {}", path.display()))?;
    usize::try_from(metadata.len())
        .map(Some)
        .map_err(|_| anyhow!("file too large to summarize: {}", path.display()))
}

fn render_capture_summary(capture: &CaptureSummary) -> String {
    let mut output = String::new();
    output.push_str(&format!("## {}\n\n", capture.name));
    output.push_str(&format!(
        "- Status: {}\n",
        capture
            .response_status
            .map(|status| status.to_string())
            .unwrap_or_else(|| "unknown".to_string())
    ));
    output.push_str(&format!(
        "- Request bytes: {}\n",
        capture
            .raw_request_bytes
            .map(|length| length.to_string())
            .unwrap_or_else(|| "unknown".to_string())
    ));
    output.push_str(&format!(
        "- Response bytes: {}\n",
        capture
            .raw_response_bytes
            .map(|length| length.to_string())
            .unwrap_or_else(|| "unknown".to_string())
    ));
    output.push_str(&format!(
        "- Prompt captured: {}\n",
        if capture.has_prompt { "yes" } else { "no" }
    ));

    if let Some(parsed_request) = &capture.parsed_request {
        output.push_str(&format!(
            "- Cursor path: `{}`\n",
            parsed_request.input.cursor_path.display()
        ));
        output.push_str(&format!(
            "- Cursor excerpt bytes: {}\n",
            parsed_request.input.cursor_excerpt.len()
        ));
        output.push_str(&format!(
            "- Cursor offset in excerpt: {}\n",
            parsed_request.input.cursor_offset_in_excerpt
        ));
        output.push_str(&format!("- Trigger: `{:?}`\n", parsed_request.trigger));
        output.push_str(&format!(
            "- Event count: {}\n",
            parsed_request.input.events.len()
        ));
        output.push_str(&format!(
            "- Related file count: {}\n",
            parsed_request
                .input
                .related_files
                .as_ref()
                .map(|related_files| related_files.len())
                .unwrap_or(0)
        ));
    } else {
        output.push_str("- Parsed request: unavailable\n");
    }

    let mode_header = capture
        .request_headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("X-Zed-Predict-Edits-Mode"))
        .map(|(_, value)| value.as_str())
        .unwrap_or("missing");
    output.push_str(&format!("- Mode header: `{mode_header}`\n"));

    if let Some(parsed_response) = &capture.parsed_response {
        output.push_str(&format!(
            "- Response request id: `{}`\n",
            parsed_response.request_id
        ));
        output.push_str(&format!(
            "- Output length: {}\n",
            parsed_response.output.len()
        ));
        output.push_str(&format!(
            "- Editable range: {}..{}\n",
            parsed_response.editable_range.start, parsed_response.editable_range.end
        ));
        output.push_str(&format!(
            "- Model version: `{}`\n",
            parsed_response
                .model_version
                .as_deref()
                .unwrap_or("missing")
        ));
    } else {
        output.push_str("- Parsed response: unavailable\n");
    }

    if !capture.request_headers.is_empty() {
        output.push_str("- Request headers:\n");
        for (name, value) in &capture.request_headers {
            output.push_str(&format!("  - `{name}`: `{value}`\n"));
        }
    }

    if !capture.response_headers.is_empty() {
        output.push_str("- Response headers:\n");
        for (name, value) in &capture.response_headers {
            output.push_str(&format!("  - `{name}`: `{value}`\n"));
        }
    }

    if let Some(parsed_request) = &capture.parsed_request {
        if let Ok(mode) = mode_header.parse::<PredictEditsMode>() {
            output.push_str(&format!(
                "- Request synopsis: `{}` with {:?} mode\n",
                parsed_request.input.cursor_path.display(),
                mode
            ));
        }
    }

    output.push('\n');
    output
}
