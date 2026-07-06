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
const R_TEXTSTRING_IMPLICIT: u64 = 5;
const R_TEXTSTRING_EXPLICIT: u64 = 6;
const R_PROPNAME_IMPLICIT: u64 = 7;
const R_PROPNAME_EXPLICIT: u64 = 8;
const R_PROPSTRING_IMPLICIT: u64 = 9;
const R_PROPSTRING_EXPLICIT: u64 = 10;
const R_LAYERNAME_DATA: u64 = 11;
const R_LAYERNAME_TEXT: u64 = 12;
const R_TEXT: u64 = 19;
const R_TRAPEZOID_AB: u64 = 23;
const R_TRAPEZOID_A: u64 = 24;
const R_TRAPEZOID_B: u64 = 25;
const R_CTRAPEZOID: u64 = 26;
const R_CIRCLE: u64 = 27;
const R_PROPERTY: u64 = 28;
const R_PROPERTY_REPEAT: u64 = 29;
const R_XNAME_IMPLICIT: u64 = 30;
const R_XNAME_EXPLICIT: u64 = 31;
const R_CBLOCK: u64 = 34;
const R_CELL_REF: u64 = 13;
const R_CELL_NAME: u64 = 14;
const R_XYABSOLUTE: u64 = 15;
const R_XYRELATIVE: u64 = 16;
const R_PLACEMENT: u64 = 17;
const R_PLACEMENT_TRANSFORM: u64 = 18;
const R_RECTANGLE: u64 = 20;
const R_POLYGON: u64 = 21;
const R_PATH: u64 = 22;

/// Human name for an OASIS record id — makes "unsupported record" errors actionable
/// (which is the depth item to grow next) instead of a bare number.
fn record_name(id: u64) -> &'static str {
    match id {
        5 | 6 => "TEXTSTRING",
        7 | 8 => "PROPNAME",
        9 | 10 => "PROPSTRING",
        11 | 12 => "LAYERNAME",
        19 => "TEXT",
        23..=25 => "TRAPEZOID",
        26 => "CTRAPEZOID",
        27 => "CIRCLE",
        28 | 29 => "PROPERTY",
        30 | 31 => "XNAME",
        32 => "XELEMENT",
        33 => "XGEOMETRY",
        34 => "CBLOCK",
        _ => "unknown",
    }
}

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
    let mut st = ParseState::default();
    st.process(&b[MAGIC.len()..])?;
    st.finish_cell();
    // resolve refnum-referenced cell names from the (possibly end-located) table
    for (idx, rn) in st.refnums.iter().enumerate() {
        if let Some(rn) = rn {
            if let Some(name) = st.cellnames.get(*rn as usize) {
                if !name.is_empty() {
                    st.lib.cells[idx].name = name.clone();
                }
            }
        }
    }
    Ok(st.lib)
}

/// Reader state threaded through the record stream — including any CBLOCK-compressed
/// sub-streams, which `process` re-enters with the same modal/table state.
#[derive(Default)]
struct ParseState {
    lib: Library,
    cellnames: Vec<String>,
    textstrings: Vec<String>,
    cell: Option<Cell>,
    // refnum per finished cell, resolved to a name after the whole stream is scanned
    refnums: Vec<Option<u64>>,
    cur_refnum: Option<u64>,
    // geometry modal state
    m_layer: i16,
    m_datatype: i16,
    m_halfwidth: i64,
    m_geom_w: i64,
    m_geom_h: i64,
    m_x: i32,
    m_y: i32,
    // text-specific modal state (never perturbs geometry)
    m_textlayer: i16,
    m_texttype: i16,
    m_text_x: i32,
    m_text_y: i32,
    m_textstring: String,
    // last repetition (modal, reused by repetition-type 0)
    m_repetition: Vec<(i32, i32)>,
    ended: bool,
}

impl ParseState {
    fn finish_cell(&mut self) {
        if let Some(c) = self.cell.take() {
            self.lib.cells.push(c);
            self.refnums.push(self.cur_refnum.take());
        }
    }

    fn push_el(&mut self, el: Element) {
        if let Some(c) = self.cell.as_mut() {
            c.elements.push(el);
        }
    }

    /// Emit `el` once per repetition offset (or once at (0,0) when `reps` is empty).
    fn push_repeated(&mut self, el: Element, reps: &[(i32, i32)]) {
        if reps.is_empty() {
            self.push_el(el);
        } else {
            for &(dx, dy) in reps {
                self.push_el(translate(&el, dx, dy));
            }
        }
    }

    fn process(&mut self, b: &[u8]) -> Result<(), OasisError> {
        let mut i = 0;
        while i < b.len() && !self.ended {
            let id = ru(b, &mut i)?;
            match id {
                R_PAD => {}
                R_START => {
                    let _ver = rstr(b, &mut i)?;
                    let unit = rreal(b, &mut i)?;
                    if unit > 0.0 {
                        self.lib.db_unit = 1e-6 / unit;
                        self.lib.user_unit = 1.0 / unit;
                    }
                    let offset_flag = ru(b, &mut i)?;
                    if offset_flag == 0 {
                        for _ in 0..12 {
                            ru(b, &mut i)?; // table offsets, unused (records are scanned)
                        }
                    }
                }
                R_END => self.ended = true,
                R_CELLNAME_IMPLICIT => self.cellnames.push(rstr(b, &mut i)?),
                R_TEXTSTRING_IMPLICIT => self.textstrings.push(rstr(b, &mut i)?),
                R_TEXTSTRING_EXPLICIT => {
                    let s = rstr(b, &mut i)?;
                    let rn = ru(b, &mut i)? as usize;
                    if rn >= self.textstrings.len() {
                        self.textstrings.resize(rn + 1, String::new());
                    }
                    self.textstrings[rn] = s;
                }
                R_CELLNAME_EXPLICIT => {
                    let name = rstr(b, &mut i)?;
                    let rn = ru(b, &mut i)? as usize;
                    if rn >= self.cellnames.len() {
                        self.cellnames.resize(rn + 1, String::new());
                    }
                    self.cellnames[rn] = name;
                }
                R_PROPNAME_IMPLICIT | R_PROPSTRING_IMPLICIT => {
                    rstr(b, &mut i)?; // name/string table entry — value not modeled
                }
                R_PROPNAME_EXPLICIT | R_PROPSTRING_EXPLICIT | R_XNAME_IMPLICIT | R_XNAME_EXPLICIT => {
                    skip_name_record(b, &mut i, id)?;
                }
                R_LAYERNAME_DATA | R_LAYERNAME_TEXT => {
                    rstr(b, &mut i)?; // layer name
                    read_interval(b, &mut i)?; // layer interval
                    read_interval(b, &mut i)?; // datatype/texttype interval
                }
                R_CELL_REF => {
                    self.finish_cell();
                    let rn = ru(b, &mut i)?;
                    let name = self
                        .cellnames
                        .get(rn as usize)
                        .filter(|n| !n.is_empty())
                        .cloned()
                        .unwrap_or_else(|| format!("CELL{rn}"));
                    self.cur_refnum = Some(rn);
                    self.cell = Some(Cell { name, elements: Vec::new() });
                }
                R_CELL_NAME => {
                    self.finish_cell();
                    let name = rstr(b, &mut i)?;
                    self.cell = Some(Cell { name, elements: Vec::new() });
                }
                R_XYABSOLUTE | R_XYRELATIVE => { /* only absolute geometry is emitted */ }
                R_RECTANGLE => {
                    let (el, reps) = read_rectangle(b, &mut i, self)?;
                    self.push_repeated(el, &reps);
                }
                R_POLYGON => {
                    let (el, reps) = read_polygon(b, &mut i, self)?;
                    self.push_repeated(el, &reps);
                }
                R_PATH => {
                    let (el, reps) = read_path(b, &mut i, self)?;
                    self.push_repeated(el, &reps);
                }
                R_TRAPEZOID_AB | R_TRAPEZOID_A | R_TRAPEZOID_B => {
                    let (el, reps) = read_trapezoid(b, &mut i, id, self)?;
                    self.push_repeated(el, &reps);
                }
                R_CTRAPEZOID => {
                    let (el, reps) = read_ctrapezoid(b, &mut i, self)?;
                    self.push_repeated(el, &reps);
                }
                R_CIRCLE => {
                    let (el, reps) = read_circle(b, &mut i, self)?;
                    self.push_repeated(el, &reps);
                }
                R_TEXT => {
                    let (el, reps) = read_text(b, &mut i, self)?;
                    self.push_repeated(el, &reps);
                }
                R_PLACEMENT | R_PLACEMENT_TRANSFORM => {
                    let (el, reps) = read_placement(b, &mut i, id, self)?;
                    if let Some(el) = el {
                        self.push_repeated(el, &reps);
                    }
                }
                R_PROPERTY | R_PROPERTY_REPEAT => skip_property(b, &mut i, id)?,
                R_CBLOCK => {
                    let comp_type = ru(b, &mut i)?;
                    let _uncomp_len = ru(b, &mut i)?;
                    let comp_len = ru(b, &mut i)? as usize;
                    let end = i
                        .checked_add(comp_len)
                        .filter(|e| *e <= b.len())
                        .ok_or_else(|| OasisError("cblock length runs past end of file".into()))?;
                    let comp = &b[i..end];
                    i = end;
                    if comp_type != 0 {
                        return Err(OasisError(format!(
                            "cblock compression type {comp_type} unsupported (only DEFLATE=0)"
                        )));
                    }
                    let data = inflate_cblock(comp)?;
                    self.process(&data)?; // decompressed records continue in this context
                }
                other => {
                    return Err(OasisError(format!(
                        "unsupported OASIS record {other} ({}) at byte {}; reader covers \
                         RECTANGLE/POLYGON/PATH/TRAPEZOID/CTRAPEZOID/CIRCLE/TEXT/PLACEMENT + \
                         repetitions, properties, name tables, and CBLOCK",
                        record_name(other),
                        i - 1
                    )));
                }
            }
        }
        Ok(())
    }
}

/// DEFLATE-inflate a CBLOCK payload (raw RFC-1951; falls back to zlib-wrapped).
fn inflate_cblock(comp: &[u8]) -> Result<Vec<u8>, OasisError> {
    miniz_oxide::inflate::decompress_to_vec(comp)
        .or_else(|_| miniz_oxide::inflate::decompress_to_vec_zlib(comp))
        .map_err(|e| OasisError(format!("cblock DEFLATE inflate failed: {e:?}")))
}

/// Translate an element by `(dx, dy)` — used to expand a repetition.
fn translate(el: &Element, dx: i32, dy: i32) -> Element {
    let shift = |pts: &[(i32, i32)]| pts.iter().map(|&(x, y)| (x + dx, y + dy)).collect::<Vec<_>>();
    match el {
        Element::Boundary { layer, datatype, pts } => {
            Element::Boundary { layer: *layer, datatype: *datatype, pts: shift(pts) }
        }
        Element::Path { layer, datatype, width, pts } => {
            Element::Path { layer: *layer, datatype: *datatype, width: *width, pts: shift(pts) }
        }
        Element::Box { layer, boxtype, pts } => {
            Element::Box { layer: *layer, boxtype: *boxtype, pts: shift(pts) }
        }
        Element::Text { layer, texttype, x, y, string } => {
            Element::Text { layer: *layer, texttype: *texttype, x: x + dx, y: y + dy, string: string.clone() }
        }
        Element::Sref { sname, x, y, reflect, mag, angle } => {
            Element::Sref { sname: sname.clone(), x: x + dx, y: y + dy, reflect: *reflect, mag: *mag, angle: *angle }
        }
        Element::Aref { sname, cols, rows, pts, reflect, mag, angle } => Element::Aref {
            sname: sname.clone(),
            cols: *cols,
            rows: *rows,
            pts: shift(pts),
            reflect: *reflect,
            mag: *mag,
            angle: *angle,
        },
    }
}

/// A geometry reader returns the element plus the repetition offsets to stamp it at
/// (empty = a single copy at its own position).
type GeoResult = Result<(Element, Vec<(i32, i32)>), OasisError>;
/// Like [`GeoResult`] but the element is optional (a placement may target no cell).
type PlacementResult = Result<(Option<Element>, Vec<(i32, i32)>), OasisError>;

fn read_rectangle(b: &[u8], i: &mut usize, st: &mut ParseState) -> GeoResult {
    let info = rb(b, i)?;
    let (s, w, h, x, y, r, d, l) = (
        info & 0x80 != 0, info & 0x40 != 0, info & 0x20 != 0, info & 0x10 != 0,
        info & 0x08 != 0, info & 0x04 != 0, info & 0x02 != 0, info & 0x01 != 0,
    );
    if l {
        st.m_layer = ru(b, i)? as i16;
    }
    if d {
        st.m_datatype = ru(b, i)? as i16;
    }
    if w {
        st.m_geom_w = ru(b, i)? as i64;
    }
    if h {
        st.m_geom_h = ru(b, i)? as i64;
    } else if s {
        st.m_geom_h = st.m_geom_w; // square: height mirrors width
    }
    if x {
        st.m_x = rs(b, i)? as i32;
    }
    if y {
        st.m_y = rs(b, i)? as i32;
    }
    let reps = if r { read_repetition(b, i, st)? } else { Vec::new() };
    let rect = Rect::new(st.m_x, st.m_y, st.m_x + st.m_geom_w as i32, st.m_y + st.m_geom_h as i32);
    Ok((Element::Boundary { layer: st.m_layer, datatype: st.m_datatype, pts: rect.as_boundary() }, reps))
}

fn read_polygon(b: &[u8], i: &mut usize, st: &mut ParseState) -> GeoResult {
    let info = rb(b, i)?;
    let (p, x, y, r, d, l) = (
        info & 0x20 != 0, info & 0x10 != 0, info & 0x08 != 0,
        info & 0x04 != 0, info & 0x02 != 0, info & 0x01 != 0,
    );
    if l {
        st.m_layer = ru(b, i)? as i16;
    }
    if d {
        st.m_datatype = ru(b, i)? as i16;
    }
    let deltas: Vec<(i64, i64)> = if p { read_point_list(b, i, true)? } else { Vec::new() };
    if x {
        st.m_x = rs(b, i)? as i32;
    }
    if y {
        st.m_y = rs(b, i)? as i32;
    }
    let reps = if r { read_repetition(b, i, st)? } else { Vec::new() };
    let mut pts = Vec::with_capacity(deltas.len() + 2);
    let (mut cx, mut cy) = (st.m_x as i64, st.m_y as i64);
    pts.push((cx as i32, cy as i32));
    for (dx, dy) in &deltas {
        cx += dx;
        cy += dy;
        pts.push((cx as i32, cy as i32));
    }
    if pts.first() != pts.last() {
        pts.push(pts[0]); // close the ring
    }
    Ok((Element::Boundary { layer: st.m_layer, datatype: st.m_datatype, pts }, reps))
}

/// Read an OASIS point-list → the deltas between successive vertices. Covers the
/// manhattan (0/1 alternating, 2 any-direction), octangular (3), and all-angle
/// g-delta (4) forms — the ones real writers emit; the double-delta form (5) is
/// reserved.
///
/// `closed` = the shape auto-closes (a POLYGON, not a PATH). For the alternating
/// manhattan forms (0/1) a closed shape **omits its last delta** — the
/// (count+1)th edge continues the H/V alternation back to the start axis, and the
/// ring closes on the other axis. We synthesize that implied delta; without it the
/// straight ring-closure cuts a corner (real writers, e.g. gdstk, use type 0).
fn read_point_list(b: &[u8], i: &mut usize, closed: bool) -> Result<Vec<(i64, i64)>, OasisError> {
    let ptype = ru(b, i)?;
    let count = ru(b, i)? as usize;
    let mut deltas = Vec::with_capacity(count);
    match ptype {
        0 | 1 => {
            // manhattan, alternating 1-deltas; type 0 starts horizontal, 1 vertical
            let mut horiz = ptype == 0;
            let (mut sum_h, mut sum_v) = (0i64, 0i64);
            for _ in 0..count {
                let d1 = rs(b, i)?;
                if horiz {
                    deltas.push((d1, 0));
                    sum_h += d1;
                } else {
                    deltas.push((0, d1));
                    sum_v += d1;
                }
                horiz = !horiz;
            }
            if closed {
                // `horiz` now holds the orientation of the implied (count+1)th delta.
                if horiz {
                    deltas.push((-sum_h, 0));
                } else {
                    deltas.push((0, -sum_v));
                }
            }
        }
        2 => {
            // manhattan, each a 2-delta: 2 low bits = direction, rest = magnitude
            for _ in 0..count {
                let v = ru(b, i)?;
                let m = (v >> 2) as i64;
                deltas.push(match v & 3 {
                    0 => (m, 0),
                    1 => (0, m),
                    2 => (-m, 0),
                    _ => (0, -m),
                });
            }
        }
        3 => {
            // octangular, each a 3-delta: 3 low bits = direction, rest = magnitude
            for _ in 0..count {
                let v = ru(b, i)?;
                let m = (v >> 3) as i64;
                deltas.push(match v & 7 {
                    0 => (m, 0),
                    1 => (0, m),
                    2 => (-m, 0),
                    3 => (0, -m),
                    4 => (m, m),
                    5 => (-m, m),
                    6 => (-m, -m),
                    _ => (m, -m),
                });
            }
        }
        4 => {
            for _ in 0..count {
                deltas.push(read_gdelta(b, i)?);
            }
        }
        t => return Err(OasisError(format!("point-list type {t} not supported in v0"))),
    }
    Ok(deltas)
}

/// PATH record → an open centre-line polyline with a width (`Element::Path`). The
/// per-end extension scheme is parsed (to stay byte-aligned) but not modeled — our
/// consumers use the centre-line and width; ends are stroked flush.
fn read_path(b: &[u8], i: &mut usize, st: &mut ParseState) -> GeoResult {
    let info = rb(b, i)?;
    let (e, w, p, x, y, r, d, l) = (
        info & 0x80 != 0, info & 0x40 != 0, info & 0x20 != 0, info & 0x10 != 0,
        info & 0x08 != 0, info & 0x04 != 0, info & 0x02 != 0, info & 0x01 != 0,
    );
    if l {
        st.m_layer = ru(b, i)? as i16;
    }
    if d {
        st.m_datatype = ru(b, i)? as i16;
    }
    if w {
        st.m_halfwidth = ru(b, i)? as i64;
    }
    if e {
        // extension-scheme byte: bits 2-3 start, bits 0-1 end; 3 = explicit signed-int
        let ext = rb(b, i)?;
        if (ext >> 2) & 3 == 3 {
            rs(b, i)?;
        }
        if ext & 3 == 3 {
            rs(b, i)?;
        }
    }
    let deltas = if p { read_point_list(b, i, false)? } else { Vec::new() };
    if x {
        st.m_x = rs(b, i)? as i32;
    }
    if y {
        st.m_y = rs(b, i)? as i32;
    }
    let reps = if r { read_repetition(b, i, st)? } else { Vec::new() };
    let mut pts = Vec::with_capacity(deltas.len() + 1);
    let (mut cx, mut cy) = (st.m_x as i64, st.m_y as i64);
    pts.push((cx as i32, cy as i32));
    for (dx, dy) in &deltas {
        cx += dx;
        cy += dy;
        pts.push((cx as i32, cy as i32));
    }
    Ok((Element::Path { layer: st.m_layer, datatype: st.m_datatype, width: (2 * st.m_halfwidth) as i32, pts }, reps))
}

/// TEXT record → a label (`Element::Text`). The string is inline (n-string) or a
/// reference into the TEXTSTRING table. Uses text-specific modal state so it never
/// perturbs the geometry modal layer/position.
fn read_text(b: &[u8], i: &mut usize, st: &mut ParseState) -> GeoResult {
    let info = rb(b, i)?;
    let (c, n, x, y, r, t, l) = (
        info & 0x40 != 0, info & 0x20 != 0, info & 0x10 != 0, info & 0x08 != 0,
        info & 0x04 != 0, info & 0x02 != 0, info & 0x01 != 0,
    );
    if c {
        st.m_textstring = if n {
            let rn = ru(b, i)? as usize;
            st.textstrings.get(rn).cloned().unwrap_or_default()
        } else {
            rstr(b, i)?
        };
    }
    if l {
        st.m_textlayer = ru(b, i)? as i16;
    }
    if t {
        st.m_texttype = ru(b, i)? as i16;
    }
    if x {
        st.m_text_x = rs(b, i)? as i32;
    }
    if y {
        st.m_text_y = rs(b, i)? as i32;
    }
    let reps = if r { read_repetition(b, i, st)? } else { Vec::new() };
    let el = Element::Text {
        layer: st.m_textlayer,
        texttype: st.m_texttype,
        x: st.m_text_x,
        y: st.m_text_y,
        string: st.m_textstring.clone(),
    };
    Ok((el, reps))
}

fn read_placement(
    b: &[u8],
    i: &mut usize,
    id: u64,
    st: &mut ParseState,
) -> PlacementResult {
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
            sname = st.cellnames.get(rn).cloned().unwrap_or_else(|| format!("CELL{rn}"));
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
        st.m_x = rs(b, i)? as i32;
    }
    if y {
        st.m_y = rs(b, i)? as i32;
    }
    let reps = if r { read_repetition(b, i, st)? } else { Vec::new() };
    if angle == -0.0 {
        angle = 0.0;
    }
    if !c {
        return Ok((None, reps)); // placement without a target cell — skip
    }
    Ok((Some(Element::Sref { sname, x: st.m_x, y: st.m_y, reflect, mag, angle }), reps))
}

// ---- trapezoids, circle, repetition, and skipped-record helpers ---------------

/// TRAPEZOID (records 23/24/25): a bounding box (w,h) at (x,y) with two edge deltas
/// slanting the parallel-edge orientation set by the `O` info bit.
fn read_trapezoid(b: &[u8], i: &mut usize, id: u64, st: &mut ParseState) -> GeoResult {
    let info = rb(b, i)?;
    let o = info & 0x80 != 0; // 0 = horizontal parallel edges, 1 = vertical
    let (w, h, x, y, r, d, l) = (
        info & 0x40 != 0, info & 0x20 != 0, info & 0x10 != 0,
        info & 0x08 != 0, info & 0x04 != 0, info & 0x02 != 0, info & 0x01 != 0,
    );
    if l {
        st.m_layer = ru(b, i)? as i16;
    }
    if d {
        st.m_datatype = ru(b, i)? as i16;
    }
    if w {
        st.m_geom_w = ru(b, i)? as i64;
    }
    if h {
        st.m_geom_h = ru(b, i)? as i64;
    }
    let delta_a = if id != R_TRAPEZOID_B { rs(b, i)? } else { 0 };
    let delta_b = if id != R_TRAPEZOID_A { rs(b, i)? } else { 0 };
    if x {
        st.m_x = rs(b, i)? as i32;
    }
    if y {
        st.m_y = rs(b, i)? as i32;
    }
    let reps = if r { read_repetition(b, i, st)? } else { Vec::new() };
    let (px, py, ww, hh) = (st.m_x as i64, st.m_y as i64, st.m_geom_w, st.m_geom_h);
    // SEMI-P39 §28: (x,y) is the bounding-box lower-left. Horizontal → delta_a = xP−xR,
    // delta_b = xQ−xS (R,S bottom; P,Q top); vertical → delta_a = yP−yR, delta_b = yQ−yS
    // (P,Q left edge; R,S right edge). The reference corner sits on the bbox edge and the
    // partner is offset by the delta (clamp so the bbox stays [x,x+w]×[y,y+h]).
    let v: Vec<(i64, i64)> = if !o {
        let xr = px - delta_a.min(0);
        let xp = xr + delta_a;
        let xs = px + ww - delta_b.max(0);
        let xq = xs + delta_b;
        vec![(xr, py), (xs, py), (xq, py + hh), (xp, py + hh)]
    } else {
        let yr = py - delta_a.min(0);
        let yp = yr + delta_a;
        let ys = py + hh - delta_b.max(0);
        let yq = ys + delta_b;
        vec![(px, yp), (px + ww, yr), (px + ww, ys), (px, yq)]
    };
    let mut pts: Vec<(i32, i32)> = v.into_iter().map(|(a, c)| (a as i32, c as i32)).collect();
    if pts.first() != pts.last() {
        if let Some(&f) = pts.first() {
            pts.push(f);
        }
    }
    Ok((Element::Boundary { layer: st.m_layer, datatype: st.m_datatype, pts }, reps))
}

/// CTRAPEZOID (record 26): one of 26 compact trapezoid/triangle shapes in the
/// bounding box (w,h) at (x,y). Types are expanded to explicit polygons.
fn read_ctrapezoid(b: &[u8], i: &mut usize, st: &mut ParseState) -> GeoResult {
    let info = rb(b, i)?;
    let t_present = info & 0x80 != 0;
    let (w, h, x, y, r, d, l) = (
        info & 0x40 != 0, info & 0x20 != 0, info & 0x10 != 0,
        info & 0x08 != 0, info & 0x04 != 0, info & 0x02 != 0, info & 0x01 != 0,
    );
    if l {
        st.m_layer = ru(b, i)? as i16;
    }
    if d {
        st.m_datatype = ru(b, i)? as i16;
    }
    let ctype = if t_present { ru(b, i)? } else { 0 };
    if w {
        st.m_geom_w = ru(b, i)? as i64;
    }
    if h {
        st.m_geom_h = ru(b, i)? as i64;
    }
    // Triangle/square ctrapezoid types store only one dimension; the other equals it.
    if !h {
        st.m_geom_h = st.m_geom_w;
    }
    if !w {
        st.m_geom_w = st.m_geom_h;
    }
    if x {
        st.m_x = rs(b, i)? as i32;
    }
    if y {
        st.m_y = rs(b, i)? as i32;
    }
    let reps = if r { read_repetition(b, i, st)? } else { Vec::new() };
    let pts = ctrapezoid_polygon(ctype, st.m_x as i64, st.m_y as i64, st.m_geom_w, st.m_geom_h)?;
    Ok((Element::Boundary { layer: st.m_layer, datatype: st.m_datatype, pts }, reps))
}

/// Expand a CTRAPEZOID type (0..25) at `(x,y)` with box `(w,h)` into a closed polygon.
/// The 26 forms are the SEMI-P39 compact set: 0–3 wedges off a full box, 4–7 double
/// wedges, 8–15 half-height/width wedges, 16–23 the four right triangles, 24 a square,
/// 25 a rectangle.
fn ctrapezoid_polygon(ctype: u64, x: i64, y: i64, w: i64, h: i64) -> Result<Vec<(i32, i32)>, OasisError> {
    let (x0, y0, x1, y1) = (x, y, x + w, y + h);
    let close = |mut v: Vec<(i64, i64)>| {
        if v.first() != v.last() {
            if let Some(&f) = v.first() {
                v.push(f);
            }
        }
        v.into_iter().map(|(a, c)| (a as i32, c as i32)).collect::<Vec<_>>()
    };
    let v = match ctype {
        // 0..3 horizontal right-trapezoids, one 45° corner cut of leg h (w >= h)
        0 => vec![(x0, y0), (x1, y0), (x1 - h, y1), (x0, y1)],
        1 => vec![(x0, y0), (x1, y0), (x1, y1), (x0 + h, y1)],
        2 => vec![(x0, y0), (x1 - h, y0), (x1, y1), (x0, y1)],
        3 => vec![(x0 + h, y0), (x1, y0), (x1, y1), (x0, y1)],
        // 4..5 isoceles horizontal trapezoids, 6..7 horizontal parallelograms (w >= 2h)
        4 => vec![(x0, y0), (x1, y0), (x1 - h, y1), (x0 + h, y1)],
        5 => vec![(x0 + h, y0), (x1 - h, y0), (x1, y1), (x0, y1)],
        6 => vec![(x0 + h, y0), (x1, y0), (x1 - h, y1), (x0, y1)],
        7 => vec![(x0, y0), (x1 - h, y0), (x1, y1), (x0 + h, y1)],
        // 8..11 vertical right-trapezoids, one 45° cut of leg w (h >= w)
        8 => vec![(x0, y0), (x1, y0), (x1, y1 - w), (x0, y1)],
        9 => vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1 - w)],
        10 => vec![(x0, y0), (x1, y0 + w), (x1, y1), (x0, y1)],
        11 => vec![(x0, y0 + w), (x1, y0), (x1, y1), (x0, y1)],
        // 12..13 isoceles vertical trapezoids, 14..15 vertical parallelograms (h >= 2w)
        12 => vec![(x0, y0), (x1, y0 + w), (x1, y1 - w), (x0, y1)],
        13 => vec![(x0, y0 + w), (x1, y0), (x1, y1), (x0, y1 - w)],
        14 => vec![(x0, y0), (x1, y0 + w), (x1, y1), (x0, y1 - w)],
        15 => vec![(x0, y0 + w), (x1, y0), (x1, y1 - w), (x0, y1)],
        // 16..19 the four right triangles (legs w = h)
        16 => vec![(x0, y0), (x1, y0), (x0, y1)],
        17 => vec![(x0, y0), (x1, y0), (x1, y1)],
        18 => vec![(x1, y0), (x1, y1), (x0, y1)],
        19 => vec![(x0, y0), (x1, y1), (x0, y1)],
        24 => vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1)], // rectangle (w x h)
        25 => vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1)], // square (w x w)
        // 20..23 are the rare 2x isoceles-triangle forms; not yet validated, so we
        // error clearly rather than emit possibly-wrong geometry (types 0..19,24,25 are
        // validated against gdstk).
        20..=23 => {
            return Err(OasisError(format!(
                "ctrapezoid type {ctype} (2x isoceles triangle) not yet supported — please report"
            )))
        }
        t => return Err(OasisError(format!("ctrapezoid type {t} out of range (0..25)"))),
    };
    Ok(close(v))
}

/// CIRCLE (record 27): centre + radius, approximated as a 48-gon polygon.
fn read_circle(b: &[u8], i: &mut usize, st: &mut ParseState) -> GeoResult {
    let info = rb(b, i)?;
    let (has_r, x, y, rep, d, l) = (
        info & 0x20 != 0, info & 0x10 != 0, info & 0x08 != 0,
        info & 0x04 != 0, info & 0x02 != 0, info & 0x01 != 0,
    );
    if l {
        st.m_layer = ru(b, i)? as i16;
    }
    if d {
        st.m_datatype = ru(b, i)? as i16;
    }
    let radius = if has_r { ru(b, i)? as i64 } else { st.m_halfwidth };
    st.m_halfwidth = radius; // circle radius shares the half-width modal per spec
    if x {
        st.m_x = rs(b, i)? as i32;
    }
    if y {
        st.m_y = rs(b, i)? as i32;
    }
    let reps = if rep { read_repetition(b, i, st)? } else { Vec::new() };
    const N: usize = 48;
    let (cx, cy, rr) = (st.m_x as f64, st.m_y as f64, radius as f64);
    let mut pts: Vec<(i32, i32)> = (0..N)
        .map(|k| {
            let a = std::f64::consts::TAU * k as f64 / N as f64;
            ((cx + rr * a.cos()).round() as i32, (cy + rr * a.sin()).round() as i32)
        })
        .collect();
    if let Some(&f) = pts.first() {
        pts.push(f);
    }
    Ok((Element::Boundary { layer: st.m_layer, datatype: st.m_datatype, pts }, reps))
}

/// Read a repetition → the list of `(dx, dy)` offsets to stamp a shape at. Type 0
/// reuses the modal repetition; every other type also updates it.
fn read_repetition(b: &[u8], i: &mut usize, st: &mut ParseState) -> Result<Vec<(i32, i32)>, OasisError> {
    let rtype = ru(b, i)?;
    let g = |v: (i64, i64)| (v.0 as i32, v.1 as i32);
    let reps: Vec<(i32, i32)> = match rtype {
        0 => return Ok(st.m_repetition.clone()),
        1 => {
            let (nx, ny) = (ru(b, i)? as i64 + 2, ru(b, i)? as i64 + 2);
            let (dx, dy) = (ru(b, i)? as i64, ru(b, i)? as i64);
            (0..ny).flat_map(|iy| (0..nx).map(move |ix| ((ix * dx) as i32, (iy * dy) as i32))).collect()
        }
        2 => {
            let n = ru(b, i)? as i64 + 2;
            let dx = ru(b, i)? as i64;
            (0..n).map(|ix| ((ix * dx) as i32, 0)).collect()
        }
        3 => {
            let n = ru(b, i)? as i64 + 2;
            let dy = ru(b, i)? as i64;
            (0..n).map(|iy| (0, (iy * dy) as i32)).collect()
        }
        4 | 5 => {
            let n = ru(b, i)? as usize + 2;
            let grid = if rtype == 5 { ru(b, i)? as i64 } else { 1 };
            let mut xs = 0i64;
            let mut v = vec![(0, 0)];
            for _ in 0..n - 1 {
                xs += ru(b, i)? as i64 * grid;
                v.push((xs as i32, 0));
            }
            v
        }
        6 | 7 => {
            let n = ru(b, i)? as usize + 2;
            let grid = if rtype == 7 { ru(b, i)? as i64 } else { 1 };
            let mut ys = 0i64;
            let mut v = vec![(0, 0)];
            for _ in 0..n - 1 {
                ys += ru(b, i)? as i64 * grid;
                v.push((0, ys as i32));
            }
            v
        }
        8 => {
            let (nx, ny) = (ru(b, i)? as i64 + 2, ru(b, i)? as i64 + 2);
            let (ax, ay) = read_gdelta(b, i)?;
            let (bx, by) = read_gdelta(b, i)?;
            (0..ny)
                .flat_map(|iy| {
                    (0..nx).map(move |ix| ((ix * ax + iy * bx) as i32, (ix * ay + iy * by) as i32))
                })
                .collect()
        }
        9 => {
            let n = ru(b, i)? as i64 + 2;
            let (gx, gy) = read_gdelta(b, i)?;
            (0..n).map(|k| ((k * gx) as i32, (k * gy) as i32)).collect()
        }
        10 | 11 => {
            let n = ru(b, i)? as usize + 2;
            let grid = if rtype == 11 { ru(b, i)? as i64 } else { 1 };
            let (mut cx, mut cy) = (0i64, 0i64);
            let mut v = vec![(0, 0)];
            for _ in 0..n - 1 {
                let (gx, gy) = read_gdelta(b, i)?;
                cx += gx * grid;
                cy += gy * grid;
                v.push(g((cx, cy)));
            }
            v
        }
        t => return Err(OasisError(format!("repetition type {t} not supported"))),
    };
    st.m_repetition = reps.clone();
    Ok(reps)
}

/// Skip a LAYERNAME interval (`type` + up to two bounds).
fn read_interval(b: &[u8], i: &mut usize) -> Result<(), OasisError> {
    let t = ru(b, i)?;
    match t {
        0 => {}                 // all values
        1..=3 => { ru(b, i)?; }
        4 => { ru(b, i)?; ru(b, i)?; }
        _ => return Err(OasisError(format!("bad interval type {t}"))),
    }
    Ok(())
}

/// Skip a PROPNAME/PROPSTRING/XNAME explicit record (a string + a reference number).
fn skip_name_record(b: &[u8], i: &mut usize, _id: u64) -> Result<(), OasisError> {
    rstr(b, i)?; // the name / string
    ru(b, i)?; // the reference number
    Ok(())
}

/// Skip a PROPERTY record without modeling it — consume its typed value list so the
/// stream stays aligned.
fn skip_property(b: &[u8], i: &mut usize, id: u64) -> Result<(), OasisError> {
    if id == R_PROPERTY_REPEAT {
        return Ok(()); // reuse the modal property — nothing to consume
    }
    let info = rb(b, i)?;
    // prop-info-byte is `UUUUVCNS`: C (bit 2) = property name present; N (bit 1) = that
    // name is a reference-number (else a string); V (bit 3) set = reuse the modal
    // value-list (so no count/values follow), clear = the value-list is present with
    // UUUU (bits 4-7) as its count. S (bit 0), the standard-property flag, is not needed
    // to walk the record.
    let (c, n, v_present) = (info & 0x04 != 0, info & 0x02 != 0, info & 0x08 == 0);
    if c {
        if n {
            ru(b, i)?; // propname reference number
        } else {
            rstr(b, i)?; // propname string
        }
    }
    if v_present {
        // value-count is the high nibble of the info byte (15 → an explicit uint)
        let count = if (info >> 4) == 15 { ru(b, i)? } else { (info >> 4) as u64 };
        for _ in 0..count {
            skip_prop_value(b, i)?;
        }
    }
    Ok(())
}

/// Skip one PROPERTY value by its leading type byte.
fn skip_prop_value(b: &[u8], i: &mut usize) -> Result<(), OasisError> {
    let t = ru(b, i)?;
    match t {
        0..=7 => {
            rreal_typed(b, i, t)?;
        }
        8 => {
            ru(b, i)?;
        } // unsigned-integer
        9 => {
            rs(b, i)?;
        } // signed-integer
        10..=12 => {
            rstr(b, i)?;
        } // a/b/n-string
        13..=15 => {
            ru(b, i)?;
        } // prop-string reference number
        other => return Err(OasisError(format!("bad property value type {other}"))),
    }
    Ok(())
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
    rreal_typed(b, i, t)
}

/// Read a real's payload given its already-consumed type byte (shared by `rreal` and
/// the PROPERTY value skipper, where the type is the value tag).
fn rreal_typed(b: &[u8], i: &mut usize, t: u64) -> Result<f64, OasisError> {
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

    #[test]
    fn ctrapezoid_validated_shapes() {
        use crate::geom::poly_area;
        // type 16: right triangle, legs 40 → area 800
        assert_eq!(poly_area(&ctrapezoid_polygon(16, 0, 0, 40, 40).unwrap()), 800.0);
        // type 0: 100×40 box, 45° cut of leg h=40 at top-right → 3200
        assert_eq!(poly_area(&ctrapezoid_polygon(0, 0, 0, 100, 40).unwrap()), 3200.0);
        // type 8: vertical, 40×100 box, 45° cut of leg w=40 at top-right → 3200
        assert_eq!(poly_area(&ctrapezoid_polygon(8, 0, 0, 40, 100).unwrap()), 3200.0);
        // type 24: rectangle 40×40 → 1600
        assert_eq!(poly_area(&ctrapezoid_polygon(24, 0, 0, 40, 40).unwrap()), 1600.0);
        // the rare 2× forms (20..23) error rather than emit possibly-wrong geometry
        assert!(ctrapezoid_polygon(20, 0, 0, 40, 40).is_err());
        assert!(ctrapezoid_polygon(99, 0, 0, 40, 40).is_err());
    }

    #[test]
    fn manhattan_polygon_implied_closure() {
        // point-list: type 0 (manhattan, horizontal-first), count 4, stored deltas
        // +40, +20, -20, +20 — the L-shape's edges minus the implied last delta.
        // signed-int encoding: +40=0x50, +20=0x28, -20=0x29.
        let bytes = [0u8, 4, 0x50, 0x28, 0x29, 0x28];

        // closed (POLYGON): the implied (-sum_h, 0) = (-20, 0) is synthesized, so the
        // corner is preserved (area 1200, not the corner-cut 800).
        let mut i = 0;
        let closed = read_point_list(&bytes, &mut i, true).unwrap();
        assert_eq!(closed, [(40, 0), (0, 20), (-20, 0), (0, 20), (-20, 0)]);

        // open (PATH): only the 4 stored deltas, no implied vertex.
        let mut j = 0;
        let open = read_point_list(&bytes, &mut j, false).unwrap();
        assert_eq!(open, [(40, 0), (0, 20), (-20, 0), (0, 20)]);
    }

    // PROPERTY info-byte is `UUUUVCNS`; value presence is the V bit (3), not S (0).
    // A standard property (S=1) with a value list (V=0) must still have its values
    // consumed — the earlier S-bit read hid them and desynced the stream.
    #[test]
    fn property_standard_flag_does_not_hide_values() {
        // info 0x11: UUUU=1 (one value), V=0 (values present), C=0, N=0, S=1.
        // value: type 8 (unsigned-integer) = 42. Trailing 0x63 must survive.
        let buf = [0x11u8, 0x08, 42, 0x63];
        let mut i = 0;
        skip_property(&buf, &mut i, R_PROPERTY).unwrap();
        assert_eq!(i, 3, "info + value-type + value consumed, sentinel left");
        assert_eq!(buf[i], 0x63);
    }

    // A named property (C=1, name given by reference-number N=1) with one value.
    #[test]
    fn property_with_name_ref_and_value() {
        // info 0x16: UUUU=1, V=0, C=1, N=1, S=0. name-ref=3; value type 8 = 5.
        let buf = [0x16u8, 3, 0x08, 5, 0x63];
        let mut i = 0;
        skip_property(&buf, &mut i, R_PROPERTY).unwrap();
        assert_eq!(i, 4);
        assert_eq!(buf[i], 0x63);
    }

    // V=1 means "reuse the modal value-list": UUUU is not a count and no values follow.
    // The old S-bit logic read UUUU as a count here and over-consumed.
    #[test]
    fn property_reuse_values_consumes_no_value_bytes() {
        // info 0x28: UUUU=2, V=1 (reuse), C=0, N=0, S=0. No value bytes follow.
        let buf = [0x28u8, 0x63];
        let mut i = 0;
        skip_property(&buf, &mut i, R_PROPERTY).unwrap();
        assert_eq!(i, 1, "only the info byte is consumed when V=1");
        assert_eq!(buf[i], 0x63);
    }
}
