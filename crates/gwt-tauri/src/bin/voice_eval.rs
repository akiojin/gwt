//! Offline voice accuracy evaluation utility.
//!
//! Usage:
//!   cargo run -p gwt-tauri --bin voice_eval -- \
//!     --manifest tests/voice_eval/manifest.json \
//!     --qualities fast,balanced,accurate \
//!     --output tests/voice_eval/latest-report.json

use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::env;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

const VOICE_MODELS_ENV: &str = "GWT_VOICE_MODEL_DIR";
const WHISPER_MODEL_BASE_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main";
const WHISPER_SAMPLE_RATE: u32 = 16_000;

#[derive(Debug, Clone)]
struct CliArgs {
    manifest: PathBuf,
    targets: Vec<ModelTarget>,
    output: Option<PathBuf>,
    baseline: Option<PathBuf>,
    max_wer_delta: f64,
    max_cer_delta: f64,
    use_gpu: bool,
}

#[derive(Debug, Clone)]
struct ModelTarget {
    id: String,
    model_name: String,
    file_name: String,
}

#[derive(Debug, Clone, Deserialize)]
struct ManifestSample {
    id: Option<String>,
    audio_path: String,
    reference: String,
    language: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ManifestDocument {
    Array(Vec<ManifestSample>),
    Object {
        samples: Vec<ManifestSample>,
        default_language: Option<String>,
    },
}

#[derive(Debug, Clone)]
struct LoadedSample {
    id: String,
    audio_path: String,
    reference: String,
    language: String,
    samples_16khz: Vec<f32>,
    duration_secs: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SampleMetrics {
    id: String,
    audio_path: String,
    language: String,
    reference: String,
    transcript: String,
    reference_words: usize,
    reference_chars: usize,
    word_distance: usize,
    char_distance: usize,
    wer: f64,
    cer: f64,
    duration_secs: f64,
    latency_ms: f64,
    realtime_factor: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AggregateMetrics {
    sample_count: usize,
    total_reference_words: usize,
    total_reference_chars: usize,
    total_word_distance: usize,
    total_char_distance: usize,
    wer: f64,
    cer: f64,
    avg_latency_ms: f64,
    avg_realtime_factor: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct QualityReport {
    quality: String,
    model_name: String,
    aggregate: AggregateMetrics,
    samples: Vec<SampleMetrics>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VoiceEvalReport {
    generated_at: String,
    manifest_path: String,
    use_gpu: bool,
    qualities: Vec<QualityReport>,
}

fn print_help() {
    println!("voice_eval - Offline Whisper accuracy evaluator");
    println!();
    println!("Required:");
    println!("  --manifest <path>            Path to manifest JSON");
    println!();
    println!("Optional:");
    println!(
        "  --qualities <csv>            Quality aliases: fast,balanced,accurate (default: all)"
    );
    println!("  --models <csv>               Model aliases/files (e.g. medium,large-v3-turbo)");
    println!("                               Presets: popular, popular-lite");
    println!("  --output <path>              Write full JSON report");
    println!("  --baseline <path>            Baseline report JSON for regression check");
    println!("  --max-wer-delta <float>      Allowed WER increase (default: 0.02)");
    println!("  --max-cer-delta <float>      Allowed CER increase (default: 0.02)");
    println!("  --gpu                        Enable GPU flag for whisper context");
    println!("  -h, --help                   Show this help");
}

fn parse_csv(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .map(|v| v.to_string())
        .collect::<Vec<_>>()
}

fn push_unique_target(targets: &mut Vec<ModelTarget>, target: ModelTarget) {
    if targets.iter().any(|t| t.id == target.id) {
        return;
    }
    targets.push(target);
}

fn quality_target(name: &str, model_name: &str, file_name: &str) -> ModelTarget {
    ModelTarget {
        id: name.to_string(),
        model_name: model_name.to_string(),
        file_name: file_name.to_string(),
    }
}

fn resolve_quality_token(token: &str) -> Option<ModelTarget> {
    match token.trim().to_lowercase().as_str() {
        "fast" => Some(quality_target("fast", "tiny", "ggml-tiny.bin")),
        "balanced" => Some(quality_target("balanced", "base", "ggml-base.bin")),
        "accurate" => Some(quality_target("accurate", "small", "ggml-small.bin")),
        _ => None,
    }
}

fn direct_model_target(id: &str, file_name: &str) -> ModelTarget {
    ModelTarget {
        id: id.to_string(),
        model_name: id.to_string(),
        file_name: file_name.to_string(),
    }
}

fn resolve_model_alias(token: &str) -> Option<ModelTarget> {
    match token.trim().to_lowercase().as_str() {
        "tiny" => Some(direct_model_target("tiny", "ggml-tiny.bin")),
        "base" => Some(direct_model_target("base", "ggml-base.bin")),
        "small" => Some(direct_model_target("small", "ggml-small.bin")),
        "medium" => Some(direct_model_target("medium", "ggml-medium.bin")),
        "large-v3" => Some(direct_model_target("large-v3", "ggml-large-v3.bin")),
        "large-v3-turbo" => Some(direct_model_target(
            "large-v3-turbo",
            "ggml-large-v3-turbo.bin",
        )),
        "medium-q5_0" => Some(direct_model_target("medium-q5_0", "ggml-medium-q5_0.bin")),
        "large-v3-q5_0" => Some(direct_model_target(
            "large-v3-q5_0",
            "ggml-large-v3-q5_0.bin",
        )),
        "large-v3-turbo-q5_0" => Some(direct_model_target(
            "large-v3-turbo-q5_0",
            "ggml-large-v3-turbo-q5_0.bin",
        )),
        "medium-q8_0" => Some(direct_model_target("medium-q8_0", "ggml-medium-q8_0.bin")),
        "large-v3-turbo-q8_0" => Some(direct_model_target(
            "large-v3-turbo-q8_0",
            "ggml-large-v3-turbo-q8_0.bin",
        )),
        _ => None,
    }
}

fn resolve_model_token(token: &str) -> Result<Vec<ModelTarget>, String> {
    let normalized = token.trim().to_lowercase();
    if normalized.is_empty() {
        return Ok(Vec::new());
    }

    match normalized.as_str() {
        "popular" => {
            return Ok(vec![
                direct_model_target("medium", "ggml-medium.bin"),
                direct_model_target("large-v3-turbo", "ggml-large-v3-turbo.bin"),
                direct_model_target("large-v3", "ggml-large-v3.bin"),
            ]);
        }
        "popular-lite" => {
            return Ok(vec![
                direct_model_target("medium-q5_0", "ggml-medium-q5_0.bin"),
                direct_model_target("large-v3-turbo-q5_0", "ggml-large-v3-turbo-q5_0.bin"),
                direct_model_target("large-v3-q5_0", "ggml-large-v3-q5_0.bin"),
            ]);
        }
        _ => {}
    }

    if let Some(target) = resolve_model_alias(&normalized) {
        return Ok(vec![target]);
    }

    if let Some(target) = resolve_quality_token(&normalized) {
        return Ok(vec![ModelTarget {
            id: target.model_name.clone(),
            model_name: target.model_name,
            file_name: target.file_name,
        }]);
    }

    if normalized.starts_with("ggml-") && normalized.ends_with(".bin") {
        let model_name = normalized
            .trim_start_matches("ggml-")
            .trim_end_matches(".bin")
            .to_string();
        return Ok(vec![ModelTarget {
            id: model_name.clone(),
            model_name,
            file_name: normalized,
        }]);
    }

    if normalized
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        let file_name = format!("ggml-{normalized}.bin");
        return Ok(vec![ModelTarget {
            id: normalized.clone(),
            model_name: normalized,
            file_name,
        }]);
    }

    Err(format!("Unknown model token: {token}"))
}

fn parse_cli_args() -> Result<CliArgs, String> {
    let mut manifest: Option<PathBuf> = None;
    let mut targets: Vec<ModelTarget> = Vec::new();
    let mut selected_any = false;
    let mut output: Option<PathBuf> = None;
    let mut baseline: Option<PathBuf> = None;
    let mut max_wer_delta = 0.02_f64;
    let mut max_cer_delta = 0.02_f64;
    let mut use_gpu = false;

    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--manifest" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--manifest requires a value".to_string())?;
                manifest = Some(PathBuf::from(value));
            }
            "--qualities" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--qualities requires a value".to_string())?;
                let parsed = parse_csv(&value);
                if parsed.is_empty() {
                    return Err("--qualities must not be empty".to_string());
                }
                selected_any = true;
                for token in parsed {
                    let target = resolve_quality_token(&token)
                        .ok_or_else(|| format!("Unknown quality token: {token}"))?;
                    push_unique_target(&mut targets, target);
                }
            }
            "--models" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--models requires a value".to_string())?;
                let parsed = parse_csv(&value);
                if parsed.is_empty() {
                    return Err("--models must not be empty".to_string());
                }
                selected_any = true;
                for token in parsed {
                    for target in resolve_model_token(&token)? {
                        push_unique_target(&mut targets, target);
                    }
                }
            }
            "--output" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--output requires a value".to_string())?;
                output = Some(PathBuf::from(value));
            }
            "--baseline" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--baseline requires a value".to_string())?;
                baseline = Some(PathBuf::from(value));
            }
            "--max-wer-delta" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--max-wer-delta requires a value".to_string())?;
                max_wer_delta = value
                    .parse::<f64>()
                    .map_err(|_| "--max-wer-delta must be a float".to_string())?;
            }
            "--max-cer-delta" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--max-cer-delta requires a value".to_string())?;
                max_cer_delta = value
                    .parse::<f64>()
                    .map_err(|_| "--max-cer-delta must be a float".to_string())?;
            }
            "--gpu" => {
                use_gpu = true;
            }
            "-h" | "--help" => {
                print_help();
                std::process::exit(0);
            }
            other => {
                return Err(format!("Unknown option: {other}"));
            }
        }
    }

    let manifest = manifest.ok_or_else(|| "--manifest is required".to_string())?;

    if !selected_any {
        for token in ["fast", "balanced", "accurate"] {
            if let Some(target) = resolve_quality_token(token) {
                push_unique_target(&mut targets, target);
            }
        }
    }

    if targets.is_empty() {
        return Err("At least one quality/model must be specified".to_string());
    }

    Ok(CliArgs {
        manifest,
        targets,
        output,
        baseline,
        max_wer_delta,
        max_cer_delta,
        use_gpu,
    })
}

fn whisper_language(language: &str) -> Option<&'static str> {
    match language.trim().to_lowercase().as_str() {
        "ja" => Some("ja"),
        "en" => Some("en"),
        _ => None,
    }
}

fn voice_model_dir() -> Result<PathBuf, String> {
    if let Ok(dir) = env::var(VOICE_MODELS_ENV) {
        let trimmed = dir.trim();
        if !trimmed.is_empty() {
            return Ok(PathBuf::from(trimmed));
        }
    }

    let home = dirs::home_dir().ok_or_else(|| "Failed to resolve home directory".to_string())?;
    Ok(home.join(".gwt").join("models").join("whisper"))
}

fn file_ready(path: &Path) -> bool {
    fs::metadata(path)
        .map(|meta| meta.is_file() && meta.len() > 0)
        .unwrap_or(false)
}

fn download_model_to(path: &Path, file_name: &str) -> Result<(), String> {
    let url = format!("{WHISPER_MODEL_BASE_URL}/{file_name}");
    let tmp_path = path.with_extension("download");

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(60 * 30))
        .build()
        .map_err(|e| format!("Failed to initialize download client: {e}"))?;

    let mut response = client
        .get(&url)
        .send()
        .map_err(|e| format!("Failed to download model from {url}: {e}"))?;

    if !response.status().is_success() {
        return Err(format!(
            "Model download failed with HTTP status {}",
            response.status()
        ));
    }

    let mut file = File::create(&tmp_path)
        .map_err(|e| format!("Failed to create temporary model file: {e}"))?;
    response
        .copy_to(&mut file)
        .map_err(|e| format!("Failed while downloading model bytes: {e}"))?;
    file.flush()
        .map_err(|e| format!("Failed to flush downloaded model bytes: {e}"))?;

    fs::rename(&tmp_path, path).map_err(|e| format!("Failed to finalize model file: {e}"))?;
    Ok(())
}

fn ensure_model_ready(target: &ModelTarget) -> Result<PathBuf, String> {
    let model_dir = voice_model_dir()?;
    fs::create_dir_all(&model_dir)
        .map_err(|e| format!("Failed to create voice model directory: {e}"))?;

    let model_path = model_dir.join(&target.file_name);
    if !file_ready(&model_path) {
        println!(
            "[voice-eval] Downloading model for {} -> {}",
            target.id,
            model_path.display()
        );
        download_model_to(&model_path, &target.file_name)?;
    }

    if !file_ready(&model_path) {
        return Err("Voice model is present but invalid (empty file)".to_string());
    }

    Ok(model_path)
}

fn resample_to_16khz(samples: &[f32], sample_rate: u32) -> Vec<f32> {
    if sample_rate == WHISPER_SAMPLE_RATE || samples.is_empty() {
        return samples.to_vec();
    }

    let ratio = WHISPER_SAMPLE_RATE as f64 / sample_rate as f64;
    let out_len = ((samples.len() as f64) * ratio).max(1.0).round() as usize;
    let mut out = Vec::with_capacity(out_len);

    for i in 0..out_len {
        let src_pos = i as f64 / ratio;
        let idx = src_pos.floor() as usize;
        let frac = (src_pos - idx as f64) as f32;

        let left = samples[idx.min(samples.len() - 1)];
        let right = samples[(idx + 1).min(samples.len() - 1)];
        out.push((left + (right - left) * frac).clamp(-1.0, 1.0));
    }

    out
}

fn read_wav_mono_f32(path: &Path) -> Result<(Vec<f32>, u32), String> {
    let mut reader = hound::WavReader::open(path)
        .map_err(|e| format!("Failed to open wav file {}: {e}", path.display()))?;
    let spec = reader.spec();
    if spec.channels == 0 {
        return Err(format!("WAV has invalid channel count: {}", path.display()));
    }

    let channels = spec.channels as usize;
    let mut interleaved = Vec::<f32>::new();
    match spec.sample_format {
        hound::SampleFormat::Float => {
            for sample in reader.samples::<f32>() {
                let value = sample
                    .map_err(|e| format!("Failed to read float WAV sample: {e}"))?
                    .clamp(-1.0, 1.0);
                interleaved.push(value);
            }
        }
        hound::SampleFormat::Int => {
            let bits = spec.bits_per_sample.max(1);
            let max_amplitude = ((1_i64 << (bits - 1)) - 1).max(1) as f32;
            for sample in reader.samples::<i32>() {
                let value = sample.map_err(|e| format!("Failed to read int WAV sample: {e}"))?
                    as f32
                    / max_amplitude;
                interleaved.push(value.clamp(-1.0, 1.0));
            }
        }
    }

    if interleaved.is_empty() {
        return Ok((Vec::new(), spec.sample_rate));
    }

    if channels == 1 {
        return Ok((interleaved, spec.sample_rate));
    }

    let mut mono = Vec::<f32>::with_capacity(interleaved.len() / channels + 1);
    for frame in interleaved.chunks(channels) {
        let sum = frame.iter().copied().sum::<f32>();
        mono.push((sum / channels as f32).clamp(-1.0, 1.0));
    }
    Ok((mono, spec.sample_rate))
}

fn parse_manifest(path: &Path) -> Result<Vec<ManifestSample>, String> {
    let raw = fs::read_to_string(path)
        .map_err(|e| format!("Failed to read manifest {}: {e}", path.display()))?;
    let doc: ManifestDocument =
        serde_json::from_str(&raw).map_err(|e| format!("Failed to parse manifest JSON: {e}"))?;

    let (samples, default_language) = match doc {
        ManifestDocument::Array(samples) => (samples, "auto".to_string()),
        ManifestDocument::Object {
            samples,
            default_language,
        } => (
            samples,
            default_language.unwrap_or_else(|| "auto".to_string()),
        ),
    };

    if samples.is_empty() {
        return Err("Manifest contains no samples".to_string());
    }

    let normalized = samples
        .into_iter()
        .enumerate()
        .map(|(idx, mut sample)| {
            let id = sample
                .id
                .clone()
                .unwrap_or_else(|| format!("sample-{:03}", idx + 1));
            sample.id = Some(id);
            if sample.language.as_deref().unwrap_or("").trim().is_empty() {
                sample.language = Some(default_language.clone());
            }
            sample
        })
        .collect::<Vec<_>>();

    Ok(normalized)
}

fn resolve_audio_path(manifest_path: &Path, audio_path: &str) -> PathBuf {
    let given = PathBuf::from(audio_path);
    if given.is_absolute() {
        return given;
    }
    let parent = manifest_path.parent().unwrap_or_else(|| Path::new("."));
    parent.join(given)
}

fn load_samples(manifest_path: &Path) -> Result<Vec<LoadedSample>, String> {
    let entries = parse_manifest(manifest_path)?;
    let mut loaded = Vec::<LoadedSample>::with_capacity(entries.len());

    for entry in entries {
        let id = entry.id.unwrap_or_else(|| "sample".to_string());
        let audio_abs = resolve_audio_path(manifest_path, &entry.audio_path);
        let language = entry.language.unwrap_or_else(|| "auto".to_string());
        let (raw_samples, raw_rate) = read_wav_mono_f32(&audio_abs)?;
        let samples_16khz = resample_to_16khz(&raw_samples, raw_rate);
        let duration_secs = if WHISPER_SAMPLE_RATE == 0 {
            0.0
        } else {
            samples_16khz.len() as f64 / WHISPER_SAMPLE_RATE as f64
        };
        loaded.push(LoadedSample {
            id,
            audio_path: audio_abs.to_string_lossy().to_string(),
            reference: entry.reference,
            language,
            samples_16khz,
            duration_secs,
        });
    }

    Ok(loaded)
}

fn transcribe_with_context(
    context: &WhisperContext,
    samples_16khz: &[f32],
    language: &str,
) -> Result<String, String> {
    let mut state = context
        .create_state()
        .map_err(|e| format!("Failed to create whisper state: {e}"))?;
    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 0 });
    let threads = std::thread::available_parallelism()
        .map(|n| n.get() as i32)
        .unwrap_or(2)
        .clamp(1, 16);
    params.set_n_threads(threads);
    params.set_print_special(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    params.set_translate(false);
    params.set_language(whisper_language(language));

    state
        .full(params, samples_16khz)
        .map_err(|e| format!("Whisper transcription failed: {e}"))?;

    let mut transcript = String::new();
    for segment in state.as_iter() {
        transcript.push_str(&segment.to_string());
    }
    Ok(transcript.trim().to_string())
}

fn is_cjk(ch: char) -> bool {
    matches!(
        ch as u32,
        0x3040..=0x30ff   // Hiragana + Katakana
            | 0x3400..=0x4dbf // CJK Extension A
            | 0x4e00..=0x9fff // CJK Unified Ideographs
            | 0xf900..=0xfaff // CJK Compatibility Ideographs
            | 0xff66..=0xff9f // Halfwidth Katakana
    )
}

fn normalize_text(text: &str) -> String {
    let lowered = text.to_lowercase();
    let mut out = String::with_capacity(lowered.len());
    let mut last_space = false;

    for ch in lowered.chars() {
        let mapped = if ch.is_whitespace() {
            ' '
        } else if ch.is_alphanumeric() || is_cjk(ch) {
            ch
        } else {
            ' '
        };

        if mapped == ' ' {
            if !last_space {
                out.push(' ');
                last_space = true;
            }
        } else {
            out.push(mapped);
            last_space = false;
        }
    }

    out.trim().to_string()
}

fn tokenize_words(text: &str) -> Vec<String> {
    normalize_text(text)
        .split_whitespace()
        .map(|v| v.to_string())
        .collect::<Vec<_>>()
}

fn tokenize_chars(text: &str) -> Vec<char> {
    normalize_text(text)
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect::<Vec<_>>()
}

fn levenshtein<T: PartialEq>(left: &[T], right: &[T]) -> usize {
    if left.is_empty() {
        return right.len();
    }
    if right.is_empty() {
        return left.len();
    }

    let mut prev = (0..=right.len()).collect::<Vec<usize>>();
    let mut curr = vec![0_usize; right.len() + 1];

    for (i, left_item) in left.iter().enumerate() {
        curr[0] = i + 1;
        for (j, right_item) in right.iter().enumerate() {
            let substitution_cost = if left_item == right_item { 0 } else { 1 };
            let insert = curr[j] + 1;
            let delete = prev[j + 1] + 1;
            let substitute = prev[j] + substitution_cost;
            curr[j + 1] = insert.min(delete).min(substitute);
        }
        std::mem::swap(&mut prev, &mut curr);
    }

    prev[right.len()]
}

fn compute_sample_metrics(
    sample: &LoadedSample,
    transcript: String,
    latency_ms: f64,
    realtime_factor: f64,
) -> SampleMetrics {
    let ref_words = tokenize_words(&sample.reference);
    let hyp_words = tokenize_words(&transcript);
    let ref_chars = tokenize_chars(&sample.reference);
    let hyp_chars = tokenize_chars(&transcript);

    let word_distance = levenshtein(&ref_words, &hyp_words);
    let char_distance = levenshtein(&ref_chars, &hyp_chars);

    let reference_words = ref_words.len();
    let reference_chars = ref_chars.len();

    let wer = if reference_words == 0 {
        if hyp_words.is_empty() {
            0.0
        } else {
            1.0
        }
    } else {
        word_distance as f64 / reference_words as f64
    };
    let cer = if reference_chars == 0 {
        if hyp_chars.is_empty() {
            0.0
        } else {
            1.0
        }
    } else {
        char_distance as f64 / reference_chars as f64
    };

    SampleMetrics {
        id: sample.id.clone(),
        audio_path: sample.audio_path.clone(),
        language: sample.language.clone(),
        reference: sample.reference.clone(),
        transcript,
        reference_words,
        reference_chars,
        word_distance,
        char_distance,
        wer,
        cer,
        duration_secs: sample.duration_secs,
        latency_ms,
        realtime_factor,
    }
}

fn aggregate_metrics(samples: &[SampleMetrics]) -> AggregateMetrics {
    let sample_count = samples.len();
    let total_reference_words = samples.iter().map(|s| s.reference_words).sum::<usize>();
    let total_reference_chars = samples.iter().map(|s| s.reference_chars).sum::<usize>();
    let total_word_distance = samples.iter().map(|s| s.word_distance).sum::<usize>();
    let total_char_distance = samples.iter().map(|s| s.char_distance).sum::<usize>();
    let total_latency_ms = samples.iter().map(|s| s.latency_ms).sum::<f64>();
    let total_rtf = samples.iter().map(|s| s.realtime_factor).sum::<f64>();

    let wer = if total_reference_words == 0 {
        0.0
    } else {
        total_word_distance as f64 / total_reference_words as f64
    };
    let cer = if total_reference_chars == 0 {
        0.0
    } else {
        total_char_distance as f64 / total_reference_chars as f64
    };

    AggregateMetrics {
        sample_count,
        total_reference_words,
        total_reference_chars,
        total_word_distance,
        total_char_distance,
        wer,
        cer,
        avg_latency_ms: if sample_count == 0 {
            0.0
        } else {
            total_latency_ms / sample_count as f64
        },
        avg_realtime_factor: if sample_count == 0 {
            0.0
        } else {
            total_rtf / sample_count as f64
        },
    }
}

fn evaluate_quality(
    loaded_samples: &[LoadedSample],
    target: &ModelTarget,
    use_gpu: bool,
) -> Result<QualityReport, String> {
    let model_path = ensure_model_ready(target)?;

    let mut context_params = WhisperContextParameters::new();
    context_params.use_gpu(use_gpu);

    let context = WhisperContext::new_with_params(&model_path.to_string_lossy(), context_params)
        .map_err(|e| format!("Failed to load whisper model {}: {e}", model_path.display()))?;

    let mut sample_reports = Vec::<SampleMetrics>::with_capacity(loaded_samples.len());

    for sample in loaded_samples {
        let start = Instant::now();
        let transcript =
            transcribe_with_context(&context, &sample.samples_16khz, &sample.language)?;
        let elapsed = start.elapsed().as_secs_f64();
        let latency_ms = elapsed * 1000.0;
        let realtime_factor = if sample.duration_secs > 0.0 {
            elapsed / sample.duration_secs
        } else {
            0.0
        };

        sample_reports.push(compute_sample_metrics(
            sample,
            transcript,
            latency_ms,
            realtime_factor,
        ));
    }

    Ok(QualityReport {
        quality: target.id.clone(),
        model_name: target.model_name.clone(),
        aggregate: aggregate_metrics(&sample_reports),
        samples: sample_reports,
    })
}

fn print_summary(report: &VoiceEvalReport) {
    println!();
    println!("=== Voice Accuracy Report ===");
    println!("manifest: {}", report.manifest_path);
    println!("gpu: {}", if report.use_gpu { "on" } else { "off" });

    for quality in &report.qualities {
        println!(
            "quality={} model={} WER={:.4} CER={:.4} avg_latency={:.1}ms avg_rtf={:.2}",
            quality.quality,
            quality.model_name,
            quality.aggregate.wer,
            quality.aggregate.cer,
            quality.aggregate.avg_latency_ms,
            quality.aggregate.avg_realtime_factor
        );
    }

    if let Some(best) = report
        .qualities
        .iter()
        .min_by(|a, b| a.aggregate.cer.total_cmp(&b.aggregate.cer))
    {
        println!(
            "best-by-cer: quality={} CER={:.4} WER={:.4}",
            best.quality, best.aggregate.cer, best.aggregate.wer
        );
    }
    println!("=============================");
}

fn save_report(path: &Path, report: &VoiceEvalReport) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            format!(
                "Failed to create output directory {}: {e}",
                parent.display()
            )
        })?;
    }
    let json = serde_json::to_string_pretty(report)
        .map_err(|e| format!("Failed to serialize report: {e}"))?;
    fs::write(path, json).map_err(|e| format!("Failed to write report {}: {e}", path.display()))
}

fn enforce_regression_limit(
    current: &VoiceEvalReport,
    baseline_path: &Path,
    max_wer_delta: f64,
    max_cer_delta: f64,
) -> Result<(), String> {
    let raw = fs::read_to_string(baseline_path).map_err(|e| {
        format!(
            "Failed to read baseline report {}: {e}",
            baseline_path.display()
        )
    })?;
    let baseline: VoiceEvalReport =
        serde_json::from_str(&raw).map_err(|e| format!("Invalid baseline JSON: {e}"))?;

    let baseline_map = baseline
        .qualities
        .iter()
        .map(|q| (q.quality.clone(), q.aggregate.clone()))
        .collect::<HashMap<_, _>>();

    for current_quality in &current.qualities {
        let Some(b) = baseline_map.get(&current_quality.quality) else {
            continue;
        };

        let wer_delta = current_quality.aggregate.wer - b.wer;
        let cer_delta = current_quality.aggregate.cer - b.cer;

        if wer_delta > max_wer_delta {
            return Err(format!(
                "WER regression: quality={} baseline={:.4} current={:.4} delta={:.4} limit={:.4}",
                current_quality.quality,
                b.wer,
                current_quality.aggregate.wer,
                wer_delta,
                max_wer_delta
            ));
        }

        if cer_delta > max_cer_delta {
            return Err(format!(
                "CER regression: quality={} baseline={:.4} current={:.4} delta={:.4} limit={:.4}",
                current_quality.quality,
                b.cer,
                current_quality.aggregate.cer,
                cer_delta,
                max_cer_delta
            ));
        }
    }

    Ok(())
}

fn run() -> Result<(), String> {
    let args = parse_cli_args()?;
    whisper_rs::install_logging_hooks();
    let manifest_abs = fs::canonicalize(&args.manifest).unwrap_or(args.manifest.clone());
    let loaded_samples = load_samples(&manifest_abs)?;
    println!(
        "[voice-eval] Loaded {} samples from {}",
        loaded_samples.len(),
        manifest_abs.display()
    );

    let mut quality_reports = Vec::<QualityReport>::new();
    for target in &args.targets {
        println!(
            "[voice-eval] Evaluating target={} (file={})",
            target.id, target.file_name
        );
        quality_reports.push(evaluate_quality(&loaded_samples, target, args.use_gpu)?);
    }

    let report = VoiceEvalReport {
        generated_at: Utc::now().to_rfc3339(),
        manifest_path: manifest_abs.to_string_lossy().to_string(),
        use_gpu: args.use_gpu,
        qualities: quality_reports,
    };

    print_summary(&report);

    if let Some(output) = &args.output {
        save_report(output, &report)?;
        println!("[voice-eval] Wrote report: {}", output.display());
    }

    if let Some(baseline) = &args.baseline {
        enforce_regression_limit(&report, baseline, args.max_wer_delta, args.max_cer_delta)?;
        println!(
            "[voice-eval] Regression check passed (WER<=+{:.4}, CER<=+{:.4})",
            args.max_wer_delta, args.max_cer_delta
        );
    }

    Ok(())
}

fn main() {
    if let Err(err) = run() {
        eprintln!("voice_eval failed: {err}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_text_removes_punctuation() {
        let input = "Hello,   Whisper!!  今日は?";
        assert_eq!(normalize_text(input), "hello whisper 今日は");
    }

    #[test]
    fn levenshtein_distance_works() {
        let a = vec!["a", "b", "c"];
        let b = vec!["a", "x", "c", "d"];
        assert_eq!(levenshtein(&a, &b), 2);
    }

    #[test]
    fn resolves_popular_model_preset() {
        let models = resolve_model_token("popular").unwrap();
        let ids = models.into_iter().map(|m| m.id).collect::<Vec<_>>();
        assert_eq!(ids, vec!["medium", "large-v3-turbo", "large-v3"]);
    }

    #[test]
    fn resolves_explicit_model_file_name() {
        let models = resolve_model_token("ggml-large-v3-turbo-q5_0.bin").unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "large-v3-turbo-q5_0");
        assert_eq!(models[0].file_name, "ggml-large-v3-turbo-q5_0.bin");
    }
}
