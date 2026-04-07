//! Microphone capture via cpal → 16 kHz mono f32 buffer for Whisper.

use anyhow::{bail, Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, SampleFormat, Stream, StreamConfig};
use crossbeam_channel::Sender;
use parking_lot::Mutex;
use rubato::{FftFixedIn, Resampler};
use std::sync::Arc;

/// Thread-safe shared audio state (Send + Sync).
/// This can be sent to the orchestration thread while the Stream stays
/// on the main thread.
pub struct AudioShared {
    recording: Arc<Mutex<bool>>,
    buffer: Arc<Mutex<Vec<f32>>>,
    device_sample_rate: u32,
}

impl AudioShared {
    /// Begin accumulating audio samples.
    pub fn start_recording(&self) {
        self.buffer.lock().clear();
        *self.recording.lock() = true;
        log::debug!("recording started");
    }

    /// Stop accumulating and return 16 kHz mono f32 samples.
    pub fn stop_recording(&self) -> Result<Vec<f32>> {
        *self.recording.lock() = false;
        let raw = {
            let mut buf = self.buffer.lock();
            std::mem::take(&mut *buf)
        };
        log::debug!(
            "recording stopped – {} samples at {}Hz",
            raw.len(),
            self.device_sample_rate
        );

        if raw.is_empty() {
            return Ok(vec![]);
        }

        if self.device_sample_rate == 16000 {
            return Ok(raw);
        }

        // Resample to 16 kHz
        resample(&raw, self.device_sample_rate, 16000)
    }
}

/// Captures audio from the default (or named) input device.
/// Must stay on the thread that created it (cpal::Stream is not Send).
pub struct AudioCapture {
    _stream: Stream,
    pub shared: Arc<AudioShared>,
}

impl AudioCapture {
    /// Create a new capture.
    ///
    /// * `device_name` – substring match on the device name; empty → default.
    /// * `error_tx` – channel for forwarding stream errors to main thread.
    pub fn new(device_name: &str, error_tx: Sender<String>) -> Result<Self> {
        let host = cpal::default_host();

        let device = if device_name.is_empty() {
            host.default_input_device()
                .context("no default input device available")?
        } else {
            find_device_by_name(&host, device_name)?
        };

        let supported = device
            .default_input_config()
            .context("no supported input config")?;
        let sample_rate = supported.sample_rate().0;
        let channels = supported.channels() as usize;
        log::info!(
            "audio device: {} ({}Hz, {}ch, {:?})",
            device.name().unwrap_or_default(),
            sample_rate,
            channels,
            supported.sample_format()
        );

        let recording = Arc::new(Mutex::new(false));
        let buffer = Arc::new(Mutex::new(Vec::<f32>::with_capacity(16000 * 30))); // ~30s pre-alloc

        let rec = Arc::clone(&recording);
        let buf = Arc::clone(&buffer);

        let config: StreamConfig = supported.clone().into();
        let sample_format = supported.sample_format();

        let stream = match sample_format {
            SampleFormat::F32 => device.build_input_stream(
                &config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    if *rec.lock() {
                        let mono = mix_to_mono(data, channels);
                        buf.lock().extend_from_slice(&mono);
                    }
                },
                {
                    let etx = error_tx.clone();
                    move |err| {
                        let _ = etx.send(format!("audio stream error: {err}"));
                    }
                },
                None,
            )?,
            SampleFormat::I16 => device.build_input_stream(
                &config,
                move |data: &[i16], _: &cpal::InputCallbackInfo| {
                    if *rec.lock() {
                        let floats: Vec<f32> =
                            data.iter().map(|&s| s as f32 / i16::MAX as f32).collect();
                        let mono = mix_to_mono(&floats, channels);
                        buf.lock().extend_from_slice(&mono);
                    }
                },
                {
                    let etx = error_tx.clone();
                    move |err| {
                        let _ = etx.send(format!("audio stream error: {err}"));
                    }
                },
                None,
            )?,
            SampleFormat::U16 => device.build_input_stream(
                &config,
                move |data: &[u16], _: &cpal::InputCallbackInfo| {
                    if *rec.lock() {
                        let floats: Vec<f32> = data
                            .iter()
                            .map(|&s| (s as f32 / u16::MAX as f32) * 2.0 - 1.0)
                            .collect();
                        let mono = mix_to_mono(&floats, channels);
                        buf.lock().extend_from_slice(&mono);
                    }
                },
                {
                    let etx = error_tx.clone();
                    move |err| {
                        let _ = etx.send(format!("audio stream error: {err}"));
                    }
                },
                None,
            )?,
            fmt => bail!("unsupported sample format: {fmt:?}"),
        };

        stream.play().context("failed to start audio stream")?;

        let shared = Arc::new(AudioShared {
            recording,
            buffer,
            device_sample_rate: sample_rate,
        });

        Ok(Self {
            _stream: stream,
            shared,
        })
    }
}

// ── helpers ───────────────────────────────────────────────────────────

/// Downmix interleaved multi-channel audio to mono by averaging channels.
fn mix_to_mono(interleaved: &[f32], channels: usize) -> Vec<f32> {
    if channels == 1 {
        return interleaved.to_vec();
    }
    interleaved
        .chunks_exact(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect()
}

/// Resample a mono f32 signal from `from_hz` to `to_hz` using rubato FFT resampler.
fn resample(input: &[f32], from_hz: u32, to_hz: u32) -> Result<Vec<f32>> {
    let chunk_size = 1024;
    let mut resampler = FftFixedIn::<f32>::new(from_hz as usize, to_hz as usize, chunk_size, 1, 1)
        .context("failed to create resampler")?;

    let mut output = Vec::with_capacity((input.len() as f64 * to_hz as f64 / from_hz as f64) as usize + 1024);

    let mut pos = 0;
    while pos + chunk_size <= input.len() {
        let chunk = &input[pos..pos + chunk_size];
        let result = resampler.process(&[chunk], None)?;
        output.extend_from_slice(&result[0]);
        pos += chunk_size;
    }

    // Process remaining samples by zero-padding to chunk_size
    if pos < input.len() {
        let mut last_chunk = vec![0.0f32; chunk_size];
        let remaining = input.len() - pos;
        last_chunk[..remaining].copy_from_slice(&input[pos..]);
        let result = resampler.process(&[&last_chunk], None)?;
        // Only take the proportional amount of output
        let expected = (remaining as f64 * to_hz as f64 / from_hz as f64) as usize;
        let take = expected.min(result[0].len());
        output.extend_from_slice(&result[0][..take]);
    }

    Ok(output)
}

fn find_device_by_name(host: &cpal::Host, name: &str) -> Result<Device> {
    let name_lower = name.to_lowercase();
    let devices = host
        .input_devices()
        .context("failed to enumerate input devices")?;
    for d in devices {
        if let Ok(n) = d.name() {
            if n.to_lowercase().contains(&name_lower) {
                return Ok(d);
            }
        }
    }
    bail!("no input device matching '{name}' found");
}

/// Write 16 kHz mono f32 samples to a WAV file (16-bit PCM, as whisper.cpp expects).
pub fn write_wav(samples: &[f32], path: &std::path::Path) -> Result<()> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 16000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec)
        .with_context(|| format!("failed to create WAV file {}", path.display()))?;
    for &sample in samples {
        let clamped = sample.clamp(-1.0, 1.0);
        let pcm = (clamped * i16::MAX as f32) as i16;
        writer.write_sample(pcm)?;
    }
    writer.finalize()?;
    Ok(())
}
