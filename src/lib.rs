mod afp;
mod asr;
mod fingerprint;
mod hfp;
mod hfp_hmm;
mod mp4;

use base64::{engine::general_purpose::STANDARD, Engine};

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::wasm_bindgen;

#[cfg_attr(target_arch = "wasm32", wasm_bindgen)]
pub fn fingerprint_afp(samples: &[f32], sample_rate: u32) -> Result<String, String> {
    let result = afp::extract(samples, sample_rate)?;
    Ok(STANDARD.encode(fingerprint::encode_afp(&result)?))
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen)]
pub fn fingerprint_hfp(samples: &[f32], sample_rate: u32) -> Result<String, String> {
    let grid = hfp::extract(samples, sample_rate)?;
    if grid.is_empty() {
        return Ok(String::new());
    }
    Ok(STANDARD.encode(fingerprint::encode_hfp(&grid)))
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen)]
pub fn audio_asr(samples: &[f32], sample_rate: u32) -> Result<String, String> {
    Ok(STANDARD.encode(asr::encode(samples, sample_rate)?))
}
