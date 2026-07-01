//! OASIS reader + writer (round-trip), sharing the GDS in-memory model.
//!
//! OASIS (Open Artwork System Interchange Standard) is the compact, modern layout
//! interchange format — commonly an order of magnitude smaller than the equivalent
//! GDSII. Unlike GDS, records carry **no length prefix**: every record is an
//! unsigned-integer id byte followed by a payload whose shape is fixed by that id.
//! An unknown record therefore cannot be skipped, so a reader must understand every
//! record it meets.
//!
//! Both formats map onto the same [`crate::gds`] `Library` / `Cell` / `Element`
//! model, so `info`, `boolean`, and `flatten` work regardless of source format.
//!
//! v0 scope (honestly bounded, like the GDS reader's): geometry as **RECTANGLE** and
//! **POLYGON**, and cell **PLACEMENT** (from SREF; AREF is expanded to explicit
//! placements). `Path` is stroked to rectangles using its width; `Text` labels are
//! not emitted. Integers are the usual OASIS varints; the `START` unit carries the
//! grid (db-units per micron); the file is uncompressed (no CBLOCK). The reader
//! understands exactly the records this writer emits (plus modal-variable fallbacks);
//! full third-party OASIS ingest (TRAPEZOID/CTRAPEZOID/PATH/CBLOCK/properties) is
//! reserved depth. Depth reserved: modal-variable compaction, matrix repetitions,
//! CBLOCK compression, and full record coverage on read.

use crate::gds::{Cell, Element, Library};
use crate::geom::Rect;

const MAGIC: &[u8] = b"%SEMI-OASIS\r\n"; // 13 bytes

// record ids (the subset this engine reads/writes)
const R_PAD: u64 = 0;
const R_START: u64 = 1;
const R_END: u64 = 2;
const R_CELLNAME_IMPLICIT: u64 = 3;
const R_CELLNAME_EXPLICIT: u64 = 4;
const R_CELL_REF: u64 = 13;
const R_CELL_NAME: u64 = 14;
const R_XYABSOLUTE: u64 = 15;
const R_XYRELATIVE: u64 = 16;
const R_PLACEMENT: u64 = 17;
const R_PLACEMENT_TRANSFORM: u64 = 18;
const R_RECTANGLE: u64 = 20;
const R_POLYGON: u64 = 21;

#[derive(Debug)]
pub struct OasisError(pub String);
impl std::fmt::Display for OasisError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "oasis error: {}", self.0)
    }
}
impl std::error::Error for OasisError {}

impl Library {
    /// Load a layout, dispatching on extension (`.oas` / `.oasis` → OASIS, else GDS).
    pub fn load_any(path: &str) -> Result<Library, String> {
        if is_oasis_path(path) {
            oasis_load(path).map_err(|e| e.to_string())
        } else {
            Library::load(path).map_err(|e| e.to_string())
        }
    }

    /// Save a layout, dispatching on extension (`.oas` / `.oasis` → OASIS, else GDS).
    pub fn save_any(&self, path: &str) -> Result<(), String> {
        if is_oasis_path(path) {
            oasis_save(self, path).map_err(|e| e.to_string())
        } else {
            self.save(path).map_err(|e| e.to_string())
        }
    }
}

pub fn is_oasis_path(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    p.ends_with(".oas") || p.ends_with(".oasis")
}

pub fn oasis_load(path: &str) -> Result<Library, OasisError> {
    let bytes = std::fs::read(path).map_err(|e| OasisError(format!("{path}: {e}")))?;
    parse(&bytes)
}

pub fn oasis_save(lib: &Library, path: &str) -> Result<(), OasisError> {
    std::fs::write(path, to_bytes(lib)).map_err(|e| OasisError(format!("{path}: {e}")))
}

// ---- writer -------------------------------------------------------------------

pub fn to_bytes(lib: &Library) -> Vec<u8> {
    let mut o = Vec::new();
    o.extend_from_slice(MAGIC);

    // START: version, unit (db-units per micron), offset-flag=0, 12 zero table offsets.
    wu(&mut o, R_START);
    wstr(&mut o, "1.0");
    let unit = if lib.db_unit > 0.0 { 1e-6 / lib.db_unit } else { 1000.0 };
    wreal_f64(&mut o, unit);
    wu(&mut o, 0); // offset-flag: table offsets live here in START
    for _ in 0..12 {
        wu(&mut o, 0); // no strict name tables; readers scan the records
    }

    // Name every cell first (implicit CELLNAME → refnums 0,1,2,… in this order).
    for c in &lib.cells {
        wu(&mut o, R_CELLNAME_IMPLICIT);
        wstr(&mut o, &c.name);
    }
    let refnum = |name: &str| lib.cells.iter().position(|c| c.name == name).map(|i| i as u64);

    for (i, c) in lib.cells.iter().enumerate() {
        wu(&mut o, R_CELL_REF);
        wu(&mut o, i as u64);
        wu(&mut o, R_XYABSOLUTE); // absolute coordinates for this cell's records
        for el in &c.elements {
            write_elem(&mut o, el, &refnum);
        }
    }

    write_end(&mut o);
    o
}

fn write_elem(o: &mut Vec<u8>, el: &Element, refnum: &dyn Fn(&str) -> Option<u64>) {
    match el {
        Element::Boundary { layer, datatype, pts } => write_shape(o, *layer, *datatype, pts),
        Element::Box { layer, boxtype, pts } => write_shape(o, *layer, *boxtype, pts),
        Element::Path { layer, datatype, width, pts } => {
            for r in stroke_path(*width, pts) {
                write_rectangle(o, *layer, *datatype, &r);
            }
        }
        Element::Sref { sname, x, y, reflect, mag, angle } => {
            if let Some(rn) = refnum(sname) {
                write_placement(o, rn, *x, *y, *reflect, *mag, *angle);
            }
        }
        Element::Aref { sname, cols, rows, pts, reflect, mag, angle } => {
            let Some(rn) = refnum(sname) else { return };
            // Expand the array into explicit placements (v0 does not emit matrix
            // repetitions). pts = [origin, col-reference, row-reference].
            if pts.len() < 3 || *cols <= 0 || *rows <= 0 {
                return;
            }
            let (ox, oy) = pts[0];
            let dcx = (pts[1].0 - ox) / (*cols as i32).max(1);
            let dcy = (pts[1].1 - oy) / (*cols as i32).max(1);
            let drx = (pts[2].0 - ox) / (*rows as i32).max(1);
            let dry = (pts[2].1 - oy) / (*rows as i32).max(1);
            for r in 0..*rows as i32 {
                for c in 0..*cols as i32 {
                    let x = ox + c * dcx + r * drx;
                    let y = oy + c * dcy + r * dry;
                    write_placement(o, rn, x, y, *reflect, *mag, *angle);
                }
            }
        }
        Element::Text { .. } => { /* labels are non-geometric; not emitted in v0 */ }
    }
}

/// A boundary is written as a RECTANGLE when it is an axis-aligned rectangle,
/// otherwise as a POLYGON.
fn write_shape(o: &mut Vec<u8>, layer: i16, datatype: i16, pts: &[(i32, i32)]) {
    if let Some(r) = Rect::from_boundary(pts) {
        write_rectangle(o, layer, datatype, &r);
    } else {
        write_polygon(o, layer, datatype, pts);
    }
}

fn write_rectangle(o: &mut Vec<u8>, layer: i16, datatype: i16, r: &Rect) {
    // info byte S W H X Y R D L → width|height|x|y|datatype|layer present.
    wu(o, R_RECTANGLE);
    o.push(0x40 | 0x20 | 0x10 | 0x08 | 0x02 | 0x01);
    wu(o, layer as u64);
    wu(o, datatype as u64);
    wu(o, (r.x1 - r.x0) as u64); // width
    wu(o, (r.y1 - r.y0) as u64); // height
    ws(o, r.x0 as i64); // geometry-x = lower-left corner
    ws(o, r.y0 as i64); // geometry-y
}

fn write_polygon(o: &mut Vec<u8>, layer: i16, datatype: i16, pts: &[(i32, i32)]) {
    let v = distinct_vertices(pts);
    if v.len() < 3 {
        return;
    }
    // info byte 0 0 P X Y R D L → point-list|x|y|datatype|layer present.
    wu(o, R_POLYGON);
    o.push(0x20 | 0x10 | 0x08 | 0x02 | 0x01);
    wu(o, layer as u64);
    wu(o, datatype as u64);
    // point-list: type 4 (general g-delta chain), then N-1 deltas; close is implicit.
    wu(o, 4);
    wu(o, (v.len() - 1) as u64);
    for i in 0..v.len() - 1 {
        let dx = (v[i + 1].0 - v[i].0) as i64;
        let dy = (v[i + 1].1 - v[i].1) as i64;
        write_gdelta(o, dx, dy);
    }
    ws(o, v[0].0 as i64); // geometry-x = first vertex
    ws(o, v[0].1 as i64);
}

fn write_placement(o: &mut Vec<u8>, refnum: u64, x: i32, y: i32, reflect: bool, mag: f64, angle: f64) {
    let flip = if reflect { 1u8 } else { 0 };
    let ang = norm_angle(angle);
    if mag == 1.0 && (ang % 90.0).abs() < 1e-9 {
        // PLACEMENT (id 17): info C N X Y R A A F, AA = angle/90 in 0..3.
        let aa = (((ang / 90.0).round() as i64).rem_euclid(4)) as u8;
        wu(o, R_PLACEMENT);
        o.push(0x80 | 0x40 | 0x20 | 0x10 | (aa << 1) | flip);
        wu(o, refnum);
        ws(o, x as i64);
        ws(o, y as i64);
    } else {
        // PLACEMENT with transform (id 18): info C N X Y R M A F.
        wu(o, R_PLACEMENT_TRANSFORM);
        o.push(0x80 | 0x40 | 0x20 | 0x10 | 0x04 | 0x02 | flip);
        wu(o, refnum);
        wreal_f64(o, if mag == 0.0 { 1.0 } else { mag });
        wreal_f64(o, ang);
        ws(o, x as i64);
        ws(o, y as i64);
    }
}

/// Stroke a (Manhattan) centre-line path of full `width` into flush-ended rectangles.
fn stroke_path(width: i32, pts: &[(i32, i32)]) -> Vec<Rect> {
    let mut out = Vec::new();
    if width <= 0 {
        return out;
    }
    let h = width / 2;
    for w in pts.windows(2) {
        let (x0, y0) = w[0];
        let (x1, y1) = w[1];
        if y0 == y1 {
            out.push(Rect::new(x0.min(x1), y0 - h, x0.max(x1), y0 + h));
        } else if x0 == x1 {
            out.push(Rect::new(x0 - h, y0.min(y1), x0 + h, y0.max(y1)));
        } else {
            // diagonal segment: bbox-approximate (rare in Manhattan layouts)
            out.push(Rect::new(x0.min(x1) - h, y0.min(y1) - h, x0.max(x1) + h, y0.max(y1) + h));
        }
    }
    out
}

fn write_end(o: &mut Vec<u8>) {
    // The END record is fixed at 256 bytes: id(1) + padding-string + validation
    // scheme(=0). With offsets already in START, choose a 252-byte pad string so the
    // total is exactly 256: 1 + wu(252)[2] + 252 + wu(0)[1] = 256.
    wu(o, R_END);
    wu(o, 252);
    o.extend_from_slice(&[0u8; 252]);
    wu(o, 0); // validation-scheme: none
}

// ---- reader -------------------------------------------------------------------

pub fn parse(b: &[u8]) -> Result<Library, OasisError> {
    if b.len() < MAGIC.len() || &b[..MAGIC.len()] != MAGIC {
        return Err(OasisError("not an OASIS file (bad magic)".into()));
    }
    let mut i = MAGIC.len();
    let mut lib = Library::default();
    let mut cellnames: Vec<String> = Vec::new();
    let mut cell: Option<Cell> = None;
    // modal state
    let mut m_layer: i16 = 0;
    let mut m_datatype: i16 = 0;
    let mut m_x: i32 = 0;
    let mut m_y: i32 = 0;

    let push_cell = |lib: &mut Library, cell: &mut Option<Cell>| {
        if let Some(c) = cell.take() {
            lib.cells.push(c);
        }
    };

    while i < b.len() {
        let id = ru(b, &mut i)?;
        match id {
            R_PAD => {}
            R_START => {
                let _ver = rstr(b, &mut i)?;
                let unit = rreal(b, &mut i)?;
                if unit > 0.0 {
                    lib.db_unit = 1e-6 / unit;
                    lib.user_unit = 1.0 / unit;
                }
                let offset_flag = ru(b, &mut i)?;
                if offset_flag == 0 {
                    for _ in 0..12 {
                        ru(b, &mut i)?; // table offsets, unused (records are scanned)
                    }
                }
            }
            R_END => break,
            R_CELLNAME_IMPLICIT => cellnames.push(rstr(b, &mut i)?),
            R_CELLNAME_EXPLICIT => {
                let name = rstr(b, &mut i)?;
                let rn = ru(b, &mut i)? as usize;
                if rn >= cellnames.len() {
                    cellnames.resize(rn + 1, String::new());
                }
                cellnames[rn] = name;
            }
            R_CELL_REF => {
                push_cell(&mut lib, &mut cell);
                let rn = ru(b, &mut i)? as usize;
                let name = cellnames.get(rn).cloned().unwrap_or_else(|| format!("CELL{rn}"));
                cell = Some(Cell { name, elements: Vec::new() });
            }
            R_CELL_NAME => {
                push_cell(&mut lib, &mut cell);
                let name = rstr(b, &mut i)?;
                cell = Some(Cell { name, elements: Vec::new() });
            }
            R_XYABSOLUTE | R_XYRELATIVE => { /* only absolute geometry is emitted */ }
            R_RECTANGLE => {
                let el = read_rectangle(b, &mut i, &mut m_layer, &mut m_datatype, &mut m_x, &mut m_y)?;
                if let Some(c) = cell.as_mut() {
                    c.elements.push(el);
                }
            }
            R_POLYGON => {
                let el = read_polygon(b, &mut i, &mut m_layer, &mut m_datatype, &mut m_x, &mut m_y)?;
                if let Some(c) = cell.as_mut() {
                    c.elements.push(el);
                }
            }
            R_PLACEMENT | R_PLACEMENT_TRANSFORM => {
                let el = read_placement(b, &mut i, id, &cellnames, &mut m_x, &mut m_y)?;
                if let (Some(c), Some(el)) = (cell.as_mut(), el) {
                    c.elements.push(el);
                }
            }
            other => {
                return Err(OasisError(format!(
                    "unsupported OASIS record id {other} at byte {}; v0 reads the subset this engine writes",
                    i - 1
                )));
            }
        }
    }
    push_cell(&mut lib, &mut cell);
    Ok(lib)
}

fn read_rectangle(
    b: &[u8],
    i: &mut usize,
    m_layer: &mut i16,
    m_datatype: &mut i16,
    m_x: &mut i32,
    m_y: &mut i32,
) -> Result<Element, OasisError> {
    let info = rb(b, i)?;
    let (s, w, h, x, y, r, d, l) = (
        info & 0x80 != 0, info & 0x40 != 0, info & 0x20 != 0, info & 0x10 != 0,
        info & 0x08 != 0, info & 0x04 != 0, info & 0x02 != 0, info & 0x01 != 0,
    );
    if l {
        *m_layer = ru(b, i)? as i16;
    }
    if d {
        *m_datatype = ru(b, i)? as i16;
    }
    let mut width = 0i64;
    let mut height = 0i64;
    if w {
        width = ru(b, i)? as i64;
    }
    if h {
        height = ru(b, i)? as i64;
    } else if s {
        height = width; // square: height mirrors width
    }
    if x {
        *m_x = rs(b, i)? as i32;
    }
    if y {
        *m_y = rs(b, i)? as i32;
    }
    if r {
        return Err(OasisError("rectangle repetition not supported in v0".into()));
    }
    let rect = Rect::new(*m_x, *m_y, *m_x + width as i32, *m_y + height as i32);
    Ok(Element::Boundary { layer: *m_layer, datatype: *m_datatype, pts: rect.as_boundary() })
}

fn read_polygon(
    b: &[u8],
    i: &mut usize,
    m_layer: &mut i16,
    m_datatype: &mut i16,
    m_x: &mut i32,
    m_y: &mut i32,
) -> Result<Element, OasisError> {
    let info = rb(b, i)?;
    let (p, x, y, r, d, l) = (
        info & 0x20 != 0, info & 0x10 != 0, info & 0x08 != 0,
        info & 0x04 != 0, info & 0x02 != 0, info & 0x01 != 0,
    );
    if l {
        *m_layer = ru(b, i)? as i16;
    }
    if d {
        *m_datatype = ru(b, i)? as i16;
    }
    let mut deltas: Vec<(i64, i64)> = Vec::new();
    if p {
        let ptype = ru(b, i)?;
        let count = ru(b, i)? as usize;
        match ptype {
            4 | 5 => {
                for _ in 0..count {
                    deltas.push(read_gdelta(b, i)?);
                }
            }
            0 | 1 => {
                // manhattan: alternating 1-deltas; type 0 starts horizontal, 1 vertical
                let mut horiz = ptype == 0;
                for _ in 0..count {
                    let d1 = rs(b, i)?;
                    deltas.push(if horiz { (d1, 0) } else { (0, d1) });
                    horiz = !horiz;
                }
            }
            t => return Err(OasisError(format!("polygon point-list type {t} not supported in v0"))),
        }
    }
    if x {
        *m_x = rs(b, i)? as i32;
    }
    if y {
        *m_y = rs(b, i)? as i32;
    }
    if r {
        return Err(OasisError("polygon repetition not supported in v0".into()));
    }
    let mut pts = Vec::with_capacity(deltas.len() + 2);
    let (mut cx, mut cy) = (*m_x as i64, *m_y as i64);
    pts.push((cx as i32, cy as i32));
    for (dx, dy) in &deltas {
        cx += dx;
        cy += dy;
        pts.push((cx as i32, cy as i32));
    }
    if pts.first() != pts.last() {
        pts.push(pts[0]); // close the ring
    }
    Ok(Element::Boundary { layer: *m_layer, datatype: *m_datatype, pts })
}

fn read_placement(
    b: &[u8],
    i: &mut usize,
    id: u64,
    cellnames: &[String],
    m_x: &mut i32,
    m_y: &mut i32,
) -> Result<Option<Element>, OasisError> {
    let info = rb(b, i)?;
    let c = info & 0x80 != 0;
    let n = info & 0x40 != 0; // 1 = reference-number, 0 = name string
    let x = info & 0x20 != 0;
    let y = info & 0x10 != 0;
    let r = info & 0x08 != 0;

    let mut sname = String::new();
    if c {
        if n {
            let rn = ru(b, i)? as usize;
            sname = cellnames.get(rn).cloned().unwrap_or_else(|| format!("CELL{rn}"));
        } else {
            sname = rstr(b, i)?;
        }
    }

    let mut mag = 1.0f64;
    let mut angle;
    let reflect;
    if id == R_PLACEMENT_TRANSFORM {
        let m = info & 0x04 != 0;
        let a = info & 0x02 != 0;
        reflect = info & 0x01 != 0;
        if m {
            mag = rreal(b, i)?;
        }
        angle = if a { rreal(b, i)? } else { 0.0 };
    } else {
        // id 17: bits AA (0x06) = angle/90, F (0x01) = flip
        let aa = (info & 0x06) >> 1;
        reflect = info & 0x01 != 0;
        angle = aa as f64 * 90.0;
    }
    if x {
        *m_x = rs(b, i)? as i32;
    }
    if y {
        *m_y = rs(b, i)? as i32;
    }
    if r {
        return Err(OasisError("placement repetition not supported in v0".into()));
    }
    if angle == -0.0 {
        angle = 0.0;
    }
    if !c {
        return Ok(None); // placement without a target cell — skip
    }
    Ok(Some(Element::Sref { sname, x: *m_x, y: *m_y, reflect, mag, angle }))
}

// ---- primitives ---------------------------------------------------------------

/// Strip a trailing point equal to the first (an explicitly closed ring).
fn distinct_vertices(pts: &[(i32, i32)]) -> Vec<(i32, i32)> {
    let mut v = pts.to_vec();
    while v.len() >= 2 && v.first() == v.last() {
        v.pop();
    }
    v
}

fn norm_angle(a: f64) -> f64 {
    let mut a = a % 360.0;
    if a < 0.0 {
        a += 360.0;
    }
    a
}

fn wu(o: &mut Vec<u8>, mut v: u64) {
    loop {
        let mut byte = (v & 0x7f) as u8;
        v >>= 7;
        if v != 0 {
            byte |= 0x80;
        }
        o.push(byte);
        if v == 0 {
            break;
        }
    }
}

fn ws(o: &mut Vec<u8>, v: i64) {
    let sign = if v < 0 { 1u64 } else { 0 };
    wu(o, (v.unsigned_abs() << 1) | sign);
}

fn ru(b: &[u8], i: &mut usize) -> Result<u64, OasisError> {
    let mut val = 0u64;
    let mut shift = 0u32;
    loop {
        let byte = *b.get(*i).ok_or_else(|| OasisError("unexpected end of file".into()))?;
        *i += 1;
        val |= ((byte & 0x7f) as u64) << shift;
        if byte & 0x80 == 0 {
            break;
        }
        shift += 7;
    }
    Ok(val)
}

fn rs(b: &[u8], i: &mut usize) -> Result<i64, OasisError> {
    let u = ru(b, i)?;
    let mag = (u >> 1) as i64;
    Ok(if u & 1 == 1 { -mag } else { mag })
}

fn rb(b: &[u8], i: &mut usize) -> Result<u8, OasisError> {
    let v = *b.get(*i).ok_or_else(|| OasisError("unexpected end of file (info byte)".into()))?;
    *i += 1;
    Ok(v)
}

/// Write a general (form-2) g-delta: an x component packed in the first integer
/// (bit0 = form marker 1, bit1 = x sign, rest = |x|), then a signed 1-delta for y.
fn write_gdelta(o: &mut Vec<u8>, dx: i64, dy: i64) {
    let xsign = if dx < 0 { 1u64 } else { 0 };
    wu(o, (dx.unsigned_abs() << 2) | (xsign << 1) | 1);
    ws(o, dy);
}

fn read_gdelta(b: &[u8], i: &mut usize) -> Result<(i64, i64), OasisError> {
    let v = ru(b, i)?;
    if v & 1 == 1 {
        // form 2: general x in this integer, y as a following signed 1-delta
        let xmag = (v >> 2) as i64;
        let x = if (v >> 1) & 1 == 1 { -xmag } else { xmag };
        let y = rs(b, i)?;
        Ok((x, y))
    } else {
        // form 1: single octangular delta — direction in bits 1..3, magnitude above
        let dir = (v >> 1) & 7;
        let mag = (v >> 4) as i64;
        Ok(match dir {
            0 => (mag, 0),
            1 => (0, mag),
            2 => (-mag, 0),
            3 => (0, -mag),
            4 => (mag, mag),
            5 => (-mag, mag),
            6 => (-mag, -mag),
            _ => (mag, -mag),
        })
    }
}

fn wstr(o: &mut Vec<u8>, s: &str) {
    wu(o, s.len() as u64);
    o.extend_from_slice(s.as_bytes());
}

fn rstr(b: &[u8], i: &mut usize) -> Result<String, OasisError> {
    let n = ru(b, i)? as usize;
    let end = i.checked_add(n).filter(|e| *e <= b.len());
    let end = end.ok_or_else(|| OasisError("string runs past end of file".into()))?;
    let s = String::from_utf8_lossy(&b[*i..end]).into_owned();
    *i = end;
    Ok(s)
}

/// Write a real as type 7 (IEEE float64, little-endian) — exact and simple.
fn wreal_f64(o: &mut Vec<u8>, v: f64) {
    wu(o, 7);
    o.extend_from_slice(&v.to_le_bytes());
}

fn rreal(b: &[u8], i: &mut usize) -> Result<f64, OasisError> {
    let t = ru(b, i)?;
    let read_bytes = |i: &mut usize, n: usize| -> Result<Vec<u8>, OasisError> {
        let end = i.checked_add(n).filter(|e| *e <= b.len()).ok_or_else(|| OasisError("real runs past end".into()))?;
        let v = b[*i..end].to_vec();
        *i = end;
        Ok(v)
    };
    Ok(match t {
        0 => ru(b, i)? as f64,
        1 => -(ru(b, i)? as f64),
        2 => 1.0 / ru(b, i)? as f64,
        3 => -1.0 / ru(b, i)? as f64,
        4 => {
            let n = ru(b, i)? as f64;
            let d = ru(b, i)? as f64;
            n / d
        }
        5 => {
            let n = ru(b, i)? as f64;
            let d = ru(b, i)? as f64;
            -(n / d)
        }
        6 => {
            let by = read_bytes(i, 4)?;
            f32::from_le_bytes([by[0], by[1], by[2], by[3]]) as f64
        }
        7 => {
            let by = read_bytes(i, 8)?;
            f64::from_le_bytes([by[0], by[1], by[2], by[3], by[4], by[5], by[6], by[7]])
        }
        other => return Err(OasisError(format!("unknown real type {other}"))),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Library {
        Library {
            name: "TOP".into(),
            user_unit: 1e-3,
            db_unit: 1e-9,
            cells: vec![
                Cell {
                    name: "leaf".into(),
                    elements: vec![
                        // an axis-aligned rectangle → RECTANGLE
                        Element::Boundary {
                            layer: 68,
                            datatype: 20,
                            pts: Rect::new(0, 0, 100, 50).as_boundary(),
                        },
                        // an L-shaped (non-rect) polygon → POLYGON
                        Element::Boundary {
                            layer: 69,
                            datatype: 0,
                            pts: vec![(0, 0), (40, 0), (40, 20), (20, 20), (20, 40), (0, 40), (0, 0)],
                        },
                    ],
                },
                Cell {
                    name: "top".into(),
                    elements: vec![Element::Sref {
                        sname: "leaf".into(),
                        x: 1000,
                        y: 2000,
                        reflect: false,
                        mag: 1.0,
                        angle: 90.0,
                    }],
                },
            ],
        }
    }

    #[test]
    fn end_record_is_256_bytes() {
        let mut o = Vec::new();
        write_end(&mut o);
        assert_eq!(o.len(), 256);
    }

    #[test]
    fn varint_round_trips() {
        for v in [0u64, 1, 127, 128, 255, 16384, 1_000_000, u32::MAX as u64] {
            let mut o = Vec::new();
            wu(&mut o, v);
            let mut i = 0;
            assert_eq!(ru(&o, &mut i).unwrap(), v);
            assert_eq!(i, o.len());
        }
        for v in [0i64, 1, -1, 127, -128, 100_000, -100_000] {
            let mut o = Vec::new();
            ws(&mut o, v);
            let mut i = 0;
            assert_eq!(rs(&o, &mut i).unwrap(), v);
        }
    }

    #[test]
    fn gdelta_round_trips() {
        for (dx, dy) in [(10i64, 0i64), (0, -20), (30, 40), (-5, -6), (0, 0)] {
            let mut o = Vec::new();
            write_gdelta(&mut o, dx, dy);
            let mut i = 0;
            assert_eq!(read_gdelta(&o, &mut i).unwrap(), (dx, dy));
        }
    }

    #[test]
    fn library_round_trips() {
        let lib = sample();
        let back = parse(&to_bytes(&lib)).unwrap();
        assert_eq!(back.cells.len(), 2);
        assert_eq!(back.cells[0].name, "leaf");
        assert_eq!(back.cells[1].name, "top");

        // db unit survives the micron round-trip
        assert!((back.db_unit - 1e-9).abs() < 1e-18);

        // rectangle came back as a closed 5-point boundary on 68/20
        match &back.cells[0].elements[0] {
            Element::Boundary { layer, datatype, pts } => {
                assert_eq!((*layer, *datatype), (68, 20));
                assert_eq!(Rect::from_boundary(pts), Some(Rect::new(0, 0, 100, 50)));
            }
            e => panic!("expected boundary, got {e:?}"),
        }

        // L-shape survives as a 6-vertex (7-point closed) polygon on 69/0
        match &back.cells[0].elements[1] {
            Element::Boundary { layer, pts, .. } => {
                assert_eq!(*layer, 69);
                assert_eq!(pts.first(), pts.last());
                assert_eq!(distinct_vertices(pts).len(), 6);
            }
            e => panic!("expected boundary, got {e:?}"),
        }

        // placement survives with its refnum-resolved name, position, and 90° angle
        match &back.cells[1].elements[0] {
            Element::Sref { sname, x, y, angle, .. } => {
                assert_eq!(sname, "leaf");
                assert_eq!((*x, *y), (1000, 2000));
                assert!((*angle - 90.0).abs() < 1e-9);
            }
            e => panic!("expected sref, got {e:?}"),
        }
    }

    #[test]
    fn magic_is_checked() {
        assert!(parse(b"not-oasis").is_err());
    }
}
