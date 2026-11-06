use crate::afp::{AfpResult, Peak};
use aes::cipher::{BlockEncrypt, KeyInit};
use aes::Aes128;
use flate2::{write::ZlibEncoder, Compression};
use md5::{Digest, Md5};
use std::io::Write;

const AFP_VERSION: &str = "hyai_1.2.0_client_1.0.0";
const HFP_VERSION: &str = "1.1.1";
const AFP_KEY: &[u8; 16] = b"4B97221F27F02907";

fn sized_text(out: &mut Vec<u8>, text: &str) {
    out.extend_from_slice(&(text.len() as u32).to_le_bytes());
    out.extend_from_slice(text.as_bytes());
}

fn write_peaks(out: &mut Vec<u8>, peaks: &[Peak]) {
    out.extend_from_slice(b"FPVER");
    sized_text(out, AFP_VERSION);
    out.extend_from_slice(b"Peak");
    out.extend_from_slice(&(peaks.len() as u32).to_le_bytes());
    for peak in peaks {
        out.extend_from_slice(&peak.frequency_bin.to_le_bytes());
        out.extend_from_slice(&peak.time_frame.to_le_bytes());
        out.extend_from_slice(&peak.score.to_le_bytes());
    }
}

pub fn encode_afp(result: &AfpResult) -> Result<Vec<u8>, String> {
    let mut plain = Vec::new();
    sized_text(&mut plain, AFP_VERSION);
    plain.extend_from_slice(&0u32.to_le_bytes());
    plain.extend_from_slice(&0f32.to_le_bytes());
    plain.extend_from_slice(&result.duration_seconds.to_le_bytes());
    write_peaks(&mut plain, &result.normal);
    write_peaks(&mut plain, &result.dense);
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::new(6));
    encoder
        .write_all(&plain)
        .map_err(|error| error.to_string())?;
    let compressed = encoder.finish().map_err(|error| error.to_string())?;
    Ok(encrypt_afp(&compressed))
}

fn encrypt_afp(data: &[u8]) -> Vec<u8> {
    let padding = 16 - data.len() % 16;
    let mut output = data.to_vec();
    output.resize(data.len() + padding, padding as u8);
    let cipher = Aes128::new(AFP_KEY.into());
    for block in output.chunks_exact_mut(16) {
        cipher.encrypt_block(block.into());
    }
    output
}

pub fn encode_hfp(grid: &[i16]) -> Vec<u8> {
    let mut body = b"FPVER".to_vec();
    body.extend_from_slice(&(HFP_VERSION.len() as u32).to_le_bytes());
    sized_text(&mut body, HFP_VERSION);
    body.extend_from_slice(b"FP");
    body.extend_from_slice(&(grid.len() as u32).to_le_bytes());
    for pitch in grid {
        body.extend_from_slice(&pitch.to_le_bytes());
    }
    for byte in &mut body {
        *byte = !*byte;
    }
    let mut packet = vec![0, 1];
    packet.extend_from_slice(format!("{:x}", Md5::digest(&body)).as_bytes());
    packet.push(0);
    packet.extend_from_slice(&body);
    packet
}

#[cfg(test)]
mod tests {
    use super::*;
    use aes::cipher::BlockDecrypt;
    use flate2::read::ZlibDecoder;
    use std::io::Read;

    fn decrypt_padded(data: &mut [u8]) {
        let cipher = Aes128::new(AFP_KEY.into());
        for block in data.chunks_exact_mut(16) {
            cipher.decrypt_block(block.into());
        }
    }

    #[test]
    fn aes_padding_covers_empty_and_aligned_inputs() {
        for length in 0..65 {
            let input = vec![42; length];
            let mut output = encrypt_afp(&input);
            decrypt_padded(&mut output);
            assert_eq!(&output[..length], &input);
            let padding = 16 - length % 16;
            assert_eq!(output.len(), length + padding);
            assert!(output[length..]
                .iter()
                .all(|&byte| byte as usize == padding));
        }
    }

    #[test]
    fn afp_packet_contains_both_peak_sequences() {
        let result = AfpResult {
            duration_seconds: 3.0,
            normal: vec![Peak {
                frequency_bin: 20,
                time_frame: 1,
                score: 2.5,
            }],
            dense: vec![],
        };
        let mut compressed = encode_afp(&result).unwrap();
        decrypt_padded(&mut compressed);
        compressed.truncate(compressed.len() - *compressed.last().unwrap() as usize);
        let mut plain = Vec::new();
        ZlibDecoder::new(&compressed[..])
            .read_to_end(&mut plain)
            .unwrap();
        let header_end = 4 + AFP_VERSION.len();
        assert_eq!(&plain[4..header_end], AFP_VERSION.as_bytes());
        assert_eq!(&plain[header_end..header_end + 8], &[0; 8]);
        assert_eq!(&plain[header_end + 8..header_end + 12], &3f32.to_le_bytes());
        let sequence_size = 5 + 4 + AFP_VERSION.len() + 4 + 4;
        let peak_start = header_end + 12 + sequence_size;
        assert_eq!(&plain[peak_start..peak_start + 4], &20i32.to_le_bytes());
        assert_eq!(&plain[peak_start + 4..peak_start + 8], &1i32.to_le_bytes());
        assert_eq!(
            &plain[peak_start + 8..peak_start + 12],
            &2.5f32.to_le_bytes()
        );
        assert_eq!(&plain[peak_start + 12..peak_start + 17], b"FPVER");
        assert_eq!(plain.len(), header_end + 12 + 2 * sequence_size + 12);
    }

    #[test]
    fn hfp_packet_preserves_grid_and_checksum() {
        let packet = encode_hfp(&[0, 6900, 6950, 0]);
        assert_eq!(&packet[..2], &[0, 1]);
        assert_eq!(packet[34], 0);
        assert_eq!(
            &packet[2..34],
            format!("{:x}", Md5::digest(&packet[35..])).as_bytes()
        );
        let body: Vec<u8> = packet[35..].iter().map(|byte| !byte).collect();
        assert_eq!(&body[..24], b"FPVER\x05\0\0\0\x05\0\0\x001.1.1FP\x04\0\0\0");
        assert_eq!(&body[24..], &[0, 0, 244, 26, 38, 27, 0, 0]);
    }
}
