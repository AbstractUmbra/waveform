use base64::{engine::general_purpose, Engine as _};
use bytemuck;
use pyo3::prelude::*;
use pyo3::pyfunction;
use std::io::Cursor;
use std::num::{NonZeroU32, NonZeroU8};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::default;
use vorbis_rs::{VorbisEncoderBuilder, VorbisError};

#[pyclass]
struct AudioResult {
    ogg_data: Vec<u8>,
    waveform_base64: String,
    duration_seconds: f64,
}

fn decode_to_pcm(input: &[u8]) -> Result<(Vec<f32>, usize, u32), Box<dyn std::error::Error>> {
    let hint = Hint::new();
    let cursor = Cursor::new(input.to_vec());
    let boxed_cursor: Box<dyn symphonia::core::io::MediaSource> = Box::new(cursor);
    let mss = MediaSourceStream::new(boxed_cursor, Default::default());

    let probed = default::get_probe().format(
        &hint,
        mss,
        &FormatOptions::default(),
        &MetadataOptions::default(),
    )?;
    let mut format = probed.format;

    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL)
        .ok_or("No supported audio tracks")?;
    let mut decoder = default::get_codecs().make(&track.codec_params, &Default::default())?;

    let sample_rate = track
        .codec_params
        .sample_rate
        .ok_or("Unknown sample rate")?;
    let channels = track
        .codec_params
        .channels
        .ok_or("Unknown channels")?
        .count();

    let mut pcm = Vec::new();
    let track_id = track.id;

    loop {
        let packet = match format.next_packet() {
            Ok(pkt) => pkt,
            Err(err) => {
                use symphonia::core::errors::Error;
                match err {
                    Error::ResetRequired => break,
                    Error::IoError(_) | Error::DecodeError(_) => continue,
                    _ => break,
                }
            }
        };

        if packet.track_id() != track_id {
            continue;
        }

        match decoder.decode(&packet) {
            Ok(audio_buf) => {
                let mut sample_buf =
                    SampleBuffer::<f32>::new(audio_buf.capacity() as u64, *audio_buf.spec());
                sample_buf.copy_interleaved_ref(audio_buf);
                pcm.extend_from_slice(sample_buf.samples());
            }
            Err(_) => continue,
        }
    }

    Ok((pcm, channels, sample_rate))
}

fn encode_to_ogg(pcm: &[f32], channels: usize, sample_rate: u32) -> Result<Vec<u8>, VorbisError> {
    let mut output = Vec::new();

    let rate_nz = NonZeroU32::new(sample_rate).expect("blah.");

    let ch_nz = NonZeroU8::new(channels as u8).expect("blah.");

    let mut builder = VorbisEncoderBuilder::new(rate_nz, ch_nz, &mut output)?;
    let mut encoder = builder.build()?;

    let frame_count = pcm.len() / channels;
    let mut planar = vec![Vec::with_capacity(frame_count); channels];
    for (i, &sample) in pcm.iter().enumerate() {
        planar[i % channels].push(sample);
    }
    let planar_refs: Vec<&[f32]> = planar.iter().map(Vec::as_slice).collect();

    encoder.encode_audio_block(&planar_refs)?;
    encoder.finish()?;

    Ok(output)
}
fn compute_waveform_base64(pcm: &[f32], chunk_size: usize) -> String {
    let waveform: Vec<f32> = pcm
        .chunks(chunk_size)
        .map(|chunk| chunk.iter().map(|v| v.abs()).fold(0.0f32, f32::max))
        .collect();

    let bytes: &[u8] = bytemuck::cast_slice(&waveform);

    general_purpose::STANDARD.encode(bytes)
}

fn process_audio(input_data: &[u8]) -> AudioResult {
    let (pcm, channels, sample_rate) =
        decode_to_pcm(input_data).expect("Unable to process data as PCM.");
    let ogg_data = encode_to_ogg(&pcm, channels, sample_rate)
        .expect("Unable to encode the audio data to OGG format.");
    let waveform_base64 = compute_waveform_base64(&pcm, 1024);

    let total_samples = pcm.len();
    let duration_seconds = total_samples as f64 / (channels as f64 * sample_rate as f64);

    AudioResult {
        ogg_data,
        waveform_base64,
        duration_seconds,
    }
}

#[pyfunction]
#[pyo3(name = "generate")]
fn generate_waveform_from_audio(audio: &[u8]) -> AudioResult {
    process_audio(audio)
}

#[pymodule]
pub fn waveform(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(generate_waveform_from_audio, m)?)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Read as _;

    use super::*;

    #[test]
    fn test_lib() {
        let mut file = std::fs::File::open("test.mp3").expect("Unable to open test mp3.");

        let mut buf = Vec::new();
        file.read_to_end(&mut buf)
            .expect("Unable to read audio file into buffer");

        let result = generate_waveform_from_audio(&buf);
        println!(
            "{:#?}\n\n{:#?}",
            result.duration_seconds, result.waveform_base64
        )
    }
}
