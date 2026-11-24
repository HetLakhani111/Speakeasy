//! Minimal WAV output used by the development smoke test.
//!
//! Release builds never persist recordings; this exists so the microphone
//! pipeline can be verified by ear while developing.

use std::fs;
use std::path::Path;

use crate::error::{io_error, Error, Result};

pub fn write_pcm16(path: &Path, samples: &[f32], sample_rate: u32) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| io_error(parent, err))?;
    }

    let spec = hound::WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };

    let mut writer =
        hound::WavWriter::create(path, spec).map_err(|err| Error::Audio(err.to_string()))?;
    for sample in samples {
        let value = if sample.is_finite() { *sample } else { 0.0 };
        let scaled = (value.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16;
        writer
            .write_sample(scaled)
            .map_err(|err| Error::Audio(err.to_string()))?;
    }
    writer
        .finalize()
        .map_err(|err| Error::Audio(err.to_string()))?;
    Ok(())
}

/// Development helper: decodes a WAV file to mono f32 so the capture path can
/// be replayed without a working microphone. Debug builds, tests, and the
/// `dev-wav` feature only.
#[cfg(any(debug_assertions, test, feature = "dev-wav"))]
pub fn read_mono(path: &Path) -> Result<(Vec<f32>, u32)> {
    let mut reader = hound::WavReader::open(path)
        .map_err(|err| Error::Audio(format!("could not read {}: {err}", path.display())))?;
    let spec = reader.spec();
    let channels = usize::from(spec.channels.max(1));

    let decode_error = |err: hound::Error| Error::Audio(format!("could not decode {}: {err}", path.display()));
    let raw: Vec<f32> = match (spec.sample_format, spec.bits_per_sample) {
        (hound::SampleFormat::Float, _) => reader
            .samples::<f32>()
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(decode_error)?,
        (hound::SampleFormat::Int, 8) => reader
            .samples::<i8>()
            .map(|sample| sample.map(|value| f32::from(value) / 128.0))
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(decode_error)?,
        (hound::SampleFormat::Int, bits) => {
            let scale = 1.0 / (1_i64 << (u32::from(bits.min(31)) - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|sample| sample.map(|value| value as f32 * scale))
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(decode_error)?
        }
    };

    let mono = raw
        .chunks(channels)
        .map(|frame| frame.iter().sum::<f32>() / frame.len() as f32)
        .collect();
    Ok((mono, spec.sample_rate))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_pcm16() {
        let path = std::env::temp_dir().join(format!(
            "speakeasy-wav-test-{}.wav",
            std::process::id()
        ));
        let samples = vec![0.0, 0.5, -0.5, 1.0];
        write_pcm16(&path, &samples, 16_000).expect("write");

        let mut reader = hound::WavReader::open(&path).expect("open");
        let spec = reader.spec();
        assert_eq!(spec.channels, 1);
        assert_eq!(spec.sample_rate, 16_000);
        assert_eq!(spec.bits_per_sample, 16);

        let decoded = reader
            .samples::<i16>()
            .map(|sample| sample.expect("sample"))
            .collect::<Vec<_>>();
        assert_eq!(decoded.len(), samples.len());

        let _ = fs::remove_file(path);
    }
}
