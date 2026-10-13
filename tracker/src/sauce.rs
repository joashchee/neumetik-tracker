// Copyright 2026 ansiapps. Neumetik Tracker: MIT licensed, see the LICENSE at the repository root.

//! SAUCE (the Standard Architecture for Universal Comment Extensions,
//! ACiD Productions, 1994): 128 bytes at a file's end, and an optional
//! comment block before them, that the BBS scene put on its files,
//! modules among them (data type 4, audio: MOD, S3M, XM, IT, 669, STM,
//! MTM, FAR, ULT, AMF, DMF, OKT...). It names the song, the artist,
//! their group and the date.
//!
//! [`crate::load`] reads it, reads the module from the bytes before it
//! (so the record is never taken for sample data or upsets a format's
//! size checks), and keeps it as [`crate::Module::sauce`]. The record is
//! read the way Diskette's `ansiart.rs` reads ANSI art's.
//!
//! ```text
//! (EOF, 0x1A, usually, before the comments or the record)
//! COMNT               5 bytes, then 64 bytes a comment line
//! SAUCE               5 bytes
//! version             2 bytes, "00"
//! title               35 bytes, CP437, space-padded
//! author              20 bytes
//! group               20 bytes
//! date                8 bytes, CCYYMMDD
//! file size           u32
//! data type           u8 (4: audio)
//! file type           u8
//! type info 1 to 4    u16 each
//! comment lines       u8
//! flags               u8
//! type info string    22 bytes
//! ```

use crate::io::text;

/// The size of the record itself.
const RECORD: usize = 128;
/// Each comment line.
const LINE: usize = 64;

/// What a SAUCE record says about a file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Sauce {
    pub title: String,
    pub author: String,
    pub group: String,
    /// `CCYYMMDD`, as written.
    pub date: String,
    pub data_type: u8,
    pub file_type: u8,
    pub comments: Vec<String>,
}

impl Sauce {
    /// The date as `YYYY-MM-DD` when it is one, else as written.
    pub fn date_text(&self) -> String {
        let d = &self.date;
        if d.len() == 8 && d.bytes().all(|b| b.is_ascii_digit()) {
            format!("{}-{}-{}", &d[0..4], &d[4..6], &d[6..8])
        } else {
            d.clone()
        }
    }
}

/// The SAUCE record at the end of `data`, if there is one, and how many
/// bytes come before it (and its comments, and the EOF mark before them).
pub fn read(data: &[u8]) -> Option<(Sauce, usize)> {
    let at = data.len().checked_sub(RECORD)?;
    let r = &data[at..];
    if !r.starts_with(b"SAUCE") {
        return None;
    }
    let mut body = at;
    let lines = usize::from(r[104]);
    let block = 5 + lines * LINE;
    let mut comments = Vec::new();
    if lines > 0 && body >= block && data[body - block..].starts_with(b"COMNT") {
        comments = data[body - block + 5..body].chunks(LINE).map(text).filter(|l| !l.is_empty()).collect();
        body -= block;
    }
    if body > 0 && data[body - 1] == 0x1A {
        body -= 1;
    }
    let sauce = Sauce {
        title: text(&r[7..42]),
        author: text(&r[42..62]),
        group: text(&r[62..82]),
        date: text(&r[82..90]),
        data_type: r[94],
        file_type: r[95],
        comments,
    };
    Some((sauce, body))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(title: &str, author: &str, group: &str, date: &str, comments: &[&str]) -> Vec<u8> {
        let mut out = vec![0x1A];
        if !comments.is_empty() {
            out.extend(b"COMNT");
            for c in comments {
                let mut line = c.as_bytes().to_vec();
                line.resize(LINE, b' ');
                out.extend(line);
            }
        }
        let mut r = vec![b' '; RECORD];
        r[..7].copy_from_slice(b"SAUCE00");
        for (s, at, n) in [(title, 7, 35), (author, 42, 20), (group, 62, 20), (date, 82, 8)] {
            r[at..at + n].fill(b' ');
            r[at..at + s.len()].copy_from_slice(s.as_bytes());
        }
        r[90..104].fill(0);
        r[94] = 4;
        r[104] = comments.len() as u8;
        r[105..].fill(0);
        out.extend(r);
        out
    }

    #[test]
    fn reads_the_record_and_where_the_file_ends() {
        let mut file = b"module bytes".to_vec();
        file.extend(record("Space Debris", "Captain", "Image", "19910514", &["Greets to all", ""]));
        let (s, body) = read(&file).unwrap();
        assert_eq!(&file[..body], b"module bytes");
        assert_eq!((s.title.as_str(), s.author.as_str(), s.group.as_str()), ("Space Debris", "Captain", "Image"));
        assert_eq!(s.date_text(), "1991-05-14");
        assert_eq!(s.data_type, 4);
        assert_eq!(s.comments, vec!["Greets to all".to_string()]);
    }

    #[test]
    fn a_module_reads_the_same_with_one() {
        use crate::export::{export, Format};
        use crate::model::*;
        use std::sync::Arc;
        let mut m = Module::new(4);
        m.title = "with sauce".into();
        let pcm: Vec<i16> = (0..2000).map(|i| (((i as f32) * 0.1).sin() * 16000.0) as i16).collect();
        m.samples = vec![Sample { name: "sine".into(), wave: Arc::new(Wave::new(16, 1, pcm)), c5: 8363, ..Sample::empty() }];
        let mut p = Pattern::new(64, 4);
        *p.cell_mut(0, 0).unwrap() = Cell { note: 61, instr: 1, ..Default::default() };
        m.patterns = vec![Arc::new(p)];
        m.orders = vec![0];
        m.tidy();
        for f in [Format::Mod, Format::S3m, Format::Xm, Format::It] {
            let plain = export(&m, f).bytes;
            let mut signed = plain.clone();
            signed.extend(record("Ignored title", "Someone", "A Group", "19950101", &["hello"]));
            let a = crate::load(&plain, "song").unwrap();
            let b = crate::load(&signed, "song").unwrap();
            assert_eq!(a.samples, b.samples, "{f:?}");
            assert_eq!(a.patterns, b.patterns, "{f:?}");
            assert_eq!(b.title, "with sauce", "{f:?}: the module's own title stays");
            assert_eq!(b.artist, "Someone", "{f:?}");
            assert_eq!(b.sauce.as_ref().map(|s| s.group.as_str()), Some("A Group"));
            assert_eq!(a.sauce, None);
        }
    }

    #[test]
    fn no_record_and_odd_ones() {
        assert_eq!(read(b"short"), None);
        assert_eq!(read(&vec![0u8; 300]), None);
        // A comment count with no comment block: the record alone.
        let mut file = b"x".to_vec();
        let mut r = record("T", "", "", "1993", &[]);
        let n = r.len();
        r[n - RECORD + 104] = 9;
        file.extend(r);
        let (s, body) = read(&file).unwrap();
        assert_eq!(body, 1);
        assert!(s.comments.is_empty());
        assert_eq!(s.date_text(), "1993");
        // Just the record, nothing before it.
        let r = &record("", "", "", "", &[])[1..];
        assert_eq!(read(r).unwrap().1, 0);
    }
}
