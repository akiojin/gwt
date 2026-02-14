//! Voice input commands backed by Qwen3-ASR (Python runtime).

use hound::{SampleFormat, WavSpec, WavWriter};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use tracing::{error, warn};

const VOICE_PYTHON_ENV: &str = "GWT_VOICE_PYTHON";
const VOICE_SKIP_PROBE_ENV: &str = "GWT_VOICE_SKIP_QWEN_PROBE";
const VOICE_SAMPLE_RATE_FALLBACK: u32 = 16_000;
const VOICE_RUNTIME_VENV_DIR: &str = "voice-venv";
const VOICE_RUNTIME_PIP_DEPS: &[&str] = &["qwen-asr>=0.0.1"];
const QWEN_HELPER_SCRIPT: &str = include_str!("../python/qwen3_asr_runner.py");

static QWEN_RUNTIME_PROBE: Mutex<Option<Result<(), String>>> = Mutex::new(None);

fn with_panic_guard<T>(context: &str, f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(result) => result,
        Err(_) => {
            error!(
                category = "tauri",
                operation = context,
                "Unexpected panic while handling voice command"
            );
            Err(format!("Unexpected error while {context}"))
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceCapability {
    pub available: bool,
    pub reason: Option<String>,
    pub gpu_required: bool,
    pub gpu_available: bool,
    pub quality: String,
    pub model_name: String,
    pub model_ready: bool,
    pub model_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceModelPreparationResult {
    pub quality: String,
    pub model_name: String,
    pub model_path: String,
    pub ready: bool,
    pub downloaded: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceTranscriptionResult {
    pub transcript: String,
    pub quality: String,
    pub model_name: String,
    pub sample_rate: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceRuntimeSetupResult {
    pub ready: bool,
    pub installed: bool,
    pub python_path: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceTranscriptionRequest {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    pub language: String,
    pub quality: String,
    pub gpu_available: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct QwenRunnerResponse {
    ok: bool,
    transcript: Option<String>,
    error: Option<String>,
}

fn normalize_quality(raw: &str) -> String {
    match raw.trim().to_lowercase().as_str() {
        "fast" => "fast".to_string(),
        "accurate" => "accurate".to_string(),
        _ => "balanced".to_string(),
    }
}

fn normalize_language(raw: &str) -> String {
    match raw.trim().to_lowercase().as_str() {
        "ja" => "ja".to_string(),
        "en" => "en".to_string(),
        _ => "auto".to_string(),
    }
}

fn qwen_model_id_for_quality(quality: &str) -> &'static str {
    match quality {
        "fast" => "Qwen/Qwen3-ASR-0.6B",
        "accurate" => "Qwen/Qwen3-ASR-1.7B",
        _ => "Qwen/Qwen3-ASR-1.7B",
    }
}

fn is_env_truthy(name: &str) -> bool {
    std::env::var(name)
        .map(|value| {
            matches!(
                value.trim().to_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

fn voice_runtime_venv_dir() -> Result<PathBuf, String> {
    Ok(gwt_runtime_dir()?.join(VOICE_RUNTIME_VENV_DIR))
}

fn voice_runtime_python_path(venv_dir: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        return venv_dir.join("Scripts").join("python.exe");
    }

    #[cfg(not(windows))]
    {
        venv_dir.join("bin").join("python3")
    }
}

fn find_python_override() -> Result<Option<PathBuf>, String> {
    if let Ok(override_path) = std::env::var(VOICE_PYTHON_ENV) {
        let trimmed = override_path.trim();
        if trimmed.is_empty() {
            return Err(format!("{VOICE_PYTHON_ENV} is set but empty"));
        }
        let path = PathBuf::from(trimmed);
        if path.is_file() {
            return Ok(Some(path));
        }
        return Err(format!(
            "Python runtime not found at {trimmed} (from {VOICE_PYTHON_ENV})"
        ));
    }
    Ok(None)
}

fn find_system_python_binary() -> Result<PathBuf, String> {
    for candidate in ["python3.12", "python3.11", "python3", "python"] {
        if let Ok(path) = which::which(candidate) {
            return Ok(path);
        }
    }

    Err("Python runtime not found (checked python3.12/python3.11/python3/python)".to_string())
}

fn find_managed_python_binary() -> Result<PathBuf, String> {
    let venv_dir = voice_runtime_venv_dir()?;
    let python = voice_runtime_python_path(&venv_dir);
    if python.is_file() {
        Ok(python)
    } else {
        Err(format!(
            "Managed voice runtime not found at {}",
            python.to_string_lossy()
        ))
    }
}

fn find_python_binary() -> Result<PathBuf, String> {
    if let Some(path) = find_python_override()? {
        return Ok(path);
    }
    if let Ok(path) = find_managed_python_binary() {
        return Ok(path);
    }
    find_system_python_binary()
}

fn find_bootstrap_python_binary() -> Result<PathBuf, String> {
    if let Some(path) = find_python_override()? {
        return Ok(path);
    }
    find_system_python_binary()
}

fn gwt_runtime_dir() -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or_else(|| "Failed to resolve home directory".to_string())?;
    Ok(home.join(".gwt").join("runtime"))
}

fn ensure_qwen_runner_script() -> Result<PathBuf, String> {
    let runtime_dir = gwt_runtime_dir()?;
    fs::create_dir_all(&runtime_dir)
        .map_err(|e| format!("Failed to create runtime directory: {e}"))?;

    let script_path = runtime_dir.join("qwen3_asr_runner.py");

    let needs_write = match fs::read_to_string(&script_path) {
        Ok(existing) => existing != QWEN_HELPER_SCRIPT,
        Err(_) => true,
    };

    if needs_write {
        let mut file = fs::File::create(&script_path)
            .map_err(|e| format!("Failed to create qwen helper script: {e}"))?;
        file.write_all(QWEN_HELPER_SCRIPT.as_bytes())
            .map_err(|e| format!("Failed to write qwen helper script: {e}"))?;
        file.flush()
            .map_err(|e| format!("Failed to flush qwen helper script: {e}"))?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perm = fs::metadata(&script_path)
                .map_err(|e| format!("Failed to stat qwen helper script: {e}"))?
                .permissions();
            perm.set_mode(0o700);
            let _ = fs::set_permissions(&script_path, perm);
        }
    }

    Ok(script_path)
}

fn run_qwen_runner_with_python(
    python: &Path,
    action: &str,
    model_id: Option<&str>,
    audio_path: Option<&Path>,
    language: Option<&str>,
) -> Result<QwenRunnerResponse, String> {
    let script = ensure_qwen_runner_script()?;

    let mut cmd = Command::new(python);
    cmd.arg(&script).arg("--action").arg(action);

    if let Some(model_id) = model_id {
        cmd.arg("--model-id").arg(model_id);
    }
    if let Some(audio_path) = audio_path {
        cmd.arg("--audio-path").arg(audio_path);
    }
    if let Some(language) = language {
        cmd.arg("--language").arg(language);
    }

    let output = cmd
        .output()
        .map_err(|e| format!("Failed to run qwen helper script: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        return Err(format!(
            "Qwen helper failed (status={}): {}{}",
            output.status,
            if stderr.is_empty() {
                "<no stderr>"
            } else {
                &stderr
            },
            if stdout.is_empty() {
                "".to_string()
            } else {
                format!("; stdout={stdout}")
            }
        ));
    }

    let stdout = String::from_utf8(output.stdout)
        .map_err(|e| format!("Qwen helper returned non UTF-8 stdout: {e}"))?;
    let parsed: QwenRunnerResponse = serde_json::from_str(stdout.trim())
        .map_err(|e| format!("Failed to parse qwen helper response: {e}"))?;

    if parsed.ok {
        Ok(parsed)
    } else {
        Err(parsed
            .error
            .unwrap_or_else(|| "Qwen helper returned failure without error".to_string()))
    }
}

fn run_qwen_runner(
    action: &str,
    model_id: Option<&str>,
    audio_path: Option<&Path>,
    language: Option<&str>,
) -> Result<QwenRunnerResponse, String> {
    let python = find_python_binary()?;
    run_qwen_runner_with_python(&python, action, model_id, audio_path, language)
}

fn clear_runtime_probe_cache() {
    if let Ok(mut guard) = QWEN_RUNTIME_PROBE.lock() {
        *guard = None;
    }
}

fn probe_qwen_runtime() -> Result<(), String> {
    if is_env_truthy(VOICE_SKIP_PROBE_ENV) {
        return Ok(());
    }
    let _ = run_qwen_runner("probe", None, None, None)?;
    Ok(())
}

fn probe_qwen_runtime_cached() -> Result<(), String> {
    let mut guard = QWEN_RUNTIME_PROBE
        .lock()
        .map_err(|_| "Failed to lock voice runtime probe cache".to_string())?;
    if let Some(result) = guard.as_ref() {
        return result.clone();
    }
    let result = probe_qwen_runtime();
    *guard = Some(result.clone());
    result
}

fn run_command_with_output(mut cmd: Command, context: &str) -> Result<(), String> {
    let output = cmd
        .output()
        .map_err(|e| format!("{context}: failed to start command: {e}"))?;
    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Err(format!(
        "{context}: command failed (status={}): {}{}",
        output.status,
        if stderr.is_empty() {
            "<no stderr>"
        } else {
            &stderr
        },
        if stdout.is_empty() {
            "".to_string()
        } else {
            format!("; stdout={stdout}")
        }
    ))
}

fn ensure_managed_voice_runtime_sync() -> Result<VoiceRuntimeSetupResult, String> {
    if probe_qwen_runtime_cached().is_ok() {
        let python = find_python_binary()?;
        return Ok(VoiceRuntimeSetupResult {
            ready: true,
            installed: false,
            python_path: python.to_string_lossy().to_string(),
        });
    }

    let runtime_dir = gwt_runtime_dir()?;
    fs::create_dir_all(&runtime_dir)
        .map_err(|e| format!("Failed to create runtime directory: {e}"))?;

    let venv_dir = voice_runtime_venv_dir()?;
    let managed_python = voice_runtime_python_path(&venv_dir);
    let mut installed = false;

    if !managed_python.is_file() {
        let bootstrap_python = find_bootstrap_python_binary()?;
        let mut cmd = Command::new(bootstrap_python);
        cmd.arg("-m").arg("venv").arg(&venv_dir);
        run_command_with_output(cmd, "Failed to create voice runtime virtual environment")?;
        installed = true;
    }

    let mut pip_upgrade = Command::new(&managed_python);
    pip_upgrade
        .arg("-m")
        .arg("pip")
        .arg("install")
        .arg("--upgrade")
        .arg("pip")
        .env("PIP_DISABLE_PIP_VERSION_CHECK", "1");
    run_command_with_output(pip_upgrade, "Failed to update pip for voice runtime")?;

    let probe_result = run_qwen_runner_with_python(&managed_python, "probe", None, None, None);
    if probe_result.is_err() {
        let mut install = Command::new(&managed_python);
        install.arg("-m").arg("pip").arg("install").arg("--upgrade");
        for dep in VOICE_RUNTIME_PIP_DEPS {
            install.arg(dep);
        }
        install.env("PIP_DISABLE_PIP_VERSION_CHECK", "1");
        run_command_with_output(install, "Failed to install voice runtime dependencies")?;
        installed = true;
    }

    let _ = run_qwen_runner_with_python(&managed_python, "probe", None, None, None)?;
    clear_runtime_probe_cache();
    let _ = probe_qwen_runtime_cached()?;

    Ok(VoiceRuntimeSetupResult {
        ready: true,
        installed,
        python_path: managed_python.to_string_lossy().to_string(),
    })
}

fn huggingface_hub_dir() -> Result<PathBuf, String> {
    if let Ok(hf_home) = std::env::var("HF_HOME") {
        let trimmed = hf_home.trim();
        if !trimmed.is_empty() {
            return Ok(PathBuf::from(trimmed).join("hub"));
        }
    }

    if let Ok(xdg_cache) = std::env::var("XDG_CACHE_HOME") {
        let trimmed = xdg_cache.trim();
        if !trimmed.is_empty() {
            return Ok(PathBuf::from(trimmed).join("huggingface").join("hub"));
        }
    }

    let home = dirs::home_dir().ok_or_else(|| "Failed to resolve home directory".to_string())?;
    Ok(home.join(".cache").join("huggingface").join("hub"))
}

fn model_cache_dir(model_id: &str) -> Result<PathBuf, String> {
    let encoded = format!("models--{}", model_id.replace('/', "--"));
    Ok(huggingface_hub_dir()?.join(encoded))
}

fn snapshot_has_single_file_model(snapshot_dir: &Path) -> bool {
    snapshot_dir.join("model.safetensors").is_file()
}

fn snapshot_has_complete_sharded_model(snapshot_dir: &Path) -> bool {
    let index_path = snapshot_dir.join("model.safetensors.index.json");
    if !index_path.is_file() {
        return false;
    }

    let raw = match fs::read_to_string(index_path) {
        Ok(raw) => raw,
        Err(_) => return false,
    };

    let parsed: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(value) => value,
        Err(_) => return false,
    };

    let Some(weight_map) = parsed.get("weight_map").and_then(|v| v.as_object()) else {
        return false;
    };

    let mut required_files = HashSet::<String>::new();
    for value in weight_map.values() {
        if let Some(file_name) = value.as_str() {
            required_files.insert(file_name.to_string());
        }
    }

    if required_files.is_empty() {
        return false;
    }

    required_files
        .iter()
        .all(|file_name| snapshot_dir.join(file_name).is_file())
}

fn model_ready_in_cache(model_id: &str) -> bool {
    let Ok(cache_dir) = model_cache_dir(model_id) else {
        return false;
    };
    let snapshots_dir = cache_dir.join("snapshots");
    let entries = match fs::read_dir(&snapshots_dir) {
        Ok(entries) => entries,
        Err(_) => return false,
    };

    for entry in entries.flatten() {
        let snapshot_dir = entry.path();
        if !snapshot_dir.is_dir() {
            continue;
        }
        if !snapshot_dir.join("config.json").is_file() {
            continue;
        }
        if snapshot_has_single_file_model(&snapshot_dir)
            || snapshot_has_complete_sharded_model(&snapshot_dir)
        {
            return true;
        }
    }

    false
}

fn write_temp_wav(samples: &[f32], sample_rate: u32) -> Result<PathBuf, String> {
    let effective_sample_rate = if sample_rate == 0 {
        VOICE_SAMPLE_RATE_FALLBACK
    } else {
        sample_rate
    };

    let file_path = std::env::temp_dir().join(format!("gwt-voice-{}.wav", uuid::Uuid::new_v4()));

    let spec = WavSpec {
        channels: 1,
        sample_rate: effective_sample_rate,
        bits_per_sample: 16,
        sample_format: SampleFormat::Int,
    };

    let mut writer = WavWriter::create(&file_path, spec)
        .map_err(|e| format!("Failed to create temporary WAV file: {e}"))?;

    for sample in samples {
        let value = (sample.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16;
        writer
            .write_sample(value)
            .map_err(|e| format!("Failed to write temporary WAV sample: {e}"))?;
    }

    writer
        .finalize()
        .map_err(|e| format!("Failed to finalize temporary WAV file: {e}"))?;

    Ok(file_path)
}

fn prepare_model_sync(quality: &str) -> Result<(String, PathBuf, bool), String> {
    let _ = ensure_managed_voice_runtime_sync()?;

    let model_id = qwen_model_id_for_quality(quality);
    let was_ready = model_ready_in_cache(model_id);

    if !was_ready {
        let _ = run_qwen_runner("prepare", Some(model_id), None, None)?;
    }

    let cache_path = model_cache_dir(model_id)?;
    let ready = model_ready_in_cache(model_id);
    let downloaded = !was_ready && ready;

    Ok((model_id.to_string(), cache_path, downloaded))
}

fn transcribe_sync(input: VoiceTranscriptionRequest) -> Result<VoiceTranscriptionResult, String> {
    if !input.gpu_available {
        return Err("Voice input is disabled because no GPU acceleration is available".to_string());
    }

    probe_qwen_runtime_cached()?;

    let quality = normalize_quality(&input.quality);
    let model_id = qwen_model_id_for_quality(&quality);

    if input.samples.is_empty() {
        return Ok(VoiceTranscriptionResult {
            transcript: String::new(),
            quality,
            model_name: model_id.to_string(),
            sample_rate: if input.sample_rate == 0 {
                VOICE_SAMPLE_RATE_FALLBACK
            } else {
                input.sample_rate
            },
        });
    }

    let temp_wav = write_temp_wav(&input.samples, input.sample_rate)?;
    let language = normalize_language(&input.language);

    let runner_result = run_qwen_runner(
        "transcribe",
        Some(model_id),
        Some(&temp_wav),
        Some(&language),
    );

    let _ = fs::remove_file(&temp_wav);

    let response = runner_result?;
    Ok(VoiceTranscriptionResult {
        transcript: response.transcript.unwrap_or_default(),
        quality,
        model_name: model_id.to_string(),
        sample_rate: if input.sample_rate == 0 {
            VOICE_SAMPLE_RATE_FALLBACK
        } else {
            input.sample_rate
        },
    })
}

#[tauri::command]
pub fn get_voice_capability(
    gpu_available: bool,
    quality: String,
) -> Result<VoiceCapability, String> {
    with_panic_guard("checking voice capability", || {
        let quality = normalize_quality(&quality);
        let model_name = qwen_model_id_for_quality(&quality).to_string();
        let model_path = model_cache_dir(&model_name)?;
        let model_ready = model_ready_in_cache(&model_name);

        let runtime_result = if gpu_available {
            probe_qwen_runtime_cached().err()
        } else {
            None
        };

        let (available, reason) = if !gpu_available {
            (
                false,
                Some("Voice input requires GPU acceleration in this build".to_string()),
            )
        } else if let Some(runtime_error) = runtime_result {
            (
                false,
                Some(format!("Voice runtime is unavailable: {runtime_error}")),
            )
        } else {
            (true, None)
        };

        Ok(VoiceCapability {
            available,
            reason,
            gpu_required: true,
            gpu_available,
            quality,
            model_name,
            model_ready,
            model_path: model_path.to_string_lossy().to_string(),
        })
    })
}

#[tauri::command]
pub async fn prepare_voice_model(
    gpu_available: bool,
    quality: String,
) -> Result<VoiceModelPreparationResult, String> {
    if !gpu_available {
        return Err("Voice input is disabled because no GPU acceleration is available".to_string());
    }

    let quality = normalize_quality(&quality);
    let quality_for_task = quality.clone();

    let (model_name, model_path, downloaded) =
        tokio::task::spawn_blocking(move || prepare_model_sync(&quality_for_task))
            .await
            .map_err(|e| format!("Voice model preparation task failed: {e}"))??;

    Ok(VoiceModelPreparationResult {
        quality,
        model_name,
        model_path: model_path.to_string_lossy().to_string(),
        ready: true,
        downloaded,
    })
}

#[tauri::command]
pub async fn ensure_voice_runtime() -> Result<VoiceRuntimeSetupResult, String> {
    tokio::task::spawn_blocking(ensure_managed_voice_runtime_sync)
        .await
        .map_err(|e| format!("Voice runtime setup task failed: {e}"))?
}

#[tauri::command]
pub async fn transcribe_voice_audio(
    input: VoiceTranscriptionRequest,
) -> Result<VoiceTranscriptionResult, String> {
    let quality = normalize_quality(&input.quality);
    if !input.gpu_available {
        return Err("Voice input is disabled because no GPU acceleration is available".to_string());
    }
    if input.samples.is_empty() {
        return Ok(VoiceTranscriptionResult {
            transcript: String::new(),
            quality: quality.clone(),
            model_name: qwen_model_id_for_quality(&quality).to_string(),
            sample_rate: if input.sample_rate == 0 {
                VOICE_SAMPLE_RATE_FALLBACK
            } else {
                input.sample_rate
            },
        });
    }

    let handle = tokio::task::spawn_blocking(move || transcribe_sync(input));
    match handle.await {
        Ok(result) => result,
        Err(e) => {
            warn!(
                category = "tauri",
                operation = "voice_transcription",
                error = %e,
                "Voice transcription background task failed"
            );
            Err(format!("Voice transcription task failed: {e}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quality_mapping_defaults_to_balanced() {
        assert_eq!(normalize_quality(""), "balanced");
        assert_eq!(normalize_quality("unknown"), "balanced");
        assert_eq!(normalize_quality("fast"), "fast");
        assert_eq!(normalize_quality("accurate"), "accurate");
    }

    #[test]
    fn quality_maps_to_qwen_models() {
        assert_eq!(qwen_model_id_for_quality("fast"), "Qwen/Qwen3-ASR-0.6B");
        assert_eq!(qwen_model_id_for_quality("balanced"), "Qwen/Qwen3-ASR-1.7B");
        assert_eq!(qwen_model_id_for_quality("accurate"), "Qwen/Qwen3-ASR-1.7B");
    }

    #[test]
    fn language_mapping_supports_auto_and_fixed() {
        assert_eq!(normalize_language("auto"), "auto");
        assert_eq!(normalize_language("ja"), "ja");
        assert_eq!(normalize_language("en"), "en");
        assert_eq!(normalize_language("xx"), "auto");
    }

    #[test]
    fn capability_reports_unavailable_without_gpu() {
        let capability = get_voice_capability(false, "balanced".to_string()).unwrap();
        assert!(!capability.available);
        assert!(capability.reason.is_some());
    }

    #[test]
    fn model_cache_ready_detects_single_file_model() {
        let _lock = crate::commands::ENV_LOCK.lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        std::env::set_var("HF_HOME", temp.path());

        let snapshot = temp
            .path()
            .join("hub")
            .join("models--Qwen--Qwen3-ASR-0.6B")
            .join("snapshots")
            .join("abc");
        fs::create_dir_all(&snapshot).unwrap();
        fs::write(snapshot.join("config.json"), "{}").unwrap();
        fs::write(snapshot.join("model.safetensors"), "weights").unwrap();

        assert!(model_ready_in_cache("Qwen/Qwen3-ASR-0.6B"));

        std::env::remove_var("HF_HOME");
    }

    #[test]
    fn sharded_model_requires_all_files() {
        let _lock = crate::commands::ENV_LOCK.lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        std::env::set_var("HF_HOME", temp.path());

        let snapshot = temp
            .path()
            .join("hub")
            .join("models--Qwen--Qwen3-ASR-1.7B")
            .join("snapshots")
            .join("abc");
        fs::create_dir_all(&snapshot).unwrap();
        fs::write(snapshot.join("config.json"), "{}").unwrap();
        fs::write(
            snapshot.join("model.safetensors.index.json"),
            r#"{"weight_map":{"a":"model-00001-of-00002.safetensors","b":"model-00002-of-00002.safetensors"}}"#,
        )
        .unwrap();
        fs::write(snapshot.join("model-00001-of-00002.safetensors"), "part1").unwrap();

        assert!(!model_ready_in_cache("Qwen/Qwen3-ASR-1.7B"));

        fs::write(snapshot.join("model-00002-of-00002.safetensors"), "part2").unwrap();
        assert!(model_ready_in_cache("Qwen/Qwen3-ASR-1.7B"));

        std::env::remove_var("HF_HOME");
    }
}
