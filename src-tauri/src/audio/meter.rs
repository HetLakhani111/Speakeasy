//! Live spectrum analysis for the recording overlay.
//!
//! A 1024-point real FFT is taken from the most recent input samples roughly
//! sixty times per second. The magnitudes are folded into logarithmically
//! spaced bands and smoothed so the visualizer looks alive rather than jittery.

use std::sync::Arc;

use realfft::num_complex::Complex;
use realfft::{RealFftPlanner, RealToComplex};
use serde::Serialize;

use crate::error::{Error, Result};

pub const FFT_SIZE: usize = 1024;
pub const BAND_COUNT: usize = 64;

const MIN_FREQUENCY: f32 = 50.0;
const MAX_FREQUENCY: f32 = 8_000.0;
const DB_FLOOR: f32 = -70.0;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioLevels {
    /// Overall loudness, 0.0 (silence) to 1.0 (clipping).
    pub level: f32,
    /// Smoothed, dB-normalized magnitudes for each log-spaced band.
    pub bins: Vec<f32>,
}

pub struct Meter {
    fft: Arc<dyn RealToComplex<f32>>,
    window: Vec<f32>,
    window_gain: f32,
    input: Vec<f32>,
    output: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
    bands: Vec<(usize, usize)>,
    smoothed: Vec<f32>,
    smoothed_level: f32,
}

impl Meter {
    pub fn new(sample_rate: u32) -> Result<Self> {
        if sample_rate == 0 {
            return Err(Error::Audio("the input device reported a zero sample rate".into()));
        }

        let mut planner = RealFftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(FFT_SIZE);
        let input = fft.make_input_vec();
        let output = fft.make_output_vec();
        let scratch = fft.make_scratch_vec();

        let window = (0..FFT_SIZE)
            .map(|index| {
                let phase = 2.0 * std::f32::consts::PI * index as f32 / FFT_SIZE as f32;
                0.5 - 0.5 * phase.cos()
            })
            .collect::<Vec<_>>();
        let window_gain = (window.iter().sum::<f32>() * 0.5).max(1.0);

        Ok(Self {
            fft,
            window,
            window_gain,
            input,
            output,
            scratch,
            bands: band_edges(sample_rate),
            smoothed: vec![0.0; BAND_COUNT],
            smoothed_level: 0.0,
        })
    }

    /// Analyzes the newest samples (up to the last 1024 are used) and returns
    /// display-ready values.
    pub fn analyze(&mut self, samples: &[f32]) -> Result<AudioLevels> {
        if samples.is_empty() {
            return Ok(self.decay());
        }

        let start = samples.len().saturating_sub(FFT_SIZE);
        let slice = &samples[start..];

        self.input.fill(0.0);
        for (index, sample) in slice.iter().enumerate() {
            let value = if sample.is_finite() { *sample } else { 0.0 };
            self.input[index] = value * self.window[index];
        }

        self.fft
            .process_with_scratch(&mut self.input, &mut self.output, &mut self.scratch)
            .map_err(|err| Error::Audio(format!("spectrum analysis failed: {err}")))?;

        for (band, (first, last)) in self.bands.iter().enumerate() {            let mut peak = 0.0_f32;
            for bin in *first..*last {
                let value = self.output.get(bin).copied().unwrap_or_default();
                peak = peak.max(value.norm());
            }

            // Convert FFT magnitude to an approximate signal amplitude.
            let amplitude = peak * 2.0 / self.window_gain;
            let decibels = 20.0 * (amplitude + 1e-9).log10();
            let normalized = ((decibels - DB_FLOOR) / -DB_FLOOR).clamp(0.0, 1.0);

            // Instant attack, gentle release.
            let previous = self.smoothed[band];
            self.smoothed[band] = normalized.max(previous * 0.72);
        }

        let mean_square = slice.iter().map(|s| s * s).sum::<f32>() / slice.len() as f32;
        let rms = mean_square.sqrt();
        let level = if rms <= f32::EPSILON {
            0.0
        } else {
            let decibels = 20.0 * rms.log10();
            ((decibels + 60.0) / 60.0).clamp(0.0, 1.0)
        };
        self.smoothed_level = level.max(self.smoothed_level * 0.6);

        Ok(AudioLevels {
            level: self.smoothed_level,
            bins: self.smoothed.clone(),
        })
    }

    fn decay(&mut self) -> AudioLevels {
        for value in &mut self.smoothed {
            *value *= 0.6;
        }
        self.smoothed_level *= 0.6;
        AudioLevels {
            level: self.smoothed_level,
            bins: self.smoothed.clone(),
        }
    }
}

/// Maps each display band onto a contiguous range of FFT bins using a
/// logarithmic frequency scale, which matches how we hear.
fn band_edges(sample_rate: u32) -> Vec<(usize, usize)> {
    let nyquist = sample_rate as f32 / 2.0;
    let max_frequency = MAX_FREQUENCY.min(nyquist * 0.95);
    let min_frequency = MIN_FREQUENCY.min(max_frequency * 0.5);
    let ratio = (max_frequency / min_frequency).powf(1.0 / BAND_COUNT as f32);

    let bin_width = sample_rate as f32 / FFT_SIZE as f32;
    let mut edges = Vec::with_capacity(BAND_COUNT);
    let mut previous = 1_usize;
    let mut frequency = min_frequency;

    for _ in 0..BAND_COUNT {
        let next_frequency = frequency * ratio;
        let start = ((frequency / bin_width).round() as usize).max(previous);
        let end = ((next_frequency / bin_width).round() as usize)
            .max(start + 1)
            .min(FFT_SIZE / 2);
        edges.push((start, end));
        previous = end;
        frequency = next_frequency;
    }

    edges
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(frequency: f32, sample_rate: u32, amplitude: f32, length: usize) -> Vec<f32> {
        (0..length)
            .map(|index| {
                (2.0 * std::f32::consts::PI * frequency * index as f32 / sample_rate as f32).sin()
                    * amplitude
            })
            .collect()
    }

    #[test]
    fn silence_stays_quiet() {
        let mut meter = Meter::new(16_000).expect("meter");
        let levels = meter.analyze(&vec![0.0; FFT_SIZE]).expect("analysis");
        assert!(levels.level < 0.01);
        assert_eq!(levels.bins.len(), BAND_COUNT);
        assert!(levels.bins.iter().all(|value| *value < 0.01));
    }

    #[test]
    fn tone_lights_up_the_spectrum() {
        let mut meter = Meter::new(16_000).expect("meter");
        let tone = sine(1_000.0, 16_000, 0.8, FFT_SIZE);
        let levels = meter.analyze(&tone).expect("analysis");
        assert!(levels.level > 0.7, "level was {}", levels.level);
        let loudest = levels.bins.iter().fold(0.0_f32, |acc, value| acc.max(*value));
        assert!(loudest > 0.8, "loudest band was {loudest}");
    }

    #[test]
    fn bands_are_ordered_below_nyquist() {
        let bands = band_edges(48_000);
        assert_eq!(bands.len(), BAND_COUNT);
        assert!(bands.iter().all(|(start, end)| start < end && *end <= FFT_SIZE / 2));
    }
}
