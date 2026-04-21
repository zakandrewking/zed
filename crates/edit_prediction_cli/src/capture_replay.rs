use crate::capture_summary::{
    capture_request_directories, load_capture_summary, read_optional_text,
};
use anyhow::{Context as _, Result};
use clap::Args;
use similar::TextDiff;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use zeta_prompt::{
    ZetaFormat, excerpt_range_for_format, format_zeta_prompt, parse_zeta2_model_output,
};

#[derive(Debug, Args, Clone)]
pub struct ReplayCapturesArgs {
    /// Fixture or capture directory containing request-* subdirectories.
    #[arg(long)]
    pub directory: PathBuf,
    /// Zeta prompt format to replay with.
    #[arg(long)]
    pub format: Option<String>,
}

#[derive(Debug)]
struct ReplayResult {
    name: String,
    prompt_captured: bool,
    prompt_regenerated: bool,
    prompt_exact_match: Option<bool>,
    prompt_diff_summary: Option<String>,
    response_present: bool,
    response_range_matches_current_format: Option<bool>,
    response_parse_ok: Option<bool>,
    response_status: Option<u16>,
    event_count: usize,
    related_file_count: usize,
}

pub fn run_replay_captures(args: &ReplayCapturesArgs, output_path: Option<&PathBuf>) -> Result<()> {
    let format = args
        .format
        .as_deref()
        .map(ZetaFormat::parse)
        .transpose()?
        .unwrap_or_default();
    let capture_directories = capture_request_directories(&args.directory)?;
    let mut results = Vec::new();
    for capture_directory in capture_directories {
        let capture = load_capture_summary(&capture_directory)?;
        let captured_prompt = read_optional_text(&capture_directory.join("prompt.txt"))?;
        results.push(replay_capture(&capture, captured_prompt.as_deref(), format));
    }

    let output = render_replay_report(&args.directory, format, &results);
    if let Some(output_path) = output_path {
        std::fs::write(output_path, output).with_context(|| {
            format!("failed to write replay report to {}", output_path.display())
        })?;
    } else {
        print!("{output}");
    }

    Ok(())
}

fn replay_capture(
    capture: &crate::capture_summary::CaptureSummary,
    captured_prompt: Option<&str>,
    format: ZetaFormat,
) -> ReplayResult {
    let Some(request) = capture.parsed_request.as_ref() else {
        return ReplayResult {
            name: capture.name.clone(),
            prompt_captured: captured_prompt.is_some(),
            prompt_regenerated: false,
            prompt_exact_match: None,
            prompt_diff_summary: None,
            response_present: capture.parsed_response.is_some(),
            response_range_matches_current_format: None,
            response_parse_ok: None,
            response_status: capture.response_status,
            event_count: 0,
            related_file_count: 0,
        };
    };

    let regenerated_prompt = format_zeta_prompt(&request.input, format);
    let prompt_exact_match = match (captured_prompt, regenerated_prompt.as_deref()) {
        (Some(captured), Some(regenerated)) => Some(captured == regenerated),
        _ => None,
    };
    let prompt_diff_summary = match (
        captured_prompt,
        regenerated_prompt.as_deref(),
        prompt_exact_match,
    ) {
        (Some(captured), Some(regenerated), Some(false)) => {
            first_prompt_diff(captured, regenerated)
        }
        _ => None,
    };

    let response_range_matches_current_format = capture.parsed_response.as_ref().map(|response| {
        let expected_range = excerpt_range_for_format(format, &request.input.excerpt_ranges).1;
        response.editable_range == expected_range
    });
    let response_parse_ok = capture
        .parsed_response
        .as_ref()
        .map(|response| parse_zeta2_model_output(&response.output, format, &request.input).is_ok());

    ReplayResult {
        name: capture.name.clone(),
        prompt_captured: captured_prompt.is_some(),
        prompt_regenerated: regenerated_prompt.is_some(),
        prompt_exact_match,
        prompt_diff_summary,
        response_present: capture.parsed_response.is_some(),
        response_range_matches_current_format,
        response_parse_ok,
        response_status: capture.response_status,
        event_count: request.input.events.len(),
        related_file_count: request
            .input
            .related_files
            .as_ref()
            .map(|related_files| related_files.len())
            .unwrap_or(0),
    }
}

fn first_prompt_diff(captured: &str, regenerated: &str) -> Option<String> {
    let diff = TextDiff::from_lines(captured, regenerated);
    for change in diff.iter_all_changes() {
        let old_index = change.old_index().map(|index| index + 1);
        let new_index = change.new_index().map(|index| index + 1);
        if change.tag() == similar::ChangeTag::Equal {
            continue;
        }
        let snippet = change
            .to_string()
            .trim_end_matches('\n')
            .chars()
            .take(120)
            .collect::<String>();
        return Some(format!(
            "old_line={:?} new_line={:?} change={:?} snippet={:?}",
            old_index,
            new_index,
            change.tag(),
            snippet
        ));
    }
    None
}

fn render_replay_report(directory: &Path, format: ZetaFormat, results: &[ReplayResult]) -> String {
    let total = results.len();
    let prompt_captured = results
        .iter()
        .filter(|result| result.prompt_captured)
        .count();
    let prompt_regenerated = results
        .iter()
        .filter(|result| result.prompt_regenerated)
        .count();
    let prompt_exact_matches = results
        .iter()
        .filter(|result| result.prompt_exact_match == Some(true))
        .count();
    let response_present = results
        .iter()
        .filter(|result| result.response_present)
        .count();
    let response_range_matches = results
        .iter()
        .filter(|result| result.response_range_matches_current_format == Some(true))
        .count();
    let response_parse_ok = results
        .iter()
        .filter(|result| result.response_parse_ok == Some(true))
        .count();

    let mut output = String::new();
    _ = writeln!(output, "# Capture Replay Report");
    _ = writeln!(output);
    _ = writeln!(output, "Directory: `{}`", directory.display());
    _ = writeln!(output, "Format: `{}`", format);
    _ = writeln!(output, "Fixtures: `{total}`");
    _ = writeln!(output, "Prompts captured: `{prompt_captured}`");
    _ = writeln!(output, "Prompts regenerated: `{prompt_regenerated}`");
    _ = writeln!(output, "Prompt exact matches: `{prompt_exact_matches}`");
    _ = writeln!(output, "Responses present: `{response_present}`");
    _ = writeln!(
        output,
        "Response editable-range matches current format: `{response_range_matches}`"
    );
    _ = writeln!(output, "Response parse successes: `{response_parse_ok}`");
    _ = writeln!(output);

    for result in results {
        _ = writeln!(output, "## {}", result.name);
        _ = writeln!(output);
        _ = writeln!(
            output,
            "- Prompt captured: `{}`",
            if result.prompt_captured { "yes" } else { "no" }
        );
        _ = writeln!(
            output,
            "- Prompt regenerated: `{}`",
            if result.prompt_regenerated {
                "yes"
            } else {
                "no"
            }
        );
        _ = writeln!(
            output,
            "- Prompt exact match: `{}`",
            result
                .prompt_exact_match
                .map(|value| if value { "yes" } else { "no" })
                .unwrap_or("n/a")
        );
        if let Some(prompt_diff_summary) = &result.prompt_diff_summary {
            _ = writeln!(output, "- Prompt diff: `{prompt_diff_summary}`");
        }
        _ = writeln!(
            output,
            "- Response present: `{}`",
            if result.response_present { "yes" } else { "no" }
        );
        _ = writeln!(
            output,
            "- Response status: `{}`",
            result
                .response_status
                .map(|status| status.to_string())
                .unwrap_or_else(|| "unknown".to_string())
        );
        _ = writeln!(
            output,
            "- Response editable-range matches current format: `{}`",
            result
                .response_range_matches_current_format
                .map(|value| if value { "yes" } else { "no" })
                .unwrap_or("n/a")
        );
        _ = writeln!(
            output,
            "- Response parse ok: `{}`",
            result
                .response_parse_ok
                .map(|value| if value { "yes" } else { "no" })
                .unwrap_or("n/a")
        );
        _ = writeln!(output, "- Event count: `{}`", result.event_count);
        _ = writeln!(
            output,
            "- Related file count: `{}`",
            result.related_file_count
        );
        _ = writeln!(output);
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use cloud_llm_client::predict_edits_v3::{PredictEditsV3Request, PredictEditsV3Response};
    use std::ops::Range;
    use std::sync::Arc;
    use tempfile::tempdir;
    use zeta_prompt::{ExcerptRanges, ZetaPromptInput};

    #[test]
    fn replays_captured_prompt_exactly() {
        let directory = tempdir().unwrap();
        let request_dir = directory.path().join("request-0001");
        std::fs::create_dir_all(&request_dir).unwrap();

        let request = PredictEditsV3Request {
            input: ZetaPromptInput {
                cursor_path: Arc::from(Path::new("src/main.rs")),
                cursor_excerpt: Arc::from("fn main() {\n    println!(\"hi\");\n}\n"),
                cursor_offset_in_excerpt: 3,
                excerpt_start_row: Some(0),
                events: Vec::new(),
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
        let prompt = format_zeta_prompt(&request.input, ZetaFormat::default()).unwrap();
        let response = PredictEditsV3Response {
            request_id: "stub-request-1".to_string(),
            output: String::new(),
            editable_range: excerpt_range_for_format(
                ZetaFormat::default(),
                &request.input.excerpt_ranges,
            )
            .1,
            model_version: Some("local-stub".to_string()),
        };

        std::fs::write(
            request_dir.join("request.json"),
            serde_json::to_string_pretty(&request).unwrap(),
        )
        .unwrap();
        std::fs::write(
            request_dir.join("response.json"),
            serde_json::to_string_pretty(&response).unwrap(),
        )
        .unwrap();
        std::fs::write(request_dir.join("prompt.txt"), &prompt).unwrap();
        std::fs::write(request_dir.join("response_status.txt"), "200\n").unwrap();

        let args = ReplayCapturesArgs {
            directory: directory.path().to_path_buf(),
            format: None,
        };
        let output_path = directory.path().join("report.md");
        run_replay_captures(&args, Some(&output_path)).unwrap();

        let report = std::fs::read_to_string(output_path).unwrap();
        assert!(report.contains("Prompt exact matches: `1`"));
        assert!(report.contains("- Prompt exact match: `yes`"));
        assert!(report.contains("- Response editable-range matches current format: `yes`"));
    }
}
