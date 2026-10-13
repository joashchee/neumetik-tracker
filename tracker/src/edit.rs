//! What the editor does to a song beyond typing into it: a block's
//! volumes or effect values interpolated, amplified or faded (Impulse
//! Tracker's Alt+K and Alt+J, OpenMPT's); OpenMPT's paste modes;
//! instruments replaced in a block or swapped through the song (IT's,
//! FastTracker 2's); find and replace (IT's, OpenMPT's); and the
//! song's statistics and cleanup (OpenMPT's Cleanup: unused patterns,
//! samples and instruments taken out, the rest renumbered).

use std::sync::Arc;

use crate::model::*;

/// A rectangle of a pattern: rows from `row`, channels from `channel`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Block {
    pub row: usize,
    pub channel: usize,
    pub rows: usize,
    pub channels: usize,
}

impl Block {
    fn cells(&self, p: &Pattern) -> impl Iterator<Item = (usize, usize)> {
        let (r0, r1) = (self.row.min(p.rows), (self.row + self.rows).min(p.rows));
        let (c0, c1) = (self.channel.min(p.channels), (self.channel + self.channels).min(p.channels));
        (r0..r1).flat_map(move |r| (c0..c1).map(move |c| (r, c)))
    }
}

/// Which part of the cells a block's interpolation moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Column {
    Volume,
    Effect,
}

/// Each channel of the block from its first row's value to its last's,
/// in a straight line (the volume column, or the effect's parameter;
/// the command copied down when the middle rows have none). A channel
/// whose ends don't both have one is left. Returns the cells changed.
pub fn interpolate(p: &mut Pattern, b: Block, what: Column) -> usize {
    if b.rows < 3 {
        return 0;
    }
    let last = (b.row + b.rows - 1).min(p.rows.saturating_sub(1));
    let mut changed = 0;
    for ch in b.channel..(b.channel + b.channels).min(p.channels) {
        let (a, z) = (p.cell(b.row, ch), p.cell(last, ch));
        let ends = match what {
            Column::Volume => {
                let kind = if a.vol != Vol::None { a.vol } else { z.vol };
                if kind == Vol::None || (a.vol != Vol::None && z.vol != Vol::None && a.vol != z.vol) {
                    None
                } else {
                    // A missing end is the cell's note at full volume.
                    let full = if kind == Vol::Panning { 32 } else { 64 };
                    let va = if a.vol != Vol::None { a.volp } else if a.note != 0 { full } else { continue };
                    let vz = if z.vol != Vol::None { z.volp } else if z.note != 0 { full } else { continue };
                    Some((kind as u8, va, vz))
                }
            }
            Column::Effect => (a.fx != Fx::None && a.fx == z.fx).then_some((a.fx as u8, a.param, z.param)),
        };
        let Some((cmd, va, vz)) = ends else { continue };
        let span = (last - b.row) as f32;
        for r in b.row + 1..last {
            let v = (f32::from(va) + (f32::from(vz) - f32::from(va)) * (r - b.row) as f32 / span).round() as u8;
            if let Some(c) = p.cell_mut(r, ch) {
                match what {
                    Column::Volume => {
                        if c.vol == Vol::None || c.vol as u8 == cmd {
                            c.vol = Vol::from_u8(cmd);
                            c.volp = v;
                            changed += 1;
                        }
                    }
                    Column::Effect => {
                        if c.fx == Fx::None || c.fx as u8 == cmd {
                            c.fx = Fx::from_u8(cmd);
                            c.param = v;
                            changed += 1;
                        }
                    }
                }
            }
        }
    }
    changed
}

/// A block's volumes scaled, from `from` percent at its first row to
/// `to` at its last (the same for a plain amplify, different for a fade
/// in or out). A note with no volume column gets one, from its sample's
/// own volume (`default_volume(instrument)`). Returns the cells changed.
pub fn amplify(p: &mut Pattern, b: Block, from: f32, to: f32, default_volume: &dyn Fn(u8) -> u8) -> usize {
    let span = b.rows.saturating_sub(1).max(1) as f32;
    let mut changed = 0;
    let mut last_instr = vec![0u8; p.channels];
    for (r, ch) in b.cells(p).collect::<Vec<_>>() {
        let k = (from + (to - from) * (r - b.row) as f32 / span) / 100.0;
        let Some(c) = p.cell_mut(r, ch) else { continue };
        if c.instr > 0 {
            last_instr[ch] = c.instr;
        }
        let base = match c.vol {
            Vol::Volume => Some(c.volp),
            Vol::None if (NOTE_MIN..=NOTE_MAX).contains(&c.note) => Some(default_volume(if c.instr > 0 { c.instr } else { last_instr[ch] })),
            _ => None,
        };
        if let Some(v) = base {
            c.vol = Vol::Volume;
            c.volp = (f32::from(v) * k).round().clamp(0.0, 64.0) as u8;
            changed += 1;
        }
    }
    changed
}

/// How copied cells go into a pattern (OpenMPT's paste modes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PasteMode {
    /// Over what's there.
    Overwrite,
    /// Into what's there, field by field: only where a field is empty.
    Mix,
    /// Only into empty cells, whole (Impulse Tracker's mix paste).
    MixCells,
    /// Over and over to the pattern's end.
    Flood,
    /// What's there pushed down to make room.
    Insert,
}

/// Puts copied cells (`width` channels a row) at `row`, `channel`.
pub fn paste(p: &mut Pattern, row: usize, channel: usize, width: usize, cells: &[Cell], mode: PasteMode) {
    let width = width.max(1);
    let rows = cells.len().div_ceil(width);
    if rows == 0 {
        return;
    }
    if mode == PasteMode::Insert {
        for ch in channel..(channel + width).min(p.channels) {
            for r in (row..p.rows).rev() {
                let v = if r >= row + rows { p.cell(r - rows, ch) } else { Cell::default() };
                if let Some(c) = p.cell_mut(r, ch) {
                    *c = v;
                }
            }
        }
    }
    let total = if mode == PasteMode::Flood { p.rows.saturating_sub(row) } else { rows };
    for k in 0..total {
        for w in 0..width {
            let Some(&src) = cells.get((k % rows) * width + w) else { continue };
            let Some(c) = p.cell_mut(row + k, channel + w) else { continue };
            match mode {
                PasteMode::Mix => {
                    if c.note == 0 {
                        c.note = src.note;
                    }
                    if c.instr == 0 {
                        c.instr = src.instr;
                    }
                    if c.vol == Vol::None {
                        c.vol = src.vol;
                        c.volp = src.volp;
                    }
                    if c.fx == Fx::None {
                        c.fx = src.fx;
                        c.param = src.param;
                    }
                }
                PasteMode::MixCells => {
                    if c.is_empty() {
                        *c = src;
                    }
                }
                _ => *c = src,
            }
        }
    }
}

/// Every `from` instrument in a block (or the whole song, with no
/// block) made `to`. Returns the cells changed.
pub fn replace_instrument(m: &mut Module, from: u8, to: u8, only: Option<(usize, Block)>) -> usize {
    let mut changed = 0;
    for (i, p) in m.patterns.iter_mut().enumerate() {
        if only.is_some_and(|(pi, _)| pi != i) {
            continue;
        }
        if !p.cells.iter().any(|c| c.instr == from) {
            continue;
        }
        let p = Arc::make_mut(p);
        let block = only.map(|(_, b)| b).unwrap_or(Block { row: 0, channel: 0, rows: p.rows, channels: p.channels });
        for (r, ch) in block.cells(p).collect::<Vec<_>>() {
            if let Some(c) = p.cell_mut(r, ch) {
                if c.instr == from {
                    c.instr = to;
                    changed += 1;
                }
            }
        }
    }
    changed
}

/// Two instruments (or samples, without instruments) swapped: their
/// places in the list and every note that names them, so the song
/// sounds the same.
pub fn swap_instruments(m: &mut Module, a: u8, b: u8) -> Result<(), String> {
    let (ia, ib) = (usize::from(a).wrapping_sub(1), usize::from(b).wrapping_sub(1));
    if a == b {
        return Ok(());
    }
    if m.flags.instruments {
        if ia >= m.instruments.len() || ib >= m.instruments.len() {
            return Err("There's no such instrument.".into());
        }
        m.instruments.swap(ia, ib);
    } else {
        if ia >= m.samples.len() || ib >= m.samples.len() {
            return Err("There's no such sample.".into());
        }
        m.samples.swap(ia, ib);
    }
    for p in &mut m.patterns {
        if !p.cells.iter().any(|c| c.instr == a || c.instr == b) {
            continue;
        }
        for c in &mut Arc::make_mut(p).cells {
            if c.instr == a {
                c.instr = b;
            } else if c.instr == b {
                c.instr = a;
            }
        }
    }
    Ok(())
}

/// What find looks for: each part given must match (a note range, an
/// instrument, a volume command and value, an effect and parameter).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Query {
    /// Notes from and to (`NOTE_OFF` and the rest as themselves).
    pub notes: Option<(u8, u8)>,
    pub instr: Option<u8>,
    pub vol: Option<Vol>,
    pub volp: Option<(u8, u8)>,
    pub fx: Option<Fx>,
    pub param: Option<(u8, u8)>,
}

impl Query {
    pub fn matches(&self, c: &Cell) -> bool {
        self.notes.is_none_or(|(a, b)| c.note >= a && c.note <= b)
            && self.instr.is_none_or(|i| c.instr == i)
            && self.vol.is_none_or(|v| c.vol == v)
            && self.volp.is_none_or(|(a, b)| c.vol != Vol::None && c.volp >= a && c.volp <= b)
            && self.fx.is_none_or(|f| c.fx == f)
            && self.param.is_none_or(|(a, b)| c.fx != Fx::None && c.param >= a && c.param <= b)
    }

    pub fn is_empty(&self) -> bool {
        *self == Query::default()
    }
}

/// What replace puts in a matching cell: each part given is set; a
/// note may instead move by semitones, an instrument the same.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Replacement {
    pub note: Option<u8>,
    pub transpose: i8,
    pub instr: Option<u8>,
    pub vol: Option<(Vol, u8)>,
    pub fx: Option<(Fx, u8)>,
    /// Clears the matching cells instead.
    pub clear: bool,
}

/// Where find looks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    Song,
    Pattern(usize),
    Block(usize, Block),
}

/// Every cell that matches, as (pattern, row, channel), in pattern,
/// row and channel order.
pub fn find(m: &Module, q: &Query, scope: Scope) -> Vec<(usize, usize, usize)> {
    let mut out = Vec::new();
    for (i, p) in m.patterns.iter().enumerate() {
        let block = match scope {
            Scope::Song => Block { row: 0, channel: 0, rows: p.rows, channels: p.channels },
            Scope::Pattern(pi) if pi == i => Block { row: 0, channel: 0, rows: p.rows, channels: p.channels },
            Scope::Block(pi, b) if pi == i => b,
            _ => continue,
        };
        for (r, ch) in block.cells(p) {
            if q.matches(&p.cell(r, ch)) {
                out.push((i, r, ch));
            }
        }
    }
    out
}

/// Every match replaced. Returns how many.
pub fn replace(m: &mut Module, q: &Query, with: &Replacement, scope: Scope) -> usize {
    let found = find(m, q, scope);
    for &(pi, r, ch) in &found {
        let p = Arc::make_mut(&mut m.patterns[pi]);
        let Some(c) = p.cell_mut(r, ch) else { continue };
        if with.clear {
            *c = Cell::default();
            continue;
        }
        if let Some(n) = with.note {
            c.note = n;
        } else if with.transpose != 0 && (NOTE_MIN..=NOTE_MAX).contains(&c.note) {
            c.note = (i16::from(c.note) + i16::from(with.transpose)).clamp(i16::from(NOTE_MIN), i16::from(NOTE_MAX)) as u8;
        }
        if let Some(i) = with.instr {
            c.instr = i;
        }
        if let Some((v, x)) = with.vol {
            c.vol = v;
            c.volp = x;
        }
        if let Some((f, x)) = with.fx {
            c.fx = f;
            c.param = x;
        }
    }
    found.len()
}

/// What OpenMPT's Cleanup would tell: how long the song is, and what it
/// doesn't use.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stats {
    pub seconds: f64,
    pub patterns: usize,
    pub orders: usize,
    pub notes: usize,
    /// Patterns no order (in any order list) plays.
    pub unused_patterns: Vec<usize>,
    /// Patterns with nothing in them.
    pub empty_patterns: Vec<usize>,
    /// 0-based samples no note (or instrument) plays.
    pub unused_samples: Vec<usize>,
    pub unused_instruments: Vec<usize>,
    /// Bytes the unused samples' waveforms take.
    pub unused_bytes: u64,
}

/// The patterns every order list plays.
fn played_patterns(m: &Module) -> Vec<bool> {
    let mut used = vec![false; m.patterns.len()];
    for s in m.all_sequences() {
        for o in s.orders {
            if let Some(u) = used.get_mut(usize::from(o)) {
                *u = true;
            }
        }
    }
    used
}

/// The 1-based instruments (or samples) the played patterns name.
fn named_slots(m: &Module) -> Vec<bool> {
    let played = played_patterns(m);
    let mut named = vec![false; 256];
    for (p, _) in m.patterns.iter().zip(&played).filter(|(_, &u)| u) {
        for c in &p.cells {
            named[usize::from(c.instr)] = true;
        }
    }
    named
}

pub fn stats(m: &Module, seconds: f64) -> Stats {
    let played = played_patterns(m);
    let named = named_slots(m);
    let mut s = Stats { seconds, patterns: m.patterns.len(), orders: m.orders.len(), ..Default::default() };
    for (i, p) in m.patterns.iter().enumerate() {
        if !played[i] {
            s.unused_patterns.push(i);
        }
        if p.cells.iter().all(Cell::is_empty) && p.automation.is_empty() {
            s.empty_patterns.push(i);
        }
        if played[i] {
            s.notes += p.cells.iter().filter(|c| (NOTE_MIN..=NOTE_MAX).contains(&c.note)).count();
        }
    }
    let mut sample_used = vec![false; m.samples.len()];
    if m.flags.instruments {
        for (i, ins) in m.instruments.iter().enumerate() {
            if named[i + 1] {
                for &(_, smp) in &ins.keymap {
                    if let Some(u) = sample_used.get_mut(usize::from(smp).wrapping_sub(1)) {
                        *u = true;
                    }
                }
            } else {
                s.unused_instruments.push(i);
            }
        }
    } else {
        for (i, u) in sample_used.iter_mut().enumerate() {
            *u = named.get(i + 1).copied().unwrap_or(false);
        }
    }
    for (i, used) in sample_used.iter().enumerate() {
        if !used {
            s.unused_samples.push(i);
            s.unused_bytes += m.samples[i].wave.bytes() as u64;
        }
    }
    s
}

/// The patterns no order list plays taken out, and the orders renumbered.
pub fn remove_unused_patterns(m: &mut Module) -> usize {
    let played = played_patterns(m);
    let mut map = vec![0u16; m.patterns.len()];
    let mut kept = Vec::new();
    for (i, p) in m.patterns.iter().enumerate() {
        if played[i] {
            map[i] = kept.len() as u16;
            kept.push(p.clone());
        }
    }
    let removed = m.patterns.len() - kept.len();
    if removed == 0 {
        return 0;
    }
    let renumber = |orders: &mut Vec<u16>| {
        for o in orders.iter_mut() {
            if usize::from(*o) < map.len() {
                *o = map[usize::from(*o)];
            }
        }
    };
    renumber(&mut m.orders);
    for s in &mut m.sequences {
        renumber(&mut s.orders);
    }
    for b in &mut m.bookmarks {
        b.pattern = b.pattern.filter(|&p| played.get(usize::from(p)).copied().unwrap_or(false)).map(|p| map[usize::from(p)]);
    }
    m.patterns = kept;
    if m.patterns.is_empty() {
        m.patterns.push(Arc::new(Pattern::new(64, m.channels.len())));
    }
    removed
}

/// The samples nothing plays emptied (their slots kept, so no number
/// changes), or, with `compact`, taken out and the rest renumbered.
pub fn remove_unused_samples(m: &mut Module, compact: bool) -> usize {
    let unused = stats(m, 0.0).unused_samples;
    if unused.is_empty() {
        return 0;
    }
    if !compact {
        for &i in &unused {
            m.samples[i] = Sample::empty();
        }
        return unused.len();
    }
    let mut map = vec![0u16; m.samples.len() + 1];
    let mut kept = Vec::new();
    for (i, s) in m.samples.drain(..).enumerate() {
        if !unused.contains(&i) {
            kept.push(s);
            map[i + 1] = kept.len() as u16;
        }
    }
    m.samples = kept;
    if m.flags.instruments {
        for ins in &mut m.instruments {
            for k in &mut ins.keymap {
                k.1 = map.get(usize::from(k.1)).copied().unwrap_or(0);
            }
        }
    } else {
        renumber_cells(m, &map);
    }
    unused.len()
}

/// The instruments no note plays taken out, and the rest renumbered.
pub fn remove_unused_instruments(m: &mut Module) -> usize {
    if !m.flags.instruments {
        return 0;
    }
    let unused = stats(m, 0.0).unused_instruments;
    if unused.is_empty() {
        return 0;
    }
    let mut map = vec![0u16; m.instruments.len() + 1];
    let mut kept = Vec::new();
    for (i, x) in m.instruments.drain(..).enumerate() {
        if !unused.contains(&i) {
            kept.push(x);
            map[i + 1] = kept.len() as u16;
        }
    }
    m.instruments = kept;
    renumber_cells(m, &map);
    unused.len()
}

/// Every cell's instrument as `map` has it (1-based, 0 none).
fn renumber_cells(m: &mut Module, map: &[u16]) {
    for p in &mut m.patterns {
        if !p.cells.iter().any(|c| c.instr != 0) {
            continue;
        }
        for c in &mut Arc::make_mut(p).cells {
            if c.instr != 0 {
                c.instr = map.get(usize::from(c.instr)).copied().unwrap_or(0).min(255) as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pat(cells: &[(usize, usize, Cell)]) -> Pattern {
        let mut p = Pattern::new(16, 2);
        for &(r, c, cell) in cells {
            *p.cell_mut(r, c).unwrap() = cell;
        }
        p
    }

    #[test]
    fn interpolation_and_amplify_fill_a_block() {
        let mut p = pat(&[(0, 0, Cell { vol: Vol::Volume, volp: 0, ..Default::default() }), (4, 0, Cell { vol: Vol::Volume, volp: 64, ..Default::default() })]);
        assert_eq!(interpolate(&mut p, Block { row: 0, channel: 0, rows: 5, channels: 1 }, Column::Volume), 3);
        assert_eq!((1..4).map(|r| p.cell(r, 0).volp).collect::<Vec<_>>(), vec![16, 32, 48]);
        let mut p = pat(&[(0, 1, Cell { fx: Fx::PortaUp, param: 0x10, ..Default::default() }), (2, 1, Cell { fx: Fx::PortaUp, param: 0x30, ..Default::default() })]);
        interpolate(&mut p, Block { row: 0, channel: 1, rows: 3, channels: 1 }, Column::Effect);
        assert_eq!(p.cell(1, 1).param, 0x20);
        // A fade out from a note with no volume column: its sample's 48.
        let mut p = pat(&[(0, 0, Cell { note: 61, instr: 1, ..Default::default() }), (3, 0, Cell { note: 61, ..Default::default() })]);
        amplify(&mut p, Block { row: 0, channel: 0, rows: 4, channels: 1 }, 100.0, 0.0, &|_| 48);
        assert_eq!((p.cell(0, 0).volp, p.cell(3, 0).volp), (48, 0));
    }

    #[test]
    fn paste_modes() {
        let src = [Cell { note: 61, ..Default::default() }, Cell { instr: 2, ..Default::default() }];
        let mut p = pat(&[(0, 0, Cell { instr: 1, ..Default::default() }), (1, 0, Cell { note: 50, ..Default::default() })]);
        paste(&mut p, 0, 0, 1, &src, PasteMode::Mix);
        assert_eq!((p.cell(0, 0).note, p.cell(0, 0).instr), (61, 1));
        assert_eq!((p.cell(1, 0).note, p.cell(1, 0).instr), (50, 2));
        let mut p = pat(&[(0, 0, Cell { note: 40, ..Default::default() })]);
        paste(&mut p, 0, 0, 1, &src, PasteMode::Insert);
        assert_eq!(p.cell(2, 0).note, 40);
        let mut p = pat(&[]);
        paste(&mut p, 4, 0, 1, &src, PasteMode::Flood);
        assert_eq!(p.cell(14, 0).note, 61);
        assert_eq!(p.cell(15, 0).instr, 2);
    }

    #[test]
    fn find_replace_swap_and_cleanup() {
        let mut m = Module::new(2);
        m.samples = vec![Sample::empty(), Sample::empty(), Sample::empty()];
        m.patterns = vec![Arc::new(pat(&[(0, 0, Cell { note: 61, instr: 1, ..Default::default() }), (8, 1, Cell { note: 65, instr: 3, ..Default::default() })])), Arc::new(Pattern::new(64, 2))];
        let q = Query { notes: Some((60, 62)), ..Default::default() };
        assert_eq!(find(&m, &q, Scope::Song), vec![(0, 0, 0)]);
        assert_eq!(replace(&mut m, &q, &Replacement { transpose: 12, ..Default::default() }, Scope::Song), 1);
        assert_eq!(m.patterns[0].cell(0, 0).note, 73);
        swap_instruments(&mut m, 1, 3).unwrap();
        assert_eq!((m.patterns[0].cell(0, 0).instr, m.patterns[0].cell(8, 1).instr), (3, 1));
        let s = stats(&m, 0.0);
        assert_eq!(s.unused_patterns, vec![1]);
        assert_eq!(s.unused_samples, vec![1]);
        assert_eq!(remove_unused_patterns(&mut m), 1);
        assert_eq!(remove_unused_samples(&mut m, true), 1);
        assert_eq!(m.samples.len(), 2);
        assert_eq!((m.patterns[0].cell(0, 0).instr, m.patterns[0].cell(8, 1).instr), (2, 1));
    }
}
