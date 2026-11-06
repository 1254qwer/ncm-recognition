use crate::mp4::{self, SAMPLE_RATE};
use rusty_aac::{AacEncoder, AacEncoderConfig};

pub fn encode(samples: &[f32], sample_rate: u32) -> Result<Vec<u8>, String> {
    if sample_rate != SAMPLE_RATE {
        return Err("ASR requires 16000 Hz mono PCM".into());
    }
    if samples.is_empty() {
        return Err("ASR recording is empty".into());
    }
    if samples
        .iter()
        .any(|sample| !sample.is_finite() || sample.abs() > 1.0)
    {
        return Err("ASR requires finite normalized PCM in [-1,1]".into());
    }
    let mut encoder = AacEncoder::new(AacEncoderConfig {
        bitrate_bps: 48_000,
        ..Default::default()
    });
    encoder
        .push_pcm(samples, 1, SAMPLE_RATE)
        .map_err(|error| error.to_string())?;
    encoder.finish();
    let mut packets = Vec::new();
    loop {
        match encoder.next_packet() {
            Ok(packet) => packets.push(packet.data),
            Err(rusty_aac::Error::Eof) => break,
            Err(error) => return Err(error.to_string()),
        }
    }
    mp4::mux_asr(&packets)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mp4::test_support::packets;
    use rusty_aac::AacDecoder;

    #[test]
    fn aac_round_trip_preserves_audio() {
        let samples: Vec<_> = (0..8000)
            .map(|i| (std::f32::consts::TAU * 440.0 * i as f32 / SAMPLE_RATE as f32).sin() * 0.3)
            .collect();
        let bytes = encode(&samples, SAMPLE_RATE).unwrap();
        let packets = packets(&bytes);
        assert_eq!(packets.len(), 9);
        let mut decoder = AacDecoder::with_config_bytes(&mp4::ASC).unwrap();
        let decoded: Vec<_> = packets
            .iter()
            .flat_map(|packet| decoder.decode(packet, None).unwrap().samples)
            .collect();
        assert_eq!(decoded.len(), 9 * 1024);
        let mut signal = 0.0;
        let mut noise = 0.0;
        for i in 1024..7000 {
            let sample = samples[i] as f64;
            let restored = decoded[1024 + i] as f64;
            signal += sample * sample;
            noise += (sample - restored) * (sample - restored);
        }
        assert!(10.0 * (signal / noise).log10() > 15.0);
    }

    #[test]
    fn invalid_pcm_is_rejected() {
        assert!(encode(&[], SAMPLE_RATE).is_err());
        assert!(encode(&[f32::NAN], SAMPLE_RATE).is_err());
        assert!(encode(&[1.1], SAMPLE_RATE).is_err());
        assert!(encode(&[0.0], 8000).is_err());
    }
}
