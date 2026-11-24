//! Whisper integration.
//!
//! The model is loaded lazily on the pipeline thread and then kept resident,
//! so only the first dictation after launch pays the load cost. The GPU path is
//! used when the build has a GPU backend; if that fails (driver trouble, no
//! compatible device) we quietly retry on the CPU.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use tracing::{info, warn};
use whisper_rs::{
    FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperError,
};

use crate::error::{Error, Result};

/// Whisper is fastest with a handful of threads; beyond that, memory contention
/// starts to cost more than the extra parallelism buys.
const MAX_THREADS: usize = 8;

pub struct Transcription {
    pub text: String,
    pub inference: Duration,
    /// Non-zero only on the clip that loaded the model.
    pub load: Duration,
}

pub struct Transcriber {
    model_path: PathBuf,
    context: Option<WhisperContext>,
    threads: i32,
}

impl Transcriber {
    pub fn new(model_path: PathBuf) -> Self {
        let threads = std::thread::available_parallelism()
            .map(|count| count.get())
            .unwrap_or(4)
            .min(MAX_THREADS) as i32;

        Self {
            model_path,
            context: None,
            threads,
        }
    }

    pub fn available(&self) -> bool {
        self.model_path.is_file()
    }

    /// Points the transcriber at another model file. The resident context is
    /// dropped so the next dictation loads the new one; switching to the same
    /// path is free.
    pub fn switch_to(&mut self, model_path: PathBuf) {
        if model_path != self.model_path {
            info!(path = %model_path.display(), "switching whisper model");
            self.model_path = model_path;
            self.context = None;
        }
    }

    /// Loads the model now rather than on the first dictation. Errors are
    /// returned so the caller can log them, but a warm-up failure is not fatal:
    /// the next dictation retries the load.
    pub fn preload(&mut self) -> Result<()> {
        self.ensure_loaded().map(|_| ())
    }

    fn ensure_loaded(&mut self) -> Result<&WhisperContext> {
        if self.context.is_none() {
            let started = Instant::now();
            let force_cpu = force_cpu();

            let mut gpu = WhisperContextParameters::default();
            gpu.use_gpu(!force_cpu);
            let context = match WhisperContext::new_with_params(&self.model_path, gpu) {
                Ok(context) => context,
                Err(err) if !force_cpu && gpu_backend_compiled() => {
                    warn!(%err, "GPU model load failed, falling back to CPU");
                    let mut cpu = WhisperContextParameters::default();
                    cpu.use_gpu(false);
                    WhisperContext::new_with_params(&self.model_path, cpu)
                        .map_err(|err| load_error(&self.model_path, err))?
                }
                Err(err) => return Err(load_error(&self.model_path, err)),
            };

            info!(
                load_ms = started.elapsed().as_millis() as u64,
                threads = self.threads,
                gpu_requested = !force_cpu,
                "whisper model loaded"
            );
            self.context = Some(context);
        }

        self.context
            .as_ref()
            .ok_or_else(|| Error::Transcription("the whisper model is not loaded".into()))
    }

    /// Transcribes 16 kHz mono f32 samples. `language` is `auto` or an
    /// ISO-639-1 code.
    pub fn transcribe(&mut self, samples: &[f32], language: &str) -> Result<Transcription> {
        if samples.is_empty() {
            return Err(Error::Transcription("there was no audio to transcribe".into()));
        }

        let started = Instant::now();
        let load_before = self.context.is_some();
        let context = self.ensure_loaded()?;
        let load = if load_before {
            Duration::ZERO
        } else {
            started.elapsed()
        };

        let mut state = context
            .create_state()
            .map_err(|err| Error::Transcription(format!("could not prepare whisper: {err}")))?;

        // Greedy with a single candidate is the fastest usable strategy and is
        // accurate enough for short dictation. Beam search can more than double
        // the latency, which would blow the release-to-paste budget.
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_n_threads(self.threads);
        params.set_language(if language == "auto" { None } else { Some(language) });
        params.set_translate(false);
        params.set_no_context(true);
        params.set_single_segment(false);
        params.set_suppress_blank(true);
        params.set_suppress_nst(true);
        params.set_temperature(0.0);
        params.set_temperature_inc(0.0);
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);

        state
            .full(params, samples)
            .map_err(|err| Error::Transcription(format!("whisper could not run: {err}")))?;

        let inference = started.elapsed().saturating_sub(load);
        let count = state.full_n_segments();
        let mut text = String::new();
        for index in 0..count {
            if let Some(segment) = state.get_segment(index) {
                match segment.to_str_lossy() {
                    Ok(part) => text.push_str(&part),
                    Err(err) => warn!(%err, "could not read a whisper segment"),
                }
            }
        }

        Ok(Transcription {
            text,
            inference,
            load,
        })
    }
}

/// Mirrors the whisper-rs features that put a GPU backend into the binary: Metal
/// on macOS, Vulkan in every Windows build, and the opt-in features elsewhere.
fn gpu_backend_compiled() -> bool {
    cfg!(target_os = "macos")
        || cfg!(target_os = "windows")
        || cfg!(feature = "vulkan")
        || cfg!(feature = "cuda")
}

/// `SPEAKEASY_FORCE_CPU=1` skips the GPU so the CPU path can be measured on a
/// machine that has a working device. It also documents itself in the logs.
fn force_cpu() -> bool {
    std::env::var_os("SPEAKEASY_FORCE_CPU").is_some_and(|value| !value.is_empty())
}

fn load_error(path: &std::path::Path, err: WhisperError) -> Error {
    Error::Transcription(format!(
        "could not load {}: {err}",
        path.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Opt-in smoke test against the real model and a real recording. Run it in
    /// release mode to measure the latency the shipped app will see:
    ///
    /// ```text
    /// SPEAKEASY_MODEL=... SPEAKEASY_SMOKE_WAV=... \
    ///   cargo test --release --lib smoke -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "needs the downloaded model and a WAV file"]
    fn smoke_transcribes_a_wav() {
        let (Ok(model), Ok(wav)) = (
            std::env::var("SPEAKEASY_MODEL"),
            std::env::var("SPEAKEASY_SMOKE_WAV"),
        ) else {
            return;
        };

        let (samples, rate) = crate::audio::wav::read_mono(std::path::Path::new(&wav))
            .expect("read the smoke WAV");
        let samples = crate::audio::resample_to_16k(&samples, rate);
        let audio_ms = samples.len() as u64 / 16;

        let mut transcriber = Transcriber::new(PathBuf::from(model));
        let started = Instant::now();
        let transcription = transcriber
            .transcribe(&samples, "auto")
            .expect("transcribe the smoke WAV");

        println!(
            "threads={} audio_ms={audio_ms} inference_ms={} total_ms={} text={:?}",
            transcriber.threads,
            transcription.inference.as_millis(),
            started.elapsed().as_millis(),
            transcription.text,
        );
        assert!(!transcription.text.trim().is_empty(), "no speech recognized");
    }
}
