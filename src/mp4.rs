pub const SAMPLE_RATE: u32 = 16_000;
const FRAME_SAMPLES: u32 = 1024;
pub const ASC: [u8; 2] = [0x14, 0x08]; // AAC-LC, 16 kHz, mono

fn be16(out: &mut Vec<u8>, v: u16) {
    out.extend(v.to_be_bytes());
}
fn be32(out: &mut Vec<u8>, v: u32) {
    out.extend(v.to_be_bytes());
}
fn atom(tag: &[u8; 4], payload: Vec<u8>) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + 8);
    be32(&mut out, (payload.len() + 8) as u32);
    out.extend(tag);
    out.extend(payload);
    out
}
fn full(tag: &[u8; 4], flags: u32, payload: Vec<u8>) -> Vec<u8> {
    let mut b = flags.to_be_bytes().to_vec();
    b.extend(payload);
    atom(tag, b)
}
fn join(items: Vec<Vec<u8>>) -> Vec<u8> {
    items.into_iter().flatten().collect()
}
fn matrix(out: &mut Vec<u8>) {
    for v in [0x10000, 0, 0, 0, 0x10000, 0, 0, 0, 0x40000000] {
        be32(out, v);
    }
}
fn descriptor(tag: u8, payload: Vec<u8>) -> Vec<u8> {
    let mut b = vec![tag, payload.len() as u8];
    b.extend(payload);
    b
}

pub fn mux_asr(packets: &[Vec<u8>]) -> Result<Vec<u8>, String> {
    if packets.is_empty() {
        return Err("cannot mux an empty ASR recording".into());
    }
    let count = u32::try_from(packets.len()).map_err(|_| "too many AAC packets")?;
    // Container duration includes one additional AAC frame.
    let duration = count
        .checked_mul(FRAME_SAMPLES)
        .and_then(|n| n.checked_add(FRAME_SAMPLES))
        .ok_or("MP4 duration overflow")?;
    let media_bytes = packets
        .iter()
        .try_fold(0usize, |n, p| n.checked_add(p.len()))
        .ok_or("MP4 size overflow")?;
    if media_bytes > u32::MAX as usize - 4096 - packets.len() * 4 {
        return Err("ASR MP4 exceeds 32-bit box sizes".into());
    }
    let ftyp = atom(b"ftyp", b"mp42\0\0\0\0isommp42".to_vec());
    let make_moov = |chunk_offset: u32| {
        let movie_duration = ((duration as u64 * 1000) / SAMPLE_RATE as u64) as u32;
        let mut mvhd = vec![0; 8];
        be32(&mut mvhd, 1000);
        be32(&mut mvhd, movie_duration);
        be32(&mut mvhd, 0x10000);
        be16(&mut mvhd, 0x100);
        mvhd.extend([0; 10]);
        matrix(&mut mvhd);
        mvhd.extend([0; 24]);
        be32(&mut mvhd, 2);
        let mut tkhd = vec![0; 8];
        be32(&mut tkhd, 1);
        be32(&mut tkhd, 0);
        be32(&mut tkhd, movie_duration);
        tkhd.extend([0; 8]);
        be16(&mut tkhd, 0);
        be16(&mut tkhd, 0);
        be16(&mut tkhd, 0x100);
        be16(&mut tkhd, 0);
        matrix(&mut tkhd);
        tkhd.extend([0; 8]);
        let mut mdhd = vec![0; 8];
        be32(&mut mdhd, SAMPLE_RATE);
        be32(&mut mdhd, duration);
        be16(&mut mdhd, 0x55c4);
        be16(&mut mdhd, 0);
        let mut hdlr = vec![0; 4];
        hdlr.extend(b"soun");
        hdlr.extend([0; 12]);
        hdlr.extend(b"SoundHandler\0");
        let mut decoder = vec![0x40, 0x15, 0, 3, 0];
        // These fixed container fields are separate from encoder bitrate.
        be32(&mut decoder, 128000);
        be32(&mut decoder, 128000);
        decoder.extend(descriptor(5, ASC.to_vec()));
        let mut es = vec![0, 0, 0];
        es.extend(descriptor(4, decoder));
        es.extend(descriptor(6, vec![2]));
        let esds = full(b"esds", 0, descriptor(3, es));
        let mut mp4a = vec![0; 6];
        be16(&mut mp4a, 1);
        mp4a.extend([0; 8]);
        be16(&mut mp4a, 1);
        be16(&mut mp4a, 16);
        mp4a.extend([0; 4]);
        be32(&mut mp4a, SAMPLE_RATE << 16);
        mp4a.extend(esds);
        let mut stsd = vec![];
        be32(&mut stsd, 1);
        stsd.extend(atom(b"mp4a", mp4a));
        let mut stts = vec![];
        be32(&mut stts, if count > 1 { 2 } else { 1 });
        be32(&mut stts, 1);
        be32(&mut stts, FRAME_SAMPLES);
        if count > 1 {
            be32(&mut stts, count - 1);
            be32(&mut stts, FRAME_SAMPLES);
        }
        let mut stsz = vec![];
        be32(&mut stsz, 0);
        be32(&mut stsz, count);
        for p in packets {
            be32(&mut stsz, p.len() as u32);
        }
        let mut stsc = vec![];
        for n in [1, 1, count, 1] {
            be32(&mut stsc, n);
        }
        let mut stco = vec![];
        be32(&mut stco, 1);
        be32(&mut stco, chunk_offset);
        let stbl = atom(
            b"stbl",
            join(vec![
                full(b"stsd", 0, stsd),
                full(b"stts", 0, stts),
                full(b"stsz", 0, stsz),
                full(b"stsc", 0, stsc),
                full(b"stco", 0, stco),
            ]),
        );
        let mut dref = vec![];
        be32(&mut dref, 1);
        dref.extend(full(b"url ", 1, vec![]));
        let minf = atom(
            b"minf",
            join(vec![
                full(b"smhd", 0, vec![0; 4]),
                atom(b"dinf", full(b"dref", 0, dref)),
                stbl,
            ]),
        );
        let mdia = atom(
            b"mdia",
            join(vec![full(b"mdhd", 0, mdhd), full(b"hdlr", 0, hdlr), minf]),
        );
        atom(
            b"moov",
            join(vec![
                full(b"mvhd", 0, mvhd),
                atom(b"trak", join(vec![full(b"tkhd", 7, tkhd), mdia])),
            ]),
        )
    };
    let moov = make_moov(0);
    let offset = (ftyp.len() + moov.len() + 8) as u32;
    Ok(join(vec![
        ftyp,
        make_moov(offset),
        atom(b"mdat", packets.iter().flatten().copied().collect()),
    ]))
}

#[cfg(test)]
pub(crate) mod test_support {
    pub fn u32_be(bytes: &[u8]) -> u32 {
        u32::from_be_bytes(bytes[..4].try_into().unwrap())
    }

    pub fn box_data<'a>(mut bytes: &'a [u8], tag: &[u8; 4]) -> &'a [u8] {
        loop {
            let size = u32_be(bytes) as usize;
            if &bytes[4..8] == tag {
                return &bytes[8..size];
            }
            bytes = &bytes[size..];
        }
    }

    pub fn sample_table(bytes: &[u8]) -> &[u8] {
        [b"moov", b"trak", b"mdia", b"minf", b"stbl"]
            .iter()
            .fold(bytes, |data, tag| box_data(data, tag))
    }

    pub fn packets(bytes: &[u8]) -> Vec<&[u8]> {
        let sizes = box_data(sample_table(bytes), b"stsz");
        let mut media = box_data(bytes, b"mdat");
        sizes[12..]
            .chunks_exact(4)
            .map(|length| {
                let (packet, rest) = media.split_at(u32_be(length) as usize);
                media = rest;
                packet
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;

    #[test]
    fn sample_tables_preserve_packet_boundaries() {
        for input in [vec![vec![1]], vec![vec![1], vec![2, 3], vec![4, 5, 6]]] {
            let bytes = mux_asr(&input).unwrap();
            assert_eq!(&box_data(&bytes, b"ftyp")[..4], b"mp42");
            assert_eq!(packets(&bytes), input);
            let table = sample_table(&bytes);
            let sizes = box_data(table, b"stsz");
            assert_eq!(u32_be(&sizes[8..]), input.len() as u32);
            let offset = u32_be(&box_data(table, b"stco")[8..]) as usize;
            assert_eq!(&bytes[offset..], input.concat());
            let description = box_data(table, b"stsd");
            let audio = box_data(&description[8..], b"mp4a");
            assert_eq!(u32_be(&audio[24..]), SAMPLE_RATE << 16);
            assert_eq!(&audio[16..18], &1u16.to_be_bytes());
            assert!(audio.windows(4).any(|data| data == [5, 2, ASC[0], ASC[1]]));
        }
        assert!(mux_asr(&[]).is_err());
    }
}
