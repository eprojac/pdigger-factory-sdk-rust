//! Looks (spec 4.6.2): how well one stone's look agrees with another's, the same number in every
//! SDK. The sums are exact integers; the only floating-point steps are one product under a square
//! root and one division.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;

use crate::models::StoneLook;

/// Looks are compared at whole-cell shifts of up to this many cells each way ...
pub const LOOK_SHIFT: i64 = 3;
/// ... where at least this many cells (and half the smaller look's read cells) pair.
pub const LOOK_MIN: i64 = 24;
/// Faces whose sides differ by less than this share are also compared a quarter turn round.
pub const LOOK_SQUARE: f64 = 0.12;
/// The most cells (`nu` x `nv`) a look may have: one of more is not a look, so every sum stays
/// exact.
pub const LOOK_CELLS: i64 = 4096;

type Rows = Vec<Vec<i64>>;

/// A look's cells as rows: values (0 where not read), read marks (1 or 0) and squared values.
#[derive(Debug, Clone)]
struct Grid {
    v: Rows,
    m: Rows,
    vv: Rows,
}

/// A look the rules can read.
#[derive(Debug)]
pub(crate) struct Cells {
    square: bool,
    n: i64,
    grid: Grid,
}

/// The cells of a look; `None` when the rules cannot read it (another version, more than
/// [`LOOK_CELLS`] cells, cells that are not canonical base64 of `nu` x `nv` bytes).
pub(crate) fn read_look(lk: &StoneLook) -> Option<Cells> {
    // each side is bounded before the product, which can then not overflow
    if lk.v != 1 || lk.nu < 1 || lk.nv < 1 || lk.nu > LOOK_CELLS || lk.nv > LOOK_CELLS || lk.nu * lk.nv > LOOK_CELLS {
        return None;
    }
    if lk.d.len() as i64 != 4 * ((lk.nu * lk.nv + 2) / 3) {
        return None; // not the length of nu x nv bytes: not decoded
    }
    #[cfg(test)]
    tests::DECODED.with(|n| n.set(n.get() + 1));
    let raw = STANDARD.decode(lk.d.as_bytes()).ok()?;
    if STANDARD.encode(&raw) != lk.d || raw.len() as i64 != lk.nu * lk.nv {
        return None;
    }
    let (nu, nv) = (lk.nu as usize, lk.nv as usize);
    let mut grid = Grid { v: Vec::with_capacity(nv), m: Vec::with_capacity(nv), vv: Vec::with_capacity(nv) };
    let mut n = 0;
    for j in 0..nv {
        let (mut v, mut m, mut vv) = (vec![0; nu], vec![0; nu], vec![0; nu]);
        for i in 0..nu {
            let x = raw[j * nu + i] as i8 as i64;
            if x != -128 {
                v[i] = x;
                m[i] = 1;
                vv[i] = x * x;
                n += 1;
            }
        }
        grid.v.push(v);
        grid.m.push(m);
        grid.vv.push(vv);
    }
    let (big, small) = (lk.at.l.max(lk.at.s), lk.at.l.min(lk.at.s));
    Some(Cells { square: big > 0.0 && (big - small) / big < LOOK_SQUARE, n, grid })
}

fn turned(rows: &Rows, q: u8) -> Rows {
    let (nv, nu) = (rows.len(), rows[0].len());
    match q {
        2 => (0..nv).map(|j| (0..nu).map(|i| rows[nv - 1 - j][nu - 1 - i]).collect()).collect(),
        1 => (0..nu).map(|r| (0..nv).map(|c| rows[c][nu - 1 - r]).collect()).collect(),
        _ => (0..nu).map(|r| (0..nv).map(|c| rows[nv - 1 - c][r]).collect()).collect(),
    }
}

impl Grid {
    fn turned(&self, q: u8) -> Grid {
        Grid { v: turned(&self.v, q), m: turned(&self.m, q), vv: turned(&self.vv, q) }
    }
}

/// The six integer sums of a placement: pairs, both sides' values, their squares, their products.
#[derive(Default)]
struct Sums {
    n: i64,
    sa: i64,
    sb: i64,
    saa: i64,
    sbb: i64,
    sab: i64,
}

impl Sums {
    /// The normalised cross-correlation; `None` below `need` pairs or for a flat side.
    fn zncc(&self, need: i64) -> Option<f64> {
        if self.n < need {
            return None;
        }
        let da = self.n * self.saa - self.sa * self.sa;
        let db = self.n * self.sbb - self.sb * self.sb;
        if da <= 0 || db <= 0 {
            return None;
        }
        Some((self.n * self.sab - self.sa * self.sb) as f64 / ((da as f64) * (db as f64)).sqrt())
    }
}

/// The sums of a's cells against b's, a's cell (0, 0) on b's cell (ov, ou).
fn sums(a: &Grid, b: &Grid, ov: i64, ou: i64) -> Sums {
    let (nva, nua, nvb, nub) = (a.v.len() as i64, a.v[0].len() as i64, b.v.len() as i64, b.v[0].len() as i64);
    let mut s = Sums::default();
    let (i0, i1) = (0.max(-ou), nua.min(nub - ou));
    for j in 0.max(-ov)..nva.min(nvb - ov) {
        let (ja, jb) = (j as usize, (j + ov) as usize);
        for i in i0..i1 {
            let (ia, ib) = (i as usize, (i + ou) as usize);
            s.n += a.m[ja][ia] * b.m[jb][ib];
            s.sa += a.v[ja][ia] * b.m[jb][ib];
            s.sb += b.v[jb][ib] * a.m[ja][ia];
            s.saa += a.vv[ja][ia] * b.m[jb][ib];
            s.sbb += b.vv[jb][ib] * a.m[ja][ia];
            s.sab += a.v[ja][ia] * b.v[jb][ib];
        }
    }
    s
}

fn half_span(h: i64) -> &'static [i64] {
    match h.signum() {
        1 => &[0, 1],
        -1 => &[-1, 0],
        _ => &[0],
    }
}

/// b sampled half a cell from (ov, ou) toward (hv, hu): each of nv x nu cells the sum of the 2 or
/// 4 cells around, read only when all of them are.
fn half_grid(b: &Grid, nv: usize, nu: usize, ov: i64, ou: i64, hv: i64, hu: i64) -> Grid {
    let (nvb, nub) = (b.v.len() as i64, b.v[0].len() as i64);
    let mut h = Grid { v: vec![vec![0; nu]; nv], m: vec![vec![0; nu]; nv], vv: vec![vec![0; nu]; nv] };
    for j in 0..nv {
        for i in 0..nu {
            let (mut s, mut ok) = (0, true);
            for a in half_span(hv) {
                for c in half_span(hu) {
                    let (y, x) = (j as i64 + ov + a, i as i64 + ou + c);
                    if y >= 0 && y < nvb && x >= 0 && x < nub && b.m[y as usize][x as usize] == 1 {
                        s += b.v[y as usize][x as usize];
                    } else {
                        ok = false;
                    }
                }
            }
            if ok {
                h.v[j][i] = s;
                h.m[j][i] = 1;
                h.vv[j][i] = s * s;
            }
        }
    }
    h
}

/// [`look_pairing`] of two read looks: the score and the pair count of the placement that gave it.
pub(crate) fn pair_cells(a: &Cells, b: &Cells) -> Option<(f64, i64)> {
    let need = LOOK_MIN.max((a.n.min(b.n) + 1) / 2);
    let (nva, nua) = (a.grid.v.len(), a.grid.v[0].len());
    let mut turns = vec![b.grid.clone(), b.grid.turned(2)];
    if a.square && b.square {
        turns.push(b.grid.turned(1));
        turns.push(b.grid.turned(3));
    }
    let mut best: Option<(f64, i64)> = None;
    for t in &turns {
        // the floor of the centre offset
        let ov = (t.v.len() as i64 - nva as i64).div_euclid(2);
        let ou = (t.v[0].len() as i64 - nua as i64).div_euclid(2);
        let mut top: Option<(f64, i64, i64, i64)> = None;
        for dv in -LOOK_SHIFT..=LOOK_SHIFT {
            for du in -LOOK_SHIFT..=LOOK_SHIFT {
                let s = sums(&a.grid, t, ov + dv, ou + du);
                if let Some(z) = s.zncc(need)
                    && top.is_none_or(|(b, ..)| z > b)
                {
                    top = Some((z, s.n, dv, du));
                }
            }
        }
        let Some((mut z0, mut n0, tv, tu)) = top else { continue };
        // half-cell shifts: each cell the sum of the 2 or 4 cells of b around the half position, all read
        for hv in -1..=1 {
            for hu in -1..=1 {
                if hu == 0 && hv == 0 {
                    continue;
                }
                let h = half_grid(t, nva, nua, ov + tv, ou + tu, hv, hu);
                let s = sums(&a.grid, &h, 0, 0);
                if let Some(z) = s.zncc(need)
                    && z > z0
                {
                    (z0, n0) = (z, s.n);
                }
            }
        }
        if best.is_none_or(|(b, _)| z0 > b) {
            best = Some((z0, n0));
        }
    }
    best
}

/// The cells of a look the tray rules use (spec 4.6.3): one [`read_look`] reads, with pitch `sp`
/// 2 and scale `q` 32, at least 3 x 3 cells, both sides of its face above 0 and at least
/// [`LOOK_MIN`] cells read.
pub(crate) fn tray_cells(lk: &StoneLook) -> Option<Cells> {
    let c = read_look(lk)?;
    let fits = lk.sp == 2.0 && lk.q == 32.0 && lk.nu >= 3 && lk.nv >= 3 && lk.at.l > 0.0 && lk.at.s > 0.0;
    (fits && c.n >= LOOK_MIN).then_some(c)
}

/// A score as the integer the tray rules compare (spec 4.6.2): `floor(score * 1000 + 0.5)`, from
/// -1000 to 1000.
pub fn look_permille(score: f64) -> i64 {
    (score * 1000.0 + 0.5).floor() as i64
}

/// The tray rules use this look (spec 4.6.3): [`look_score`] reads it, with pitch `sp` 2 and scale
/// `q` 32, at least 3 x 3 cells, both sides of its face above 0 and at least 24 cells read.
pub fn usable_look(lk: &StoneLook) -> bool {
    tray_cells(lk).is_some()
}

/// How well look `a` agrees with look `b` (spec 4.6.2): the best normalised cross-correlation of
/// a's read cells with b's over b's turns (half a turn round, and a quarter and three quarters
/// round when both faces are near square), whole-cell shifts of up to 3 cells and then half-cell
/// shifts around each turn's best, from exact integer sums: the same number in every SDK. 1 is the
/// same texture; other stones score about 0.2 to 0.4. `None` when either is not a look the rules
/// can read or no placement pairs enough cells (24, and half the smaller look's read cells).
pub fn look_score(a: &StoneLook, b: &StoneLook) -> Option<f64> {
    look_pairing(a, b).map(|(score, _)| score)
}

/// [`look_score`] with the pair count of the placement that gave the score (spec 4.6.2): `None`
/// exactly when [`look_score`] is.
pub fn look_pairing(a: &StoneLook, b: &StoneLook) -> Option<(f64, i64)> {
    pair_cells(&read_look(a)?, &read_look(b)?)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::value::RawValue;

    use super::*;
    use crate::models::StoneView;

    #[derive(serde::Deserialize)]
    struct Vectors {
        looks: BTreeMap<String, Box<RawValue>>,
        unreadable: Vec<String>,
    }

    /// Every synthetic look of `fixtures/looks` the models keep reads exactly when the reference
    /// reads it.
    #[test]
    fn looks_read_as_the_reference() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/looks/looks-synthetic.json");
        let v: Vectors = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let mut kept = 0;
        for (name, raw) in &v.looks {
            let s: StoneView = serde_json::from_str(&format!(r#"{{"id": 1, "look": {}}}"#, raw.get())).unwrap();
            let Some(lk) = s.look else { continue };
            assert_eq!(read_look(&lk).is_none(), v.unreadable.contains(name), "{name}");
            kept += 1;
        }
        assert!(kept >= 20, "{kept} looks kept");
    }

    thread_local! {
        /// How many looks [`read_look`] began to decode on this thread.
        pub(super) static DECODED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }

    /// The length of `nu` x `nv` bytes is checked before anything is decoded; a length that fits
    /// still has to decode to them.
    #[test]
    fn the_length_of_the_cells_is_checked_first() {
        let look = |nu: i64, d: &str| StoneLook { v: 1, nu, nv: 1, d: d.to_string(), ..StoneLook::default() };
        let before = DECODED.with(std::cell::Cell::get);
        for d in ["A".repeat(4_000_000), "AA==AA==".to_string(), "A".to_string(), String::new()] {
            assert!(read_look(&look(1, &d)).is_none());
        }
        assert_eq!(DECODED.with(std::cell::Cell::get), before, "none of them reached the decoder");
        assert!(read_look(&look(3, "AA==")).is_none()); // the right length for three cells: decoded, one byte
        assert!(read_look(&look(1, "AA==")).is_some());
        assert_eq!(DECODED.with(std::cell::Cell::get), before + 2);
    }
}
