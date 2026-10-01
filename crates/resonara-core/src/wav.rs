use std::{io::Read, path::Path};

pub(crate) fn open(path: &Path) -> crate::Result<hound::WavReader<Box<dyn Read>>> {
    let input: Box<dyn Read> = Box::new(std::io::BufReader::new(std::fs::File::open(path)?));
    match hound::WavReader::new(input) {
        Ok(reader) => Ok(reader),
        Err(error) => {
            if !matches!(
                error,
                hound::Error::FormatError("data chunk length is not a multiple of sample size")
            ) {
                return Err(error.into());
            }
            let mut bytes = std::fs::read(path)?;
            if !exclude_counted_padding(&mut bytes) {
                return Err(error.into());
            }
            let input: Box<dyn Read> = Box::new(std::io::Cursor::new(bytes));
            Ok(hound::WavReader::new(input)?)
        }
    }
}

// Some Logic Pro 24-bit mono WAVs include the RIFF alignment byte in the
// declared data size. Only correct a single zero pad after an odd number of
// complete samples. Patch the in-memory header; never alter the source file.
fn exclude_counted_padding(bytes: &mut [u8]) -> bool {
    if bytes.get(..4) != Some(b"RIFF") || bytes.get(8..12) != Some(b"WAVE") {
        return false;
    }
    let mut offset = 12usize;
    let mut mono_pcm24 = false;
    while let Some(header) = bytes.get(offset..).and_then(|b| b.get(..8)) {
        let length = u32::from_le_bytes(header[4..8].try_into().unwrap()) as usize;
        let start = offset + 8;
        let Some(end) = start.checked_add(length).filter(|&end| end <= bytes.len()) else {
            return false;
        };
        match &header[..4] {
            b"fmt " => {
                let fmt = &bytes[start..end];
                mono_pcm24 =
                    fmt.len() >= 16 && fmt[..4] == [1, 0, 1, 0] && fmt[12..16] == [3, 0, 24, 0];
            }
            b"data" => {
                if !mono_pcm24 || length % 6 != 4 || bytes[end - 1] != 0 {
                    return false;
                }
                bytes[offset + 4..start].copy_from_slice(&((length - 1) as u32).to_le_bytes());
                return true;
            }
            _ => {}
        }
        let Some(next) = end.checked_add(length & 1) else {
            return false;
        };
        offset = next;
    }
    false
}
