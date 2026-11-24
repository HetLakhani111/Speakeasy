//! Sample-rate conversion and silence trimming.
//!
//! Whisper always wants 16 kHz mono f32. Microphones almost never deliver that,
//! so every clip passes through the windowed-sinc resampler below before it is
//! handed to the transcriber. The implementation is deliberately dependency
//! free: it runs once per clip, offline, so a short band-limited interpolation
//! is both cheap and predictable.

pub const TARGET_SAMPLE_RATE: u32 = 16_000;

/// Zero crossings kept on each side of the interpolation point. 16 taps per
/// side (33 total) gives roughly 60 dB of alias rejection with a Blackman
/// window at these ratios.
const TAPS: usize = 16;

/// Resamples `input` from `from_rate` to `to_rate`.
///
/// The output length is rounded to the nearest sample. When downsampling, the
/// sinc cutoff is lowered below the output Nyquist frequency so that content
/// which cannot be represented is filtered out instead of folding back into
/// the speech band.
pub fn resample(input: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    if input.is_empty() || from_rate == 0 || to_rate == 0 {
        return Vec::new();
    }
    if from_rate == to_rate {
        return input.to_vec();
    }

    let ratio = to_rate as f64 / from_rate as f64;
    let output_len = ((input.len() as f64) * ratio).round().max(1.0) as usize;

    // Normalized to the input sample rate. 0.5 == input Nyquist.
    let cutoff = 0.5 * ratio.min(1.0) * 0.94;
    let taps = TAPS as isize;
    let last_index = input.len() as isize - 1;

    let mut output = Vec::with_capacity(output_len);
    for index in 0..output_len {
        let position = index as f64 / ratio;
        let base = position.floor() as isize;
        let fraction = position - base as f64;

        let mut accumulator = 0.0_f64;
        let mut weight = 0.0_f64;

        for tap in -taps..=taps {
            let source = base + tap;
            if source < 0 || source > last_index {
                continue;
            }

            // Distance from the interpolation point to this input sample.
            let distance = fraction - tap as f64;
            let coefficient = sinc_filter(distance, cutoff) * blackman(distance / taps as f64);

            accumulator += f64::from(input[source as usize]) * coefficient;
            weight += coefficient;
        }

        // Normalizing removes the small DC ripple introduced by the finite
        // window and keeps the overall level stable.
        let sample = if weight.abs() > 1e-12 {
            accumulator / weight
        } else {
            0.0
        };
        output.push(sample as f32);
    }

    output
}

pub fn resample_to_16k(input: &[f32], sample_rate: u32) -> Vec<f32> {
    resample(input, sample_rate, TARGET_SAMPLE_RATE)
}

/// Ideal low-pass impulse response, `2 * fc * sinc(2 * fc * x)`.
fn sinc_filter(distance: f64, cutoff: f64) -> f64 {
    let x = 2.0 * cutoff * distance;
    if x.abs() < 1e-9 {
        2.0 * cutoff
    } else {
        2.0 * cutoff * (std::f64::consts::PI * x).sin() / (std::f64::consts::PI * x)
    }
}

/// Blackman window over `[-1, 1]`; zero at both ends.
fn blackman(position: f64) -> f64 {
    let position = position.clamp(-1.0, 1.0);
    0.42 + 0.5 * (std::f64::consts::PI * position).cos()
        + 0.08 * (2.0 * std::f64::consts::PI * position).cos()
}

/// Trims leading and trailing silence while keeping a short natural margin.
///
/// Returns the trimmed slice bounds. A clip that never crosses the speech
/// threshold collapses to an empty range so callers can discard it.
pub fn trim_silence_bounds(samples: &[f32], sample_rate: u32) -> (usize, usize) {
    if samples.is_empty() || sample_rate == 0 {
        return (0, 0);
    }

    let frame_len = ((sample_rate as usize) / 50).max(1); // 20 ms
    let frame_count = samples.len().div_ceil(frame_len);
    if frame_count == 0 {
        return (0, 0);
    }

    let mut levels = Vec::with_capacity(frame_count);
    let mut peak = 0.0_f32;
    for frame in samples.chunks(frame_len) {
        let mean_square = frame.iter().map(|s| s * s).sum::<f32>() / frame.len() as f32;
        let rms = mean_square.sqrt();
        peak = peak.max(rms);
        levels.push(rms);
    }

    if peak <= f32::EPSILON {
        return (0, 0);
    }

    // Relative to the loudest frame, with an absolute floor so that a noisy
    // room does not keep the whole clip alive. The floor sits around
    // -56 dBFS, low enough for quiet headsets but above typical room tone.
    // On a very quiet signal the floor is scaled down with the peak instead;
    // otherwise it would sit close to the speech level and cut most of it.
    let threshold = (peak * 0.06).max(0.0015_f32.min(peak * 0.3));
    let Some(first) = levels.iter().position(|level| *level >= threshold) else {
        return (0, 0);
    };
    let Some(last) = levels.iter().rposition(|level| *level >= threshold) else {
        return (0, 0);
    };

    // Keep 150 ms of natural lead-in and tail, without growing past the clip.
    let padding = ((sample_rate as usize) * 150) / 1000;
    let start = first.saturating_mul(frame_len).saturating_sub(padding);
    let end = ((last + 1) * frame_len + padding).min(samples.len());
    (start, end)
}

pub fn trim_silence(samples: &mut Vec<f32>, sample_rate: u32) -> std::ops::Range<usize> {
    let (start, end) = trim_silence_bounds(samples, sample_rate);
    if end <= start {
        samples.clear();
        return 0..0;
    }
    samples.drain(end..);
    samples.drain(..start);
    0..samples.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    fn sine(frequency: f64, sample_rate: u32, amplitude: f64, seconds: f64) -> Vec<f32> {
        let length = (sample_rate as f64 * seconds).round() as usize;
        (0..length)
            .map(|index| {
                let phase = 2.0 * PI * frequency * index as f64 / sample_rate as f64;
                (phase.sin() * amplitude) as f32
            })
            .collect()
    }

    fn rms(samples: &[f32]) -> f64 {
        if samples.is_empty() {
            return 0.0;
        }
        let mean_square = samples
            .iter()
            .map(|sample| f64::from(*sample) * f64::from(*sample))
            .sum::<f64>()
            / samples.len() as f64;
        mean_square.sqrt()
    }

    fn goertzel(samples: &[f32], sample_rate: u32, frequency: f64) -> f64 {
        let omega = 2.0 * PI * frequency / sample_rate as f64;
        let coefficient = 2.0 * omega.cos();
        let mut s_prev = 0.0;
        let mut s_prev2 = 0.0;
        for sample in samples {
            let s = f64::from(*sample) + coefficient * s_prev - s_prev2;
            s_prev2 = s_prev;
            s_prev = s;
        }
        (s_prev2 * s_prev2 + s_prev * s_prev - coefficient * s_prev * s_prev2)
            .sqrt()
            / samples.len() as f64
    }

    #[test]
    fn preserves_duration_and_level() {
        let input = sine(440.0, 48_000, 0.5, 1.0);
        let output = resample(&input, 48_000, 16_000);

        assert!(
            (output.len() as i64 - 16_000).abs() <= 2,
            "unexpected output length {}",
            output.len()
        );

        let body = &output[800..output.len() - 800];
        let amplitude = rms(body) * std::f64::consts::SQRT_2;
        assert!(
            (amplitude - 0.5).abs() < 0.01,
            "amplitude drifted to {amplitude}"
        );

        let at_440 = goertzel(body, 16_000, 440.0);
        let at_1000 = goertzel(body, 16_000, 1000.0);
        assert!(at_440 > at_1000 * 10.0, "440 Hz was not dominant");
    }

    #[test]
    fn attenuates_content_above_output_nyquist() {
        // 10 kHz can exist at 48 kHz but not at 16 kHz; it must not alias down
        // into the speech band.
        let input = sine(10_000.0, 48_000, 0.5, 0.5);
        let output = resample(&input, 48_000, 16_000);
        let body = &output[400..output.len() - 400];
        assert!(rms(body) < 0.02, "alias leaked through: {}", rms(body));
    }

    #[test]
    fn same_rate_is_passthrough() {
        let input = vec![0.1, -0.2, 0.3];
        assert_eq!(resample(&input, 16_000, 16_000), input);
    }

    #[test]
    fn trims_leading_and_trailing_silence() {
        let mut clip = vec![0.0_f32; 16_000];
        let speech = sine(300.0, 16_000, 0.2, 1.0);
        clip.extend_from_slice(&speech);
        clip.extend(std::iter::repeat_n(0.0, 16_000));

        let (start, end) = trim_silence_bounds(&clip, 16_000);
        assert!(start >= 13_500 && start <= 16_000, "start was {start}");
        assert!(end >= 32_000 && end <= 34_500, "end was {end}");
    }

    #[test]
    fn silence_trims_to_nothing() {
        let clip = vec![0.0_f32; 8_000];
        assert_eq!(trim_silence_bounds(&clip, 16_000), (0, 0));
    }

    #[test]
    fn keeps_quiet_speech() {
        // Quiet headset input around -48 dBFS peak should still survive the
        // absolute trim floor; this is roughly where TTS-through-speakers
        // lands during a microphone smoke test.
        let mut clip = vec![0.0_f32; 8_000];
        clip.extend(sine(300.0, 16_000, 0.004, 0.8));
        clip.extend(std::iter::repeat_n(0.0, 8_000));

        let (start, end) = trim_silence_bounds(&clip, 16_000);
        assert!(end > start, "quiet speech was discarded");
        assert!(end - start >= 12_800, "quiet speech was over-trimmed");
    }

    #[test]
    fn keeps_speech_below_the_absolute_floor() {
        // A distant laptop microphone can deliver speech at -60 dBFS, well
        // under the fixed floor. The threshold has to follow the peak down or
        // the whole sentence is trimmed away.
        let mut clip = vec![0.0_f32; 8_000];
        clip.extend(sine(300.0, 16_000, 0.0015, 1.0));
        clip.extend(std::iter::repeat_n(0.0, 8_000));

        let (start, end) = trim_silence_bounds(&clip, 16_000);
        assert!(end - start >= 13_000, "very quiet speech was over-trimmed");
    }
}
