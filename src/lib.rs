use ffmpeg_next::{self as ffmpeg, Dictionary};

use base64::prelude::*;
use pyo3::prelude::*;
use pyo3::pyfunction;
use std::io::Write;

pub fn calculate_duration(pcm_data: Vec<u8>, sample_rate: u64, bytes_per_sample: u64) -> f64 {
    pcm_data.len() as f64 / (sample_rate * bytes_per_sample) as f64
}

pub fn extract_waveform_points(
    pcm_data: Vec<u8>,
    samples_needed: usize,
    samples_per_point: usize,
) -> std::io::Result<Vec<u8>> {
    let mut pcm_stream = &pcm_data[..];
    let mut waveform_points: Vec<u8> = Vec::with_capacity(samples_needed);

    for i in 0..samples_needed {
        let mut max_amplitude: f32 = 0.0;

        // Calculate max amplitude for this point
        for _ in 0..samples_per_point {
            if pcm_stream.len() >= 2 {
                let sample_bytes = &pcm_stream[0..2];
                let sample =
                    i16::from_le_bytes([sample_bytes[0], sample_bytes[1]]) as f32 / 32768.0;
                max_amplitude = max_amplitude.max(sample.abs());

                pcm_stream = &pcm_stream[2..]; // Move to the next sample
            } else {
                break; // No more samples
            }
        }

        // Normalize amplitude to 0-255
        let point = std::cmp::min(255, (max_amplitude * 255.0) as u8);
        waveform_points.push(point);

        // Skip bytes to align with the next sample point
        let bytes_to_skip = ((i + 1) * pcm_data.len() / samples_needed)
            .saturating_sub(pcm_data.len() - pcm_stream.len());
        pcm_stream = &pcm_stream[bytes_to_skip.min(pcm_stream.len())..];
    }

    Ok(waveform_points)
}

pub fn use_ffmpeg(audio: &[u8]) -> Result<Vec<u8>, ffmpeg::Error> {
    ffmpeg::init().unwrap();

    let mut input_file = tempfile::NamedTempFile::new().expect("unable to create tempfile.");
    input_file
        .write_all(audio)
        .expect("Unable to write audio data to temporary file path");

    let fp = input_file.into_temp_path();

    let mut options_dict = Dictionary::new();
    options_dict.set("f", "mp3");

    let mut input = ffmpeg::format::input_with_dictionary(&fp, options_dict)?;
    let input_stream = input
        .streams()
        .best(ffmpeg::media::Type::Audio)
        .ok_or(ffmpeg::Error::StreamNotFound)?;
    let audio_stream_index = input_stream.index();

    let codec_context = ffmpeg::codec::Context::from_parameters(input_stream.parameters())?;
    let mut decoder = codec_context.decoder().audio()?;

    let mut resampler = ffmpeg::software::resampling::Context::get(
        decoder.format(),
        decoder.channel_layout(),
        decoder.rate(),
        ffmpeg::format::Sample::I16(ffmpeg_next::format::sample::Type::Packed),
        ffmpeg::channel_layout::ChannelLayout::MONO,
        48000,
    )?;

    let mut decoded_frame = ffmpeg::frame::Audio::empty();
    let mut pcm_buffer: Vec<u8> = Vec::new();
    for (stream, packet) in input.packets() {
        if stream.index() == audio_stream_index {
            decoder.send_packet(&packet)?;

            while decoder.receive_frame(&mut decoded_frame).is_ok() {
                let mut output_frame = ffmpeg::frame::Audio::empty();
                resampler.run(&decoded_frame, &mut output_frame)?;

                pcm_buffer.extend_from_slice(output_frame.data(0));
            }
        }
    }

    decoder.send_eof()?;
    while decoder.receive_frame(&mut decoded_frame).is_ok() {
        let mut output_frame = ffmpeg::frame::Audio::empty();

        resampler.run(&decoded_frame, &mut output_frame)?;
        pcm_buffer.extend_from_slice(output_frame.data(0));
    }

    Ok(pcm_buffer)
}

#[pyfunction]
#[pyo3(name = "generate")]
fn generate_waveform_from_audio(audio: &[u8]) -> (String, f64) {
    let sample_rate = 48000;
    let bytes_per_sample = 2;

    let pcm = use_ffmpeg(audio).expect("Unable to treat the input audio as waveform builder.");

    let duration: f64 = calculate_duration(pcm.clone(), sample_rate, bytes_per_sample);

    let samples_needed = std::cmp::min(256, (duration * 10.0).round() as usize);
    let samples_per_point = pcm.len() / (bytes_per_sample as usize * samples_needed);

    let waveform_points = extract_waveform_points(pcm, samples_needed, samples_per_point)
        .expect("Unable to pull waveform points from audio data");
    let waveform_b64 = BASE64_STANDARD.encode(waveform_points);

    (waveform_b64, duration)
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
        println!("{:#?}\n\n{:#?}", result.0, result.1)
    }
}
