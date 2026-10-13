//! What several loaders share: Amiga periods as notes, MOD's effects
//! (which XM, MTM and many others reuse), finetune as a rate.

use crate::model::{Fx, NOTE_MAX, NOTE_MIN};

/// The Amiga period of middle C (C-5 here, ProTracker's C-2).
pub const C5_PERIOD: f64 = 428.0;

/// The note (1-based) nearest an Amiga period; None for no period.
pub fn period_to_note(period: u16) -> Option<u8> {
    if period == 0 {
        return None;
    }
    let semis = 12.0 * (C5_PERIOD / f64::from(period)).log2();
    let note = 61.0 + semis.round();
    Some(note.clamp(f64::from(NOTE_MIN), f64::from(NOTE_MAX)) as u8)
}

/// A rate for C-5 from a MOD finetune nibble (-8 to 7, eighths of a
/// semitone).
pub fn finetune_c5(nibble: u8) -> u32 {
    let ft = ((nibble & 0x0F) as i8) << 4 >> 4;
    (8363.0 * 2f64.powf(f64::from(ft) / 96.0)).round() as u32
}

/// A rate for C-5 from a semitone offset and a finetune in 128ths of a
/// semitone (XM's relative note and finetune).
pub fn relative_c5(relative: i8, finetune: i8) -> u32 {
    let semis = f64::from(relative) + f64::from(finetune) / 128.0;
    (8363.0 * 2f64.powf(semis / 12.0)).round().clamp(1.0, 2_000_000.0) as u32
}

/// MOD's effect command and parameter in the shared command set.
/// `xm` reads FastTracker 2's extra letters (G to X) too.
pub fn mod_effect(cmd: u8, param: u8, xm: bool) -> (Fx, u8) {
    let fx = match cmd {
        0x0 if param != 0 => Fx::Arpeggio,
        0x0 => Fx::None,
        0x1 => Fx::PortaUp,
        0x2 => Fx::PortaDown,
        0x3 => Fx::TonePorta,
        0x4 => Fx::Vibrato,
        0x5 => Fx::TonePortaVol,
        0x6 => Fx::VibratoVol,
        0x7 => Fx::Tremolo,
        0x8 => Fx::Panning,
        0x9 => Fx::Offset,
        0xA => Fx::VolSlide,
        0xB => Fx::PositionJump,
        0xC => return (Fx::Volume, param.min(64)),
        // The row is written in decimal digits, as hex.
        0xD => return (Fx::PatternBreak, ((param >> 4) * 10 + (param & 0x0F)).min(255)),
        0xE => Fx::ModEx,
        0xF if param == 0 => Fx::None,
        0xF if param < 0x20 => Fx::Speed,
        0xF => Fx::Tempo,
        // FastTracker 2's: G global volume, H its slide, K key off,
        // L envelope position, P panning slide, R retrigger, T tremor,
        // X extra-fine porta.
        0x10 if xm => return (Fx::GlobalVolume, param.min(64) * 2),
        0x11 if xm => Fx::GlobalVolSlide,
        0x14 if xm => Fx::KeyOff,
        0x15 if xm => Fx::EnvelopePosition,
        0x19 if xm => Fx::PanningSlide,
        0x1B if xm => Fx::Retrig,
        0x1D if xm => Fx::Tremor,
        0x21 if xm => Fx::ExtraFinePorta,
        _ => Fx::None,
    };
    (fx, if fx == Fx::None { 0 } else { param })
}

/// S3M's and IT's effect letter (1 is A) and parameter in the shared
/// command set. `it` reads Impulse Tracker's own (M, N, V's range...).
pub fn s3m_effect(cmd: u8, param: u8, it: bool) -> (Fx, u8) {
    let fx = match cmd {
        1 => Fx::Speed,
        2 => Fx::PositionJump,
        // S3M writes the row as decimal digits in hex; IT as the number.
        3 if !it => return (Fx::PatternBreak, (param >> 4) * 10 + (param & 0x0F)),
        3 => Fx::PatternBreak,
        4 => Fx::VolSlide,
        5 => Fx::PortaDown,
        6 => Fx::PortaUp,
        7 => Fx::TonePorta,
        8 => Fx::Vibrato,
        9 => Fx::Tremor,
        10 => Fx::Arpeggio,
        11 => Fx::VibratoVol,
        12 => Fx::TonePortaVol,
        13 => Fx::ChannelVolume,
        14 => Fx::ChannelVolSlide,
        15 => Fx::Offset,
        16 => Fx::PanningSlide,
        17 => Fx::Retrig,
        18 => Fx::Tremolo,
        19 => Fx::S3mEx,
        20 => Fx::Tempo,
        21 => Fx::FineVibrato,
        // S3M's global volume is 0 to 64.
        22 if !it => return (Fx::GlobalVolume, param.min(64) * 2),
        22 => return (Fx::GlobalVolume, param.min(128)),
        23 => Fx::GlobalVolSlide,
        // S3M pans 0 to 80 (A4 is surround); IT 0 to FF.
        24 if !it => return if param == 0xA4 { (Fx::S3mEx, 0x91) } else { (Fx::Panning, (u16::from(param.min(0x80)) * 255 / 128) as u8) },
        24 => Fx::Panning,
        25 => Fx::Panbrello,
        26 => Fx::Midi,
        27 => Fx::SmoothMidi,
        _ => Fx::None,
    };
    (fx, if fx == Fx::None { 0 } else { param })
}

/// A MOD extended command (Exy) as Scream Tracker's and Impulse Tracker's
/// special command (Sxy) or the slide it stands for, for formats that
/// play by their rules but write MOD's.
pub fn mod_ex_to_s3m(param: u8) -> (Fx, u8) {
    let x = param & 0x0F;
    match param >> 4 {
        0x0 => (Fx::S3mEx, x),
        0x1 => (Fx::PortaUp, 0xF0 | x),
        0x2 => (Fx::PortaDown, 0xF0 | x),
        0x3 => (Fx::S3mEx, 0x10 | x),
        0x4 => (Fx::S3mEx, 0x30 | x),
        0x5 => (Fx::S3mEx, 0x20 | x),
        0x6 => (Fx::S3mEx, 0xB0 | x),
        0x7 => (Fx::S3mEx, 0x40 | x),
        0x8 => (Fx::S3mEx, 0x80 | x),
        0x9 => (Fx::Retrig, x),
        0xA if x > 0 => (Fx::VolSlide, x << 4 | 0x0F),
        0xB if x > 0 => (Fx::VolSlide, 0xF0 | x),
        0xC => (Fx::S3mEx, 0xC0 | x),
        0xD => (Fx::S3mEx, 0xD0 | x),
        0xE => (Fx::S3mEx, 0xE0 | x),
        0xF => (Fx::S3mEx, 0xF0 | x),
        _ => (Fx::None, 0),
    }
}

/// Old Amiga panning: channels go left, right, right, left.
pub fn amiga_pan(channel: usize) -> u8 {
    if matches!(channel % 4, 0 | 3) {
        0
    } else {
        255
    }
}

/// Whether a name looks like text a tracker would write: printable, or
/// padding.
pub fn plausible_text(b: &[u8]) -> bool {
    b.iter().all(|&c| c == 0 || (32..127).contains(&c) || c >= 128)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn periods_map_to_notes() {
        assert_eq!(period_to_note(428), Some(61));
        assert_eq!(period_to_note(856), Some(49));
        assert_eq!(period_to_note(113), Some(84));
        assert_eq!(period_to_note(404), Some(62));
        assert_eq!(period_to_note(0), None);
    }

    #[test]
    fn finetunes_are_eighths_of_a_semitone() {
        assert_eq!(finetune_c5(0), 8363);
        assert!(finetune_c5(7) > 8363 && finetune_c5(8) < 8363);
        assert_eq!(relative_c5(12, 0), 16726);
    }
}
