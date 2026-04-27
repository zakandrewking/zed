use crate::capture_summary::{CaptureSummary, capture_request_directories, load_capture_summary};
use anyhow::{Context as _, Result, anyhow, bail};
use chrono::Utc;
use clap::Args;
use cloud_llm_client::predict_edits_v3::PREDICT_EDITS_MODE_HEADER_NAME;
use edit_prediction::example_spec::ExampleSpec;
use serde::Serialize;
use smol::process::Command;
use std::fs;
use std::path::{Path, PathBuf};
use zeta_prompt::Event;

#[derive(Debug, Args, Clone)]
pub struct ImportCapturesArgs {
    /// Capture artifact directory produced by `ep serve-stub --artifact-dir`.
    #[arg(long)]
    pub directory: PathBuf,
    /// Override the repository URL recorded in generated replay fixtures.
    #[arg(long)]
    pub repository_url: Option<String>,
    /// Override the revision recorded in generated replay fixtures.
    #[arg(long)]
    pub revision: Option<String>,
    /// Prefix for generated example names.
    #[arg(long, default_value = "captured-native-request")]
    pub name_prefix: String,
    /// Copy raw request/response `.bin` payloads alongside diffable artifacts.
    #[arg(long)]
    pub include_binary_bodies: bool,
    /// Replace existing imported fixture directories.
    #[arg(long)]
    pub overwrite: bool,
}

#[derive(Debug, Serialize)]
struct ImportedCaptureManifest {
    imported_at: String,
    source_directory: String,
    request_name: String,
    repository_url: String,
    revision: String,
    trigger: String,
    mode: Option<String>,
    response_status: Option<u16>,
    request_id: Option<String>,
    model_version: Option<String>,
    event_count: usize,
    related_file_count: usize,
    prompt_captured: bool,
}

pub fn run_import_captures(args: &ImportCapturesArgs, output_path: Option<&PathBuf>) -> Result<()> {
    let output_dir = output_path.context("import-captures requires -o <output-dir>")?;
    fs::create_dir_all(output_dir)
        .with_context(|| format!("failed to create output directory {}", output_dir.display()))?;

    let repository_url = resolve_repository_url(args.repository_url.clone())?;
    let revision = resolve_revision(args.revision.clone())?;
    let capture_directories = capture_request_directories(&args.directory)?;
    if capture_directories.is_empty() {
        bail!(
            "no request-* directories found in {}",
            args.directory.display()
        );
    }

    let source_name = args
        .directory
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("captures")
        .to_string();

    let mut imported_count = 0usize;
    for capture_directory in capture_directories {
        let capture = load_capture_summary(&capture_directory)?;
        capture
            .parsed_request
            .as_ref()
            .context("capture is missing a parseable request")?;

        let fixture_dir = output_dir.join(&capture.name);
        if fixture_dir.exists() {
            if args.overwrite {
                fs::remove_dir_all(&fixture_dir).with_context(|| {
                    format!(
                        "failed to remove existing fixture directory {}",
                        fixture_dir.display()
                    )
                })?;
            } else {
                bail!(
                    "fixture directory already exists: {} (pass --overwrite to replace it)",
                    fixture_dir.display()
                );
            }
        }
        fs::create_dir_all(&fixture_dir).with_context(|| {
            format!(
                "failed to create fixture directory {}",
                fixture_dir.display()
            )
        })?;

        let spec = build_example_spec(
            &capture,
            &repository_url,
            &revision,
            &args.name_prefix,
            &source_name,
        )?;
        let markdown = spec.to_markdown();
        validate_round_trip(&spec, &markdown)
            .context("generated markdown fixture did not round-trip")?;
        fs::write(fixture_dir.join("spec.md"), markdown)
            .with_context(|| format!("failed to write spec.md in {}", fixture_dir.display()))?;

        let manifest = build_manifest(&capture, &repository_url, &revision, &capture_directory);
        let manifest_json = serde_json::to_string_pretty(&manifest)?;
        fs::write(fixture_dir.join("manifest.json"), manifest_json).with_context(|| {
            format!("failed to write manifest.json in {}", fixture_dir.display())
        })?;

        write_normalized_json(
            &fixture_dir.join("request.json"),
            capture.parsed_request.as_ref(),
        )?;
        write_normalized_json(
            &fixture_dir.join("response.json"),
            capture.parsed_response.as_ref(),
        )?;
        copy_if_present(
            &capture_directory.join("prompt.txt"),
            &fixture_dir.join("prompt.txt"),
        )?;
        copy_if_present(
            &capture_directory.join("request_headers.txt"),
            &fixture_dir.join("request_headers.txt"),
        )?;
        copy_if_present(
            &capture_directory.join("response_headers.txt"),
            &fixture_dir.join("response_headers.txt"),
        )?;
        copy_if_present(
            &capture_directory.join("response_status.txt"),
            &fixture_dir.join("response_status.txt"),
        )?;

        if args.include_binary_bodies {
            copy_if_present(
                &capture_directory.join("request_body.bin"),
                &fixture_dir.join("request_body.bin"),
            )?;
            copy_if_present(
                &capture_directory.join("response_body.bin"),
                &fixture_dir.join("response_body.bin"),
            )?;
        }

        imported_count += 1;
    }

    println!(
        "Imported {imported_count} capture(s) from {} into {}",
        args.directory.display(),
        output_dir.display()
    );

    Ok(())
}

fn validate_round_trip(spec: &ExampleSpec, markdown: &str) -> Result<()> {
    let parsed = ExampleSpec::from_markdown(markdown)?;
    if parsed.name != spec.name {
        bail!(
            "round-trip name mismatch: expected {:?}, got {:?}",
            spec.name,
            parsed.name
        );
    }
    if parsed.cursor_path != spec.cursor_path {
        bail!(
            "round-trip cursor_path mismatch: expected {:?}, got {:?}",
            spec.cursor_path,
            parsed.cursor_path
        );
    }
    if parsed.cursor_position != spec.cursor_position {
        bail!("round-trip cursor_position mismatch");
    }
    if parsed.edit_history != spec.edit_history {
        bail!("round-trip edit_history mismatch");
    }
    Ok(())
}

fn resolve_repository_url(explicit_repository_url: Option<String>) -> Result<String> {
    if let Some(repository_url) = explicit_repository_url {
        return Ok(repository_url);
    }
    run_git_command(&["remote", "get-url", "origin"])
        .context("failed to infer repository URL; pass --repository-url to override")
}

fn resolve_revision(explicit_revision: Option<String>) -> Result<String> {
    if let Some(revision) = explicit_revision {
        return Ok(revision);
    }
    run_git_command(&["rev-parse", "HEAD"])
        .context("failed to infer revision; pass --revision to override")
}

fn run_git_command(args: &[&str]) -> Result<String> {
    let output = smol::block_on(Command::new("git").args(args).output())
        .with_context(|| format!("failed to run git {}", args.join(" ")))?;
    if !output.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8(output.stdout)
        .map_err(|error| anyhow!(error))?
        .trim()
        .to_string())
}

fn build_example_spec(
    capture: &CaptureSummary,
    repository_url: &str,
    revision: &str,
    name_prefix: &str,
    source_name: &str,
) -> Result<ExampleSpec> {
    let request = capture
        .parsed_request
        .as_ref()
        .context("capture is missing request data")?;

    let mut edit_history = String::new();
    for event in &request.input.events {
        let event: &Event = event.as_ref();
        zeta_prompt::write_event(&mut edit_history, event);
        if !edit_history.ends_with('\n') {
            edit_history.push('\n');
        }
    }

    let mode = request_mode(capture);
    let related_file_count = request
        .input
        .related_files
        .as_ref()
        .map(|related_files| related_files.len())
        .unwrap_or(0);
    let response_request_id = capture
        .parsed_response
        .as_ref()
        .map(|response| response.request_id.as_str())
        .unwrap_or("unavailable");
    let response_model_version = capture
        .parsed_response
        .as_ref()
        .and_then(|response| response.model_version.as_deref())
        .unwrap_or("unavailable");
    let reason = format!(
        "Imported from native predict-edits capture artifacts.\n\nSource capture set: `{source_name}`\nCapture request: `{}`\nTrigger: `{:?}`\nMode: `{}`\nResponse status: `{}`\nRequest id: `{response_request_id}`\nModel version: `{response_model_version}`\nPrompt captured: `{}`\nEvent count: `{}`\nRelated file count: `{related_file_count}`\n\nThis fixture preserves the captured request semantics and raw artifacts. It does not include a ground-truth expected patch yet.",
        capture.name,
        request.trigger,
        mode.as_deref().unwrap_or("unknown"),
        capture
            .response_status
            .map(|status| status.to_string())
            .unwrap_or_else(|| "unknown".to_string()),
        if capture.has_prompt { "yes" } else { "no" },
        request.input.events.len(),
    );

    let mut spec = ExampleSpec {
        name: format!("{name_prefix} {source_name} {}", capture.name),
        repository_url: repository_url.to_string(),
        revision: revision.to_string(),
        tags: build_tags(capture),
        reasoning: Some(reason),
        uncommitted_diff: String::new(),
        cursor_path: request.input.cursor_path.clone(),
        cursor_position: String::new(),
        edit_history,
        expected_patches: Vec::new(),
        rejected_patch: None,
        telemetry: None,
        human_feedback: Vec::new(),
        rating: None,
    };
    spec.set_cursor_excerpt(
        &request.input.cursor_excerpt,
        request.input.cursor_offset_in_excerpt,
        "",
    );
    Ok(spec)
}

fn build_tags(capture: &CaptureSummary) -> Vec<String> {
    let mut tags = vec!["native-capture".to_string()];
    if let Some(request) = &capture.parsed_request {
        tags.push(format!("trigger-{:?}", request.trigger).to_lowercase());
    }
    if let Some(mode) = request_mode(capture) {
        tags.push(format!("mode-{mode}"));
    }
    if let Some(model_version) = capture
        .parsed_response
        .as_ref()
        .and_then(|response| response.model_version.as_ref())
    {
        tags.push(format!("model-{}", sanitize_tag(model_version)));
    }
    tags
}

fn sanitize_tag(value: &str) -> String {
    value
        .chars()
        .map(|character| match character {
            'a'..='z' | '0'..='9' => character,
            'A'..='Z' => character.to_ascii_lowercase(),
            _ => '-',
        })
        .collect()
}

fn build_manifest(
    capture: &CaptureSummary,
    repository_url: &str,
    revision: &str,
    capture_directory: &Path,
) -> ImportedCaptureManifest {
    let trigger = capture
        .parsed_request
        .as_ref()
        .map(|request| format!("{:?}", request.trigger))
        .unwrap_or_else(|| "Unavailable".to_string());
    let related_file_count = capture
        .parsed_request
        .as_ref()
        .and_then(|request| request.input.related_files.as_ref().map(Vec::len))
        .unwrap_or(0);

    ImportedCaptureManifest {
        imported_at: Utc::now().to_rfc3339(),
        source_directory: capture_directory.display().to_string(),
        request_name: capture.name.clone(),
        repository_url: repository_url.to_string(),
        revision: revision.to_string(),
        trigger,
        mode: request_mode(capture),
        response_status: capture.response_status,
        request_id: capture
            .parsed_response
            .as_ref()
            .map(|response| response.request_id.clone()),
        model_version: capture
            .parsed_response
            .as_ref()
            .and_then(|response| response.model_version.clone()),
        event_count: capture
            .parsed_request
            .as_ref()
            .map(|request| request.input.events.len())
            .unwrap_or(0),
        related_file_count,
        prompt_captured: capture.has_prompt,
    }
}

fn request_mode(capture: &CaptureSummary) -> Option<String> {
    capture
        .request_headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(PREDICT_EDITS_MODE_HEADER_NAME))
        .map(|(_, value)| value.to_ascii_lowercase())
}

fn write_normalized_json<T: Serialize>(path: &Path, value: Option<&T>) -> Result<()> {
    let Some(value) = value else {
        return Ok(());
    };
    let contents = serde_json::to_string_pretty(value)?;
    fs::write(path, contents).with_context(|| format!("failed to write {}", path.display()))
}

fn copy_if_present(source: &Path, destination: &Path) -> Result<()> {
    if !source.exists() {
        return Ok(());
    }
    let contents = fs::read(source)
        .with_context(|| format!("failed to read capture artifact {}", source.display()))?;
    fs::write(destination, contents)
        .with_context(|| format!("failed to write {}", destination.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use cloud_llm_client::predict_edits_v3::PredictEditsV3Response;
    use std::ops::Range;
    use std::sync::Arc;
    use tempfile::tempdir;
    use zeta_prompt::{ExcerptRanges, ZetaPromptInput};

    #[test]
    fn imports_capture_directory_into_round_trippable_fixture() {
        let capture_root = tempdir().unwrap();
        let request_dir = capture_root.path().join("request-0001");
        fs::create_dir_all(&request_dir).unwrap();

        let request = cloud_llm_client::predict_edits_v3::PredictEditsV3Request {
            input: ZetaPromptInput {
                cursor_path: Arc::from(Path::new("src/main.rs")),
                cursor_excerpt: Arc::from("fn main() {\n    println!(\"hi\");\n}\n"),
                cursor_offset_in_excerpt: 3,
                excerpt_start_row: Some(0),
                events: vec![Arc::new(Event::BufferChange {
                    path: Arc::from(Path::new("src/main.rs")),
                    old_path: Arc::from(Path::new("src/main.rs")),
                    diff: "@@ -1,1 +1,2 @@\n-fn main() {}\n+fn main() {\n+}\n".to_string(),
                    predicted: false,
                    in_open_source_repo: true,
                })],
                related_files: Some(Vec::new()),
                active_buffer_diagnostics: Vec::new(),
                excerpt_ranges: ExcerptRanges {
                    editable_150: Range { start: 0, end: 12 },
                    editable_180: Range { start: 0, end: 12 },
                    editable_350: Range { start: 0, end: 20 },
                    editable_512: Some(Range { start: 0, end: 20 }),
                    editable_150_context_350: Range { start: 0, end: 20 },
                    editable_180_context_350: Range { start: 0, end: 20 },
                    editable_350_context_150: Range { start: 0, end: 20 },
                    editable_350_context_512: Some(Range { start: 0, end: 20 }),
                    editable_350_context_1024: Some(Range { start: 0, end: 20 }),
                    context_4096: Some(Range { start: 0, end: 20 }),
                    context_8192: Some(Range { start: 0, end: 20 }),
                },
                syntax_ranges: None,
                in_open_source_repo: true,
                can_collect_data: false,
                repo_url: None,
            },
            trigger: Default::default(),
        };
        let response = PredictEditsV3Response {
            request_id: "stub-request-1".to_string(),
            output: String::new(),
            editable_range: 0..20,
            model_version: Some("local-stub".to_string()),
        };

        fs::write(
            request_dir.join("request.json"),
            serde_json::to_string_pretty(&request).unwrap(),
        )
        .unwrap();
        fs::write(
            request_dir.join("response.json"),
            serde_json::to_string_pretty(&response).unwrap(),
        )
        .unwrap();
        fs::write(
            request_dir.join("request_headers.txt"),
            "x-zed-predict-edits-mode: eager\n",
        )
        .unwrap();
        fs::write(request_dir.join("response_status.txt"), "200\n").unwrap();
        fs::write(request_dir.join("prompt.txt"), "prompt body").unwrap();

        let output_dir = tempdir().unwrap();
        let args = ImportCapturesArgs {
            directory: capture_root.path().to_path_buf(),
            repository_url: Some("git@github.com:zed-industries/zed.git".to_string()),
            revision: Some("deadbeef".to_string()),
            name_prefix: "captured-native-request".to_string(),
            include_binary_bodies: false,
            overwrite: false,
        };

        run_import_captures(&args, Some(&output_dir.path().to_path_buf())).unwrap();

        let fixture_dir = output_dir.path().join("request-0001");
        let spec_markdown = fs::read_to_string(fixture_dir.join("spec.md")).unwrap();
        let spec = ExampleSpec::from_markdown(&spec_markdown).unwrap();
        assert_eq!(spec.repository_url, "git@github.com:zed-industries/zed.git");
        assert_eq!(spec.revision, "deadbeef");
        assert_eq!(spec.cursor_path.as_ref(), Path::new("src/main.rs"));
        assert!(spec.tags.iter().any(|tag| tag == "native-capture"));
        assert!(fixture_dir.join("manifest.json").exists());
        assert!(fixture_dir.join("request.json").exists());
        assert!(fixture_dir.join("response.json").exists());
        assert!(fixture_dir.join("prompt.txt").exists());
    }
}
