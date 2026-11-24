//! Whisper model catalog, download, and on-disk status.
//!
//! The only network request Speakeasy ever makes is this download. It uses a
//! resumable `.part` file, reports progress to the UI, and verifies the
//! published SHA-256 before the model is moved into place.
//!
//! Users can pick a catalog entry or point Speakeasy at any GGML Whisper model
//! already on disk. Transcribing with a different model only costs one reload;
//! the pipeline notices the change before the next dictation.

use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use crossbeam_channel::{bounded, RecvTimeoutError};
use parking_lot::{Mutex, RwLock};
use serde::Serialize;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager};
use tracing::{info, warn};

use crate::error::{io_error, Error, Result};
use crate::events::{self, NotificationLevel};
use crate::AppState;

/// Larger models are markedly slower on a CPU, so the default stays at the
/// turbo build that most machines can handle.
pub const DEFAULT_MODEL_ID: &str = "large-v3-turbo-q5";

pub struct ModelSpec {
    pub id: &'static str,
    pub label: &'static str,
    pub note: &'static str,
    pub file: &'static str,
    pub url: &'static str,
    pub bytes: u64,
    pub sha256: &'static str,
}

/// The curated download list. Sizes and digests come from the Hugging Face
/// repository and are verified after every download.
pub const MODELS: &[ModelSpec] = &[
    ModelSpec {
        id: DEFAULT_MODEL_ID,
        label: "Large v3 Turbo · Q5",
        note: "The recommended balance of accuracy and download size.",
        file: "ggml-large-v3-turbo-q5_0.bin",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo-q5_0.bin",
        bytes: 574_041_195,
        sha256: "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2",
    },
    ModelSpec {
        id: "large-v3-turbo-q8",
        label: "Large v3 Turbo · Q8",
        note: "The most accurate option. The largest download; best with a GPU.",
        file: "ggml-large-v3-turbo-q8_0.bin",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo-q8_0.bin",
        bytes: 874_188_075,
        sha256: "317eb69c11673c9de1e1f0d459b253999804ec71ac4c23c17ecf5fbe24e259a1",
    },
    ModelSpec {
        id: "small-q5",
        label: "Small · Q5",
        note: "A fraction of the work of the large models. Good for older CPUs.",
        file: "ggml-small-q5_1.bin",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small-q5_1.bin",
        bytes: 190_085_487,
        sha256: "ae85e4a935d7a567bd102fe55afc16bb595bdb618e11b2fc7591bc08120411bb",
    },
    ModelSpec {
        id: "base-q5",
        label: "Base · Q5",
        note: "Quick on a CPU. Best suited to short, clear dictation.",
        file: "ggml-base-q5_1.bin",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base-q5_1.bin",
        bytes: 59_707_625,
        sha256: "422f1ae452ade6f30a004d7e5c6a43195e4433bc370bf23fac9cc591f01a8898",
    },
    ModelSpec {
        id: "tiny-q5",
        label: "Tiny · Q5",
        note: "The fastest option, with the lowest accuracy.",
        file: "ggml-tiny-q5_1.bin",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny-q5_1.bin",
        bytes: 32_152_673,
        sha256: "818710568da3ca15689e31a743197b520007872ff9576237bda97bd1b469c3d7",
    },
];

/// The identifier used for a model the user picked from their own disk.
pub const CUSTOM_MODEL_ID: &str = "custom";

const USER_AGENT: &str = concat!("Speakeasy/", env!("CARGO_PKG_VERSION"));
/// Emitting an event per 2 MiB keeps the UI smooth without flooding it.
const PROGRESS_STEP: u64 = 2 * 1024 * 1024;
const READ_BUFFER: usize = 256 * 1024;
const MAX_ATTEMPTS: u32 = 3;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
/// Per-read timeout, not a total-download limit: a slow-but-alive connection
/// can still finish a 574 MB model.
const READ_TIMEOUT: Duration = Duration::from_secs(60);
/// The reader sits on its own thread so a socket that never returns can be
/// abandoned. Without this a single dead CDN connection would hang the
/// download forever.
const STALL_TIMEOUT: Duration = Duration::from_secs(90);
const PROGRESS_LOG_STEP: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelPhase {
    Missing,
    Downloading,
    Verifying,
    Ready,
    Error,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelStatus {
    pub phase: ModelPhase,
    pub id: String,
    pub label: String,
    pub path: String,
    /// Bytes present on disk (including a partial download).
    pub received_bytes: u64,
    pub total_bytes: u64,
    pub custom: bool,
    pub error: Option<String>,
}

/// One row of the model picker.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelOption {
    pub id: String,
    pub label: String,
    pub note: String,
    pub bytes: u64,
    pub installed: bool,
    pub selected: bool,
    pub custom: bool,
}

#[derive(Clone)]
enum Selection {
    Catalog(&'static ModelSpec),
    Custom(PathBuf),
}

/// Owns the model directory, the active selection, and the download state
/// shown to the UI.
pub struct ModelManager {
    dir: PathBuf,
    selection: RwLock<Selection>,
    status: RwLock<ModelStatus>,
    downloading: AtomicBool,
    last_emit: Mutex<u64>,
}

impl ModelManager {
    pub fn new(data_dir: &Path, model_id: &str, custom_path: &str) -> Self {
        let dir = data_dir.join("models");
        let selection = resolve(model_id, custom_path)
            .unwrap_or_else(|| Selection::Catalog(&MODELS[0]));
        let status = inspect(&dir, &selection);
        Self {
            dir,
            selection: RwLock::new(selection),
            status: RwLock::new(status),
            downloading: AtomicBool::new(false),
            last_emit: Mutex::new(0),
        }
    }

    pub fn status(&self) -> ModelStatus {
        self.status.read().clone()
    }

    /// Absolute path of the selected model file. The pipeline compares this
    /// before each dictation and reloads Whisper when it changes.
    pub fn active_path(&self) -> PathBuf {
        match &*self.selection.read() {
            Selection::Catalog(spec) => self.dir.join(spec.file),
            Selection::Custom(path) => path.clone(),
        }
    }

    pub fn is_custom(&self) -> bool {
        matches!(&*self.selection.read(), Selection::Custom(_))
    }

    /// Re-checks the file unless a download owns the directory. This keeps the
    /// reported state honest if the user deletes the model while we are open.
    pub fn refresh(&self) {
        if self.downloading.load(Ordering::SeqCst) {
            return;
        }
        let selection = self.selection.read().clone();
        *self.status.write() = inspect(&self.dir, &selection);
    }

    pub fn is_ready(&self) -> bool {
        self.status.read().phase == ModelPhase::Ready
    }

    pub fn list(&self) -> Vec<ModelOption> {
        let selection = self.selection.read().clone();
        let mut options: Vec<ModelOption> = MODELS
            .iter()
            .map(|spec| ModelOption {
                id: spec.id.to_string(),
                label: spec.label.to_string(),
                note: spec.note.to_string(),
                bytes: spec.bytes,
                installed: model_file(&self.dir, spec)
                    .metadata()
                    .map(|meta| meta.len() == spec.bytes)
                    .unwrap_or(false),
                selected: matches!(&selection, Selection::Catalog(active) if active.id == spec.id),
                custom: false,
            })
            .collect();

        options.push(ModelOption {
            id: CUSTOM_MODEL_ID.to_string(),
            label: "Custom file".to_string(),
            note: "Use any GGML Whisper model already on this computer.".to_string(),
            bytes: 0,
            installed: matches!(&selection, Selection::Custom(path) if path.is_file()),
            selected: matches!(&selection, Selection::Custom(_)),
            custom: true,
        });

        options
    }

    /// Switches the active model. The caller persists the matching settings.
    pub fn select(&self, model_id: &str, custom_path: &str) -> Result<ModelStatus> {
        if self.downloading.load(Ordering::SeqCst) {
            return Err(Error::Model(
                "Wait for the current download to finish before switching models.".into(),
            ));
        }

        let selection = resolve(model_id, custom_path).ok_or_else(|| {
            Error::Model(format!("`{model_id}` is not a model Speakeasy knows about."))
        })?;

        if let Selection::Custom(path) = &selection {
            validate_custom(path)?;
        }

        let status = inspect(&self.dir, &selection);
        *self.selection.write() = selection;
        *self.status.write() = status.clone();
        info!(model = %status.id, path = %status.path, "model selection changed");
        Ok(status)
    }

    /// Starts the download on its own thread. Calling this while a download is
    /// already running is a no-op rather than an error.
    pub fn start_download(&self, app: AppHandle) -> Result<()> {
        let spec: &'static ModelSpec = match &*self.selection.read() {
            Selection::Catalog(spec) => *spec,
            Selection::Custom(_) => {
                return Err(Error::Model(
                    "Custom model files are not downloaded. Point Speakeasy at a file on disk instead."
                        .into(),
                ))
            }
        };

        if self.downloading.swap(true, Ordering::SeqCst) {
            return Ok(());
        }

        let dir = self.dir.clone();
        let thread_app = app.clone();
        let spawned = thread::Builder::new()
            .name("speakeasy-model".into())
            .spawn(move || {
                let state = thread_app.state::<AppState>();
                let manager = &state.model;
                manager.publish(&thread_app, ModelPhase::Downloading, current_len(manager), None);

                match download(&dir, spec, manager, &thread_app) {
                    Ok(()) => {
                        info!(model = %spec.id, "model is ready");
                        manager.publish(&thread_app, ModelPhase::Ready, spec.bytes, None);
                        events::notify(
                            &thread_app,
                            NotificationLevel::Info,
                            "The speech model is ready. Hold your hotkey and start dictating.",
                        );
                        // Load it in the background so the first dictation is
                        // not stuck paying the model load cost.
                        if let Err(err) = state.pipeline.warmup(thread_app.clone()) {
                            warn!(%err, "could not warm the transcription model");
                        }
                    }
                    Err(err) => {
                        warn!(%err, "model download failed");
                        manager.publish(
                            &thread_app,
                            ModelPhase::Error,
                            current_len(manager),
                            Some(err.to_string()),
                        );
                        events::notify(
                            &thread_app,
                            NotificationLevel::Error,
                            format!("The speech model download failed: {err}"),
                        );
                    }
                }

                manager.downloading.store(false, Ordering::SeqCst);
            });

        if let Err(err) = spawned {
            self.downloading.store(false, Ordering::SeqCst);
            return Err(Error::Model(format!(
                "could not start the model download: {err}"
            )));
        }

        Ok(())
    }

    fn publish(
        &self,
        app: &AppHandle,
        phase: ModelPhase,
        received: u64,
        error: Option<String>,
    ) {
        let selection = self.selection.read().clone();
        let status = match &selection {
            Selection::Catalog(spec) => ModelStatus {
                phase,
                id: spec.id.to_string(),
                label: spec.label.to_string(),
                path: model_file(&self.dir, spec).display().to_string(),
                received_bytes: received.min(spec.bytes),
                total_bytes: spec.bytes,
                custom: false,
                error,
            },
            Selection::Custom(path) => ModelStatus {
                phase,
                id: CUSTOM_MODEL_ID.to_string(),
                label: file_label(path),
                path: path.display().to_string(),
                received_bytes: 0,
                total_bytes: 0,
                custom: true,
                error,
            },
        };
        *self.status.write() = status.clone();

        let should_emit = {
            let mut last = self.last_emit.lock();
            if phase != ModelPhase::Downloading || received.saturating_sub(*last) >= PROGRESS_STEP {
                *last = received;
                true
            } else {
                false
            }
        };

        if should_emit {
            if let Err(err) = app.emit(events::EVENT_MODEL_STATUS, &status) {
                warn!(%err, "could not emit model status");
            }
        }
    }

    fn progress(&self, app: &AppHandle, received: u64) {
        self.publish(app, ModelPhase::Downloading, received, None);
    }

    fn verifying(&self, app: &AppHandle, total: u64) {
        self.publish(app, ModelPhase::Verifying, total, None);
    }
}

fn current_len(manager: &ModelManager) -> u64 {
    manager.status.read().received_bytes
}

fn resolve(model_id: &str, custom_path: &str) -> Option<Selection> {
    if model_id == CUSTOM_MODEL_ID {
        let cleaned = custom_path.trim().trim_matches('"').trim();
        if cleaned.is_empty() {
            return None;
        }
        return Some(Selection::Custom(PathBuf::from(cleaned)));
    }

    MODELS
        .iter()
        .find(|spec| spec.id == model_id)
        .map(Selection::Catalog)
}

fn model_file(dir: &Path, spec: &ModelSpec) -> PathBuf {
    dir.join(spec.file)
}

fn part_path(dir: &Path, spec: &ModelSpec) -> PathBuf {
    dir.join(format!("{}.part", spec.file))
}

fn file_label(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn inspect(dir: &Path, selection: &Selection) -> ModelStatus {
    match selection {
        Selection::Catalog(spec) => inspect_catalog(dir, spec),
        Selection::Custom(path) => inspect_custom(path),
    }
}

fn inspect_catalog(dir: &Path, spec: &ModelSpec) -> ModelStatus {
    let path = model_file(dir, spec);
    let display = path.display().to_string();

    match fs::metadata(&path) {
        Ok(meta) if meta.len() == spec.bytes => ModelStatus {
            phase: ModelPhase::Ready,
            id: spec.id.to_string(),
            label: spec.label.to_string(),
            path: display,
            received_bytes: meta.len(),
            total_bytes: spec.bytes,
            custom: false,
            error: None,
        },
        Ok(meta) => ModelStatus {
            phase: ModelPhase::Missing,
            id: spec.id.to_string(),
            label: spec.label.to_string(),
            path: display,
            received_bytes: meta.len(),
            total_bytes: spec.bytes,
            custom: false,
            error: Some(format!(
                "The model file is incomplete ({} of {} bytes). Download it again.",
                meta.len(),
                spec.bytes
            )),
        },
        Err(_) => {
            // A partial download from a previous session is resumable; show
            // its progress so the UI can offer "Resume" instead of starting
            // from zero.
            let partial = fs::metadata(part_path(dir, spec))
                .map(|meta| meta.len())
                .unwrap_or(0);
            ModelStatus {
                phase: ModelPhase::Missing,
                id: spec.id.to_string(),
                label: spec.label.to_string(),
                path: display,
                received_bytes: partial.min(spec.bytes),
                total_bytes: spec.bytes,
                custom: false,
                error: None,
            }
        }
    }
}

fn inspect_custom(path: &Path) -> ModelStatus {
    let display = path.display().to_string();
    let label = file_label(path);

    match fs::metadata(path) {
        Ok(meta) if meta.is_file() && meta.len() > 0 => ModelStatus {
            phase: ModelPhase::Ready,
            id: CUSTOM_MODEL_ID.to_string(),
            label,
            path: display,
            received_bytes: meta.len(),
            total_bytes: meta.len(),
            custom: true,
            error: None,
        },
        _ => ModelStatus {
            phase: ModelPhase::Missing,
            id: CUSTOM_MODEL_ID.to_string(),
            label,
            path: display,
            received_bytes: 0,
            total_bytes: 0,
            custom: true,
            error: Some("That model file is missing. Pick it again.".into()),
        },
    }
}

/// Rejects files that cannot be a GGML Whisper model before Whisper tries (and
/// fails with a much less helpful message).
pub fn validate_custom(path: &Path) -> Result<()> {
    let meta = fs::metadata(path).map_err(|_| {
        Error::Model(format!(
            "Could not open {}. Check that the file still exists.",
            path.display()
        ))
    })?;

    if !meta.is_file() {
        return Err(Error::Model(format!("{} is not a file.", path.display())));
    }

    if meta.len() < 1_000_000 {
        return Err(Error::Model(
            "That file is too small to be a Whisper model.".into(),
        ));
    }

    let mut file = fs::File::open(path).map_err(|err| io_error(path, err))?;
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic)
        .map_err(|err| io_error(path, err))?;

    if &magic != b"ggml" {
        return Err(Error::Model(
            "That file does not look like a GGML Whisper model (missing the `ggml` header).".into(),
        ));
    }

    Ok(())
}

/// Starts the download automatically for returning users and for debug builds,
/// where it saves a manual step every time the app data is wiped.
pub fn maybe_auto_download(app: &AppHandle) {
    let state = app.state::<AppState>();
    if state.model.is_ready() || state.model.is_custom() {
        return;
    }
    if cfg!(debug_assertions) || state.settings.get().onboarding_complete {
        match state.model.start_download(app.clone()) {
            Ok(()) => info!("started the model download automatically"),
            Err(err) => warn!(%err, "could not start the automatic model download"),
        }
    }
}

fn download(dir: &Path, spec: &'static ModelSpec, manager: &ModelManager, app: &AppHandle) -> Result<()> {
    fs::create_dir_all(dir).map_err(|err| io_error(dir, err))?;

    let final_path = model_file(dir, spec);
    if let Ok(meta) = fs::metadata(&final_path) {
        if meta.len() == spec.bytes {
            return Ok(());
        }
        // An incomplete file in the final location is unusable, but the `.part`
        // file may still hold a resumable download.
        fs::remove_file(&final_path).map_err(|err| io_error(&final_path, err))?;
    }

    let partial = part_path(dir, spec);
    if let Ok(meta) = fs::metadata(&partial) {
        if meta.len() > spec.bytes {
            fs::remove_file(&partial).map_err(|err| io_error(&partial, err))?;
        }
    }

    let mut last_error: Option<Error> = None;
    let mut received = part_len(&partial);

    for attempt in 1..=MAX_ATTEMPTS {
        match fetch(&partial, spec, manager, app) {
            Ok(bytes) => {
                received = bytes;
                last_error = None;
                break;
            }
            Err(err) => {
                received = part_len(&partial);
                warn!(attempt, %err, "model download attempt failed");
                last_error = Some(err);
                if attempt < MAX_ATTEMPTS {
                    thread::sleep(Duration::from_secs(attempt as u64));
                }
            }
        }
    }

    if let Some(err) = last_error {
        return Err(err);
    }

    if received != spec.bytes {
        return Err(Error::Model(format!(
            "download ended early ({received} of {} bytes); try again to resume",
            spec.bytes
        )));
    }

    manager.verifying(app, spec.bytes);
    let actual = sha256_file(&partial)?;
    if actual != spec.sha256 {
        // Do not keep bytes that cannot be trusted; a fresh download is cheaper
        // than debugging a corrupt model later.
        if let Err(err) = fs::remove_file(&partial) {
            warn!(%err, "could not remove the corrupt model download");
        }
        return Err(Error::Model(
            "the downloaded model did not match its checksum and was removed".into(),
        ));
    }
    info!(model = %spec.id, bytes = received, "model checksum verified");

    if final_path.exists() {
        fs::remove_file(&final_path).map_err(|err| io_error(&final_path, err))?;
    }
    fs::rename(&partial, &final_path).map_err(|err| io_error(&final_path, err))?;
    Ok(())
}

fn part_len(part_path: &Path) -> u64 {
    fs::metadata(part_path).map(|meta| meta.len()).unwrap_or(0)
}

/// Resumes from whatever is already in the `.part` file and returns the total
/// number of bytes now on disk.
fn fetch(
    part_path: &Path,
    spec: &'static ModelSpec,
    manager: &ModelManager,
    app: &AppHandle,
) -> Result<u64> {
    let mut existing = part_len(part_path);
    if existing > spec.bytes {
        fs::remove_file(part_path).map_err(|err| io_error(part_path, err))?;
        existing = 0;
    }

    let agent = ureq::AgentBuilder::new()
        .timeout_connect(CONNECT_TIMEOUT)
        .timeout_read(READ_TIMEOUT)
        .build();
    let mut request = agent.get(spec.url).set("User-Agent", USER_AGENT);
    if existing > 0 {
        request = request.set("Range", &format!("bytes={existing}-"));
    }

    let response = request
        .call()
        .map_err(|err| Error::Model(format!("download request failed: {err}")))?;
    let status = response.status();

    let mut file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(part_path)
        .map_err(|err| io_error(part_path, err))?;

    let mut received;
    if status == 206 {
        file.seek(SeekFrom::End(0))
            .map_err(|err| io_error(part_path, err))?;
        received = existing;
        info!(resuming_at = existing, "resuming the model download");
    } else if status == 200 {
        // The server ignored the range request; start over.
        file.set_len(0).map_err(|err| io_error(part_path, err))?;
        file.seek(SeekFrom::Start(0))
            .map_err(|err| io_error(part_path, err))?;
        received = 0;
    } else {
        return Err(Error::Model(format!(
            "the model server replied with HTTP {status}"
        )));
    }

    manager.progress(app, received);
    info!(status, received, "reading the model body");

    // Reading on its own thread is what makes a dead connection recoverable:
    // the writer below can time out, drop the channel, and request a fresh
    // range instead of waiting forever inside a blocked socket read.
    let mut reader = response.into_reader();
    let (chunk_tx, chunk_rx) = bounded::<std::result::Result<Vec<u8>, String>>(4);
    let reader_thread = thread::Builder::new()
        .name("speakeasy-model-read".into())
        .spawn(move || {
            let mut buffer = vec![0u8; READ_BUFFER];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => {
                        let _ = chunk_tx.send(Ok(Vec::new()));
                        break;
                    }
                    Ok(read) => {
                        if chunk_tx.send(Ok(buffer[..read].to_vec())).is_err() {
                            break;
                        }
                    }
                    Err(err) => {
                        let _ = chunk_tx.send(Err(err.to_string()));
                        break;
                    }
                }
            }
        })
        .map_err(|err| Error::Model(format!("could not start the download reader: {err}")))?;

    let mut next_log = received.saturating_add(PROGRESS_LOG_STEP);
    let mut first_chunk = true;
    loop {
        match chunk_rx.recv_timeout(STALL_TIMEOUT) {
            Ok(Ok(chunk)) if chunk.is_empty() => break,
            Ok(Ok(chunk)) => {
                file.write_all(&chunk)
                    .map_err(|err| io_error(part_path, err))?;
                received += chunk.len() as u64;
                if received > spec.bytes {
                    return Err(Error::Model(
                        "the model server sent more data than expected".into(),
                    ));
                }
                if first_chunk {
                    first_chunk = false;
                    info!(bytes = chunk.len(), "received the first model chunk");
                }
                if received >= next_log {
                    info!(received, "model download progress");
                    next_log = next_log.saturating_add(PROGRESS_LOG_STEP);
                }
                manager.progress(app, received);
            }
            Ok(Err(err)) => {
                return Err(Error::Model(format!("download interrupted: {err}")));
            }
            Err(RecvTimeoutError::Timeout) => {
                return Err(Error::Model(format!(
                    "the download stalled for {} seconds; retrying from the current position",
                    STALL_TIMEOUT.as_secs()
                )));
            }
            Err(RecvTimeoutError::Disconnected) => {
                return Err(Error::Model(
                    "the download reader stopped unexpectedly".into(),
                ));
            }
        }
    }

    let _ = reader_thread.join();
    file.sync_all().map_err(|err| io_error(part_path, err))?;
    Ok(received)
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path).map_err(|err| io_error(path, err))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; READ_BUFFER];
    loop {
        let read = file.read(&mut buffer).map_err(|err| io_error(path, err))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_metadata_is_consistent() {
        // A typo in a size or digest would make every download fail
        // verification, so the shape of each entry is asserted here.
        for spec in MODELS {
            assert_eq!(spec.sha256.len(), 64, "bad digest for {}", spec.id);
            assert!(spec.url.ends_with(spec.file), "bad URL for {}", spec.id);
            assert!(spec.bytes > 1_000_000, "bad size for {}", spec.id);
            assert!(!spec.label.is_empty() && !spec.note.is_empty());
        }
    }

    #[test]
    fn catalog_ids_and_files_are_unique() {
        let mut ids: Vec<_> = MODELS.iter().map(|spec| spec.id).collect();
        let mut files: Vec<_> = MODELS.iter().map(|spec| spec.file).collect();
        ids.sort_unstable();
        files.sort_unstable();
        ids.dedup();
        files.dedup();
        assert_eq!(ids.len(), MODELS.len());
        assert_eq!(files.len(), MODELS.len());
        assert!(MODELS.iter().any(|spec| spec.id == DEFAULT_MODEL_ID));
    }

    #[test]
    fn resolves_the_default_and_custom_selections() {
        assert!(matches!(
            resolve(DEFAULT_MODEL_ID, ""),
            Some(Selection::Catalog(spec)) if spec.id == DEFAULT_MODEL_ID
        ));
        assert!(matches!(
            resolve(CUSTOM_MODEL_ID, "  \"C:\\models\\voice.bin\"  "),
            Some(Selection::Custom(path)) if path == PathBuf::from("C:\\models\\voice.bin")
        ));
        assert!(resolve(CUSTOM_MODEL_ID, "   ").is_none());
        assert!(resolve("not-a-model", "").is_none());
    }

    #[test]
    fn rejects_files_that_are_not_models() {
        let dir = std::env::temp_dir().join(format!("speakeasy-model-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("temp dir");

        let short = dir.join("short.bin");
        fs::write(&short, b"ggml").expect("write");
        assert!(validate_custom(&short).is_err());

        let wrong = dir.join("wrong.bin");
        let mut payload = vec![0u8; 1_000_000];
        payload[..4].copy_from_slice(b"junk");
        fs::write(&wrong, &payload).expect("write");
        assert!(validate_custom(&wrong).is_err());

        let good = dir.join("good.bin");
        payload[..4].copy_from_slice(b"ggml");
        fs::write(&good, &payload).expect("write");
        assert!(validate_custom(&good).is_ok());

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn hashes_a_known_payload() {
        let dir = std::env::temp_dir().join(format!("speakeasy-model-test-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("payload.bin");
        // Empty input has a well-known SHA-256, so this checks the file
        // plumbing without embedding a made-up digest.
        fs::write(&path, b"").expect("write");
        assert_eq!(
            sha256_file(&path).expect("hash"),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        fs::remove_dir_all(&dir).ok();
    }
}
