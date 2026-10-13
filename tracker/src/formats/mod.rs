//! Every format read, found by what the file starts with (or, for the
//! formats with no mark of their own, by checks that stand in for one),
//! and the containers modules come in (gzip, zip, Unreal's packages),
//! opened first. A SAUCE record at the end is read and set aside first of
//! all (`crate::sauce`).
//!
//! `docs/tracker-formats.md` lists every tracker format known, and which
//! are read.

pub mod amiga;
pub mod amiga2;
pub mod amiga3;
pub mod common;
pub mod dos;
pub mod extra;
pub mod it;
pub mod micro;
pub mod modlike;
pub mod musicline;
pub mod packed;
pub mod other;
pub mod pc;
pub mod pc2;
pub mod pc3;
pub mod pc4;
pub mod pc5;
pub mod pc6;
pub mod protracker;
pub mod ptk;
pub mod s3m;
pub mod synth;
pub mod xm;

use crate::io::{Error, Result};
use crate::model::Module;

/// A format read: its short name, its name in words, the file
/// extensions it comes with.
pub struct FormatInfo {
    pub id: &'static str,
    pub name: &'static str,
    pub extensions: &'static [&'static str],
}

/// The formats read, roughly by how common they are.
pub const FORMATS: &[FormatInfo] = &[
    FormatInfo { id: "mod", name: "ProTracker, NoiseTracker, StarTrekker, FastTracker and TakeTracker MODs, Oktalyser, Digital Tracker", extensions: &["mod", "nst", "m15", "flt", "wow"] },
    FormatInfo { id: "stk", name: "Ultimate SoundTracker and the 15-sample SoundTrackers", extensions: &["mod", "stk", "m15"] },
    FormatInfo { id: "s3m", name: "Scream Tracker 3", extensions: &["s3m"] },
    FormatInfo { id: "xm", name: "FastTracker 2", extensions: &["xm"] },
    FormatInfo { id: "oxm", name: "OggMod XM", extensions: &["oxm"] },
    FormatInfo { id: "it", name: "Impulse Tracker", extensions: &["it"] },
    FormatInfo { id: "mptm", name: "OpenMPT", extensions: &["mptm"] },
    FormatInfo { id: "umf", name: "Universal Mod Format", extensions: &["umf", "ntt"] },
    FormatInfo { id: "mtm", name: "MultiTracker", extensions: &["mtm"] },
    FormatInfo { id: "669", name: "Composer 669 and UNIS 669", extensions: &["669"] },
    FormatInfo { id: "stm", name: "Scream Tracker 2", extensions: &["stm"] },
    FormatInfo { id: "stx", name: "Scream Tracker Music Interface Kit", extensions: &["stx"] },
    FormatInfo { id: "ptm", name: "PolyTracker", extensions: &["ptm"] },
    FormatInfo { id: "gdm", name: "General DigiMusic", extensions: &["gdm"] },
    FormatInfo { id: "dsm", name: "Digital Sound Interface Kit", extensions: &["dsm"] },
    FormatInfo { id: "ult", name: "UltraTracker", extensions: &["ult"] },
    FormatInfo { id: "far", name: "Farandole Composer", extensions: &["far"] },
    FormatInfo { id: "imf", name: "Imago Orpheus", extensions: &["imf"] },
    FormatInfo { id: "okt", name: "Oktalyzer", extensions: &["okt", "okta"] },
    FormatInfo { id: "med", name: "OctaMED (MMD0 to MMD3, and MED Packer's MMDC) and MED (MED2 to MED4)", extensions: &["med", "mmd0", "mmd1", "mmd2", "mmd3", "mmdc"] },
    FormatInfo { id: "digi", name: "DigiBooster", extensions: &["digi"] },
    FormatInfo { id: "dbm", name: "DigiBooster Pro", extensions: &["dbm"] },
    FormatInfo { id: "mtn", name: "SoundTracker 2.6 and Ice Tracker", extensions: &["mod", "mtn", "ice", "st26"] },
    FormatInfo { id: "sfx", name: "SoundFX", extensions: &["sfx", "sfx2"] },
    FormatInfo { id: "amf", name: "Asylum (Crusader)", extensions: &["amf"] },
    FormatInfo { id: "dtm", name: "Digital Tracker and Digital Home Studio", extensions: &["dtm"] },
    FormatInfo { id: "rtm", name: "Real Tracker", extensions: &["rtm"] },
    FormatInfo { id: "plm", name: "Disorder Tracker 2", extensions: &["plm"] },
    FormatInfo { id: "fnk", name: "Funktracker", extensions: &["fnk"] },
    FormatInfo { id: "c67", name: "Composer 670 (CDFM)", extensions: &["c67"] },
    FormatInfo { id: "xmf", name: "Astroidea XMF", extensions: &["xmf"] },
    FormatInfo { id: "emod", name: "Quadra Composer", extensions: &["emod"] },
    FormatInfo { id: "kris", name: "Chiptracker", extensions: &["mod", "kris"] },
    FormatInfo { id: "ims", name: "Images Music System", extensions: &["ims"] },
    FormatInfo { id: "gmc", name: "Game Music Creator", extensions: &["gmc"] },
    FormatInfo { id: "pt36", name: "ProTracker 3.6", extensions: &["ptm", "mod"] },
    FormatInfo { id: "dstudio", name: "Dynamic Studio", extensions: &["dsm"] },
    FormatInfo { id: "667", name: "Composer 667", extensions: &["667"] },
    FormatInfo { id: "gtk", name: "Graoumf Tracker", extensions: &["gtk"] },
    FormatInfo { id: "dsmi", name: "DSMI Advanced Module Format", extensions: &["amf", "dmf"] },
    FormatInfo { id: "psm", name: "Epic MegaGames MASI", extensions: &["psm"] },
    FormatInfo { id: "liq", name: "Liquid Tracker", extensions: &["liq"] },
    FormatInfo { id: "musx", name: "Archimedes Tracker", extensions: &["musx"] },
    FormatInfo { id: "dsym", name: "Digital Symphony", extensions: &["dsym"] },
    FormatInfo { id: "mgt", name: "Megatracker", extensions: &["mgt"] },
    FormatInfo { id: "mxm", name: "Cubic Tiny XM", extensions: &["mxm"] },
    FormatInfo { id: "uni", name: "MikMod UNITRK", extensions: &["uni"] },
    FormatInfo { id: "ams", name: "Extreme's Tracker", extensions: &["ams"] },
    FormatInfo { id: "ams2", name: "Velvet Studio", extensions: &["ams"] },
    FormatInfo { id: "mdl", name: "Digitrakker", extensions: &["mdl"] },
    FormatInfo { id: "dmf", name: "X-Tracker", extensions: &["dmf"] },
    FormatInfo { id: "stp", name: "SoundTracker Pro II", extensions: &["stp"] },
    FormatInfo { id: "mt2", name: "MadTracker 2", extensions: &["mt2"] },
    FormatInfo { id: "gt2", name: "Graoumf Tracker 2", extensions: &["gt2"] },
    FormatInfo { id: "puma", name: "Pumatracker", extensions: &["puma"] },
    FormatInfo { id: "j2b", name: "Jazz Jackrabbit 2 (Galaxy Sound System)", extensions: &["j2b"] },
    FormatInfo { id: "dtm-dt", name: "DigiTrekker", extensions: &["dtm"] },
    FormatInfo { id: "symmod", name: "Symphonie and Symphonie Pro", extensions: &["symmod"] },
    FormatInfo { id: "ftm", name: "Face The Music", extensions: &["ftm"] },
    FormatInfo { id: "mad", name: "PlayerPro", extensions: &["mad", "madk", "madi", "madh"] },
    FormatInfo { id: "ex", name: "Fashion Tracker", extensions: &["ex"] },
    FormatInfo { id: "dss", name: "Digital Sound Studio", extensions: &["dss"] },
    FormatInfo { id: "kdm", name: "Ken's Digital Music", extensions: &["kdm"] },
    FormatInfo { id: "dsm1", name: "Digital Sound Interface Kit (first format)", extensions: &["dsm"] },
    FormatInfo { id: "mo3", name: "MO3 (Un4seen's packed IT, XM, S3M, MOD and MTM)", extensions: &["mo3"] },
    FormatInfo { id: "skm", name: "Skale Tracker", extensions: &["skm"] },
    FormatInfo { id: "tcb", name: "TCB Tracker", extensions: &["tcb", "mod"] },
    FormatInfo { id: "etx", name: "EasyTrax", extensions: &["etx"] },
    FormatInfo { id: "cba", name: "Chuck Biscuits / Black Artist", extensions: &["cba"] },
    FormatInfo { id: "ss", name: "SoundSmith and MegaTracker", extensions: &["ss", "mtp"] },
    FormatInfo { id: "fmt", name: "FM Tracker", extensions: &["fmt"] },
    FormatInfo { id: "hmn", name: "His Master's Noise", extensions: &["mod", "hmn"] },
    FormatInfo { id: "coco", name: "Coconizer", extensions: &["coco"] },
    FormatInfo { id: "mfp", name: "Magnetic Fields Packer", extensions: &["mfp"] },
    FormatInfo { id: "amc", name: "A.M. Composer", extensions: &["amc"] },
    FormatInfo { id: "ems", name: "Editeur Musical Sequentiel", extensions: &["ems"] },
    FormatInfo { id: "jam", name: "JamCracker", extensions: &["jam"] },
    FormatInfo { id: "abk", name: "AMOS Music Bank", extensions: &["abk"] },
    FormatInfo { id: "ml", name: "Musicline Editor", extensions: &["ml"] },
    FormatInfo { id: "ptk", name: "ProTrekkr and NoiseTrekker", extensions: &["ptk", "ntk"] },
    FormatInfo { id: "ahx", name: "AHX (Abyss' Highest eXperience)", extensions: &["ahx", "thx"] },
    FormatInfo { id: "hvl", name: "HivelyTracker", extensions: &["hvl"] },
    FormatInfo { id: "fc", name: "Future Composer 1.3 and 1.4", extensions: &["fc", "fc13", "fc14", "smod"] },
    FormatInfo { id: "sid1", name: "SidMon 1", extensions: &["sid", "sid1", "sd1", "smn"] },
    FormatInfo { id: "sid2", name: "SidMon 2", extensions: &["sid2", "sd2"] },
    FormatInfo { id: "bp", name: "BP SoundMon 2.0 and 2.2", extensions: &["bp", "bp3", "sndmon"] },
    FormatInfo { id: "dm1", name: "Delta Music 1", extensions: &["dm", "dm1"] },
    FormatInfo { id: "dmu", name: "Digital Mugician 1 to 3", extensions: &["dmu", "mug", "dmu2", "mug2"] },
    FormatInfo { id: "sa", name: "Sonic Arranger", extensions: &["sa", "sonic"] },
    FormatInfo { id: "is", name: "InStereo! 1.0 and 2.0", extensions: &["is", "is20"] },
    FormatInfo { id: "ma", name: "Music Assembler", extensions: &["ma"] },
    FormatInfo { id: "syn", name: "Synthesis 4.0 and 4.2", extensions: &["syn"] },
    FormatInfo { id: "vss", name: "Voodoo Supreme Synthesizer", extensions: &["vss"] },
    FormatInfo { id: "dm2", name: "Delta Music 2", extensions: &["dm2"] },
    FormatInfo { id: "packed", name: "Packed ProTracker modules (40 packers: The Player, NoisePacker, ProPacker, Promizer...)", extensions: packed::PREFIXES },
    FormatInfo { id: "umx", name: "Unreal music packages", extensions: &["umx"] },
    FormatInfo { id: "zip", name: "Modules packed in zip or gzip", extensions: &["zip", "gz", "mdz", "s3z", "xmz", "itz", "mdgz", "s3gz", "xmgz", "itgz"] },
];

/// Reads a module: `name` (the file's name) only helps with the formats
/// that have no mark of their own. Packed files are opened first.
pub fn load(data: &[u8], name: &str) -> Result<Module> {
    load_with(data, name, &|_| None)
}

/// A file beside the module, by its name: the formats that keep their
/// samples in another file (Stonetracker's sample bank, Ken's Digital
/// Music's waves) ask for it. None when there's no such file.
pub type Companion<'a> = &'a dyn Fn(&str) -> Option<Vec<u8>>;

/// `load`, with the files beside the module to hand.
pub fn load_with(data: &[u8], name: &str, companion: Companion) -> Result<Module> {
    with_sauce(data, |data| load_packed(data, name, companion))
}

fn load_packed(data: &[u8], name: &str, companion: Companion) -> Result<Module> {
    if crate::unpack::is_gzip(data) {
        let inner = crate::unpack::gunzip(data).ok_or_else(|| Error::Bad("its gzip packing is damaged".into()))?;
        return load_plain(&inner, name.trim_end_matches(".gz"), companion);
    }
    if crate::unpack::is_zip(data) {
        let files = crate::unpack::unzip(data);
        // The first file in it that's a module; its other files are its companions.
        let inside = |n: &str| files.iter().find(|(f, _)| f.rsplit(['/', '\\']).next().is_some_and(|f| f.eq_ignore_ascii_case(n))).map(|(_, b)| b.clone()).or_else(|| companion(n));
        let mut last = Error::Bad("there's no module in the zip".into());
        for (n, f) in &files {
            match load_plain(f, n, &inside) {
                Ok(m) => return Ok(m),
                Err(e) => last = e,
            }
        }
        return Err(last);
    }
    load_plain(data, name, companion)
}

/// Reads the module before a SAUCE record at the end of `data` with
/// `load`, the record filling in what the module doesn't say; or all of
/// `data`, when there's none.
fn with_sauce(data: &[u8], load: impl FnOnce(&[u8]) -> Result<Module>) -> Result<Module> {
    let Some((sauce, body)) = crate::sauce::read(data) else { return load(data) };
    let mut m = load(&data[..body])?;
    if m.title.is_empty() {
        m.title = sauce.title.clone();
    }
    if m.artist.is_empty() {
        m.artist = sauce.author.clone();
    }
    m.sauce = Some(sauce);
    Ok(m)
}

fn load_plain(data: &[u8], name: &str, companion: Companion) -> Result<Module> {
    with_sauce(data, |data| load_module(data, name, companion))
}

fn load_module(data: &[u8], name: &str, companion: Companion) -> Result<Module> {
    let lower = name.to_ascii_lowercase();
    if it::probe(data) {
        return it::load(data);
    }
    if xm::probe(data) {
        return xm::load(data);
    }
    if s3m::probe(data) {
        return s3m::load(data);
    }
    if crate::umf::probe(data) {
        // A UMF on its own: its samples only when it carries them.
        return crate::umf::read(data, &mut |_| None).map(|l| l.module);
    }
    // Their samples in another file.
    if pc5::probe_kdm(data) {
        return pc5::load_kdm(data, companion);
    }
    if extra::probe_ss(data) {
        return extra::load_ss(data, name, companion);
    }
    if extra::probe_mfp(data, name) {
        return extra::load_mfp(data, name, companion);
    }
    if amiga::probe_med_old(data) {
        return amiga::load_med_old(data, companion);
    }
    let marked: [(fn(&[u8]) -> bool, fn(&[u8]) -> Result<Module>); 86] = [
        (dos::probe_mtm, dos::load_mtm),
        (dos::probe_669, dos::load_669),
        (dos::probe_ptm, dos::load_ptm),
        (dos::probe_gdm, dos::load_gdm),
        (dos::probe_dsm, dos::load_dsm),
        (other::probe_ult, other::load_ult),
        (other::probe_far, other::load_far),
        (other::probe_imf, other::load_imf),
        (other::probe_umx, other::load_umx),
        (amiga::probe_okt, amiga::load_okt),
        (amiga::probe_med, amiga::load_med),
        (amiga::probe_digi, amiga::load_digi),
        (amiga::probe_dbm, amiga::load_dbm),
        (amiga::probe_asylum, amiga::load_asylum),
        (amiga::probe_mtn, amiga::load_mtn),
        (modlike::probe_kris, modlike::load_kris),
        (modlike::probe_emod, modlike::load_emod),
        (modlike::probe_dtm, modlike::load_dtm),
        (pc::probe_rtm, pc::load_rtm),
        (pc::probe_plm, pc::load_plm),
        (pc::probe_fnk, pc::load_fnk),
        (micro::probe_pt36, micro::load_pt36),
        (micro::probe_musx, micro::load_musx),
        (micro::probe_dsym, micro::load_dsym),
        (micro::probe_mgt, micro::load_mgt),
        (pc2::probe_dsm_ds, pc2::load_dsm_ds),
        (pc2::probe_gtk, pc2::load_gtk),
        (pc2::probe_amf, pc2::load_amf),
        (pc2::probe_psm, pc2::load_psm),
        (pc2::probe_psm16, pc2::load_psm16),
        (pc2::probe_liq, pc2::load_liq),
        (pc2::probe_no, pc2::load_no),
        (pc3::probe_mxm, pc3::load_mxm),
        (pc3::probe_uni, pc3::load_uni),
        (pc3::probe_ams, pc3::load_ams),
        (pc3::probe_ams2, pc3::load_ams2),
        (pc3::probe_mdl, pc3::load_mdl),
        (pc3::probe_dmf_xt, pc3::load_dmf_xt),
        (amiga2::probe_stp, amiga2::load_stp),
        (pc4::probe_mt2, pc4::load_mt2),
        (pc4::probe_gt2, pc4::load_gt2),
        (pc5::probe_j2b, pc5::load_j2b),
        (pc5::probe_dtm_dt, pc5::load_dtm_dt),
        (amiga3::probe_symmod, amiga3::load_symmod),
        (amiga3::probe_ftm, amiga3::load_ftm),
        (pc5::probe_madfg, pc5::load_madfg),
        (pc5::probe_madk, pc5::load_madk),
        (amiga3::probe_dss, amiga3::load_dss),
        (pc5::probe_dsm1, pc5::load_dsm1),
        (pc6::probe_mo3, pc6::load_mo3),
        (pc6::probe_skm, pc6::load_skm),
        (extra::probe_tcb, extra::load_tcb),
        (extra::probe_etx, extra::load_etx),
        (extra::probe_cba, extra::load_cba),
        (extra::probe_fmt, extra::load_fmt),
        (extra::probe_hmn, extra::load_hmn),
        (extra::probe_amc, extra::load_amc),
        (extra::probe_abk, extra::load_abk),
        (musicline::probe_ml, musicline::load_ml),
        (ptk::probe_ptk, ptk::load_ptk),
        (synth::probe_ahx, synth::load_ahx),
        (synth::probe_fc, synth::load_fc),
        (synth::probe_sidmon2, synth::load_sidmon2),
        (synth::probe_soundmon, synth::load_soundmon),
        (synth::probe_deltamusic2, synth::load_deltamusic2),
        (synth::probe_deltamusic1, synth::load_deltamusic1),
        (synth::probe_mugician, synth::load_mugician),
        (synth::probe_sonic, synth::load_sonic),
        (synth::probe_instereo, synth::load_instereo),
        (synth::probe_synthesis, synth::load_synthesis),
        (dos::probe_stx, dos::load_stx),
        (dos::probe_stm, dos::load_stm),
        (amiga::probe_sfx, amiga::load_sfx),
        // No marks of their own: checks of their headers stand in.
        (modlike::probe_ims, modlike::load_ims),
        (modlike::probe_gmc, modlike::load_gmc),
        (pc::probe_c67, pc::load_c67),
        (pc::probe_xmf, pc::load_xmf),
        (pc2::probe_667, pc2::load_667),
        (amiga2::probe_puma, amiga2::load_puma),
        (amiga3::probe_fashion, amiga3::load_fashion),
        (extra::probe_coco, extra::load_coco),
        (extra::probe_ems, extra::load_ems),
        (extra::probe_jam, extra::load_jam),
        (synth::probe_sidmon1, synth::load_sidmon1),
        (synth::probe_musicassembler, synth::load_musicassembler),
        (synth::probe_voodoo, synth::load_voodoo),
    ];
    for (probe, load) in marked {
        if probe(data) {
            return load(data);
        }
    }
    // Packed MODs: before the MOD reader, as some keep a MOD's tag.
    if let Some(p) = packed::find(data) {
        return packed::load(data, p);
    }
    if protracker::probe_wow(data, &lower) {
        return protracker::load(data, true);
    }
    if protracker::probe(data) {
        let mut m = protracker::load(data, false)?;
        amiga3::startrekker_am(&mut m, data, name, companion);
        return Ok(m);
    }
    if protracker::probe_15(data) {
        return protracker::load(data, false);
    }
    Err(Error::Unknown)
}

/// The extensions of every format read, for a file picker.
pub fn extensions() -> Vec<&'static str> {
    let mut all: Vec<&'static str> = FORMATS.iter().flat_map(|f| f.extensions.iter().copied()).collect();
    all.sort_unstable();
    all.dedup();
    all
}
