// Copyright 2026 ansiapps. Neumetik Tracker: MIT licensed, see the LICENSE at the repository root.

//! Files put in a zip as they are (stored, not compressed: WAV files
//! hardly shrink), from the zip format's public description (PKWARE's
//! APPNOTE.TXT).

/// CRC-32, the zip's (IEEE 802.3, reflected).
fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

/// A zip of `files` (name, bytes), stored.
pub fn store(files: &[(String, Vec<u8>)]) -> Vec<u8> {
    // Names are UTF-8 (flag bit 11); every file dated 1 January 1980.
    const UTF8: u16 = 1 << 11;
    const DATE: u16 = 0x21;
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data) in files {
        let offset = out.len() as u32;
        let crc = crc32(data);
        let (size, n) = (data.len() as u32, name.len() as u16);
        out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        for v in [20u16, UTF8, 0, 0, DATE] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        for v in [crc, size, size] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        for v in [n, 0] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(data);

        central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        for v in [20u16, 20, UTF8, 0, 0, DATE] {
            central.extend_from_slice(&v.to_le_bytes());
        }
        for v in [crc, size, size] {
            central.extend_from_slice(&v.to_le_bytes());
        }
        for v in [n, 0, 0, 0, 0] {
            central.extend_from_slice(&v.to_le_bytes());
        }
        central.extend_from_slice(&0u32.to_le_bytes());
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name.as_bytes());
    }
    let at = out.len() as u32;
    let count = files.len() as u16;
    out.extend_from_slice(&central);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    for v in [0u16, 0, count, count] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&(central.len() as u32).to_le_bytes());
    out.extend_from_slice(&at.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_is_the_zips() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn a_zip_reads_back_with_the_trackers_own_unzip() {
        let files = vec![("a.wav".to_string(), b"hello".to_vec()), ("b é.wav".to_string(), vec![7; 300])];
        let z = store(&files);
        assert_eq!(&z[..4], b"PK\x03\x04");
        // neumetik-tracker unzips (modules come zipped): it reads this one back.
        assert!(neumetik_tracker::unpack::is_zip(&z));
        assert_eq!(neumetik_tracker::unpack::unzip(&z), files);
    }
}
