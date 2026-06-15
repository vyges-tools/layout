//! GDSII reader + writer (round-trip).
//!
//! GDSII is a stream of records: `u16 length` (big-endian, incl. the 4-byte header),
//! `u8 record-type`, `u8 data-type`, then `length-4` bytes of data. Coordinates are
//! big-endian `i32`; `UNITS` carries two 8-byte GDS reals (user unit, db unit in
//! metres). v0 reads BOUNDARY/PATH/SREF/AREF/BOX (TEXT skipped) and writes them back.

#[derive(Debug, Clone)]
pub enum Element {
    Boundary { layer: i16, datatype: i16, pts: Vec<(i32, i32)> },
    Path { layer: i16, datatype: i16, width: i32, pts: Vec<(i32, i32)> },
    Sref { sname: String, x: i32, y: i32, reflect: bool, mag: f64, angle: f64 },
    Aref { sname: String, cols: i16, rows: i16, pts: Vec<(i32, i32)>, reflect: bool, mag: f64, angle: f64 },
    Box { layer: i16, boxtype: i16, pts: Vec<(i32, i32)> },
    Text { layer: i16, texttype: i16, x: i32, y: i32, string: String },
}

#[derive(Debug, Clone, Default)]
pub struct Cell {
    pub name: String,
    pub elements: Vec<Element>,
}

#[derive(Debug, Clone)]
pub struct Library {
    pub name: String,
    pub user_unit: f64, // db units per user unit
    pub db_unit: f64,   // metres per db unit
    pub cells: Vec<Cell>,
}

impl Default for Library {
    fn default() -> Self {
        Library { name: "LIB".into(), user_unit: 1e-3, db_unit: 1e-9, cells: vec![] }
    }
}

#[derive(Debug)]
pub struct GdsError(pub String);
impl std::fmt::Display for GdsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "gds error: {}", self.0)
    }
}
impl std::error::Error for GdsError {}

// record types
const HEADER: u8 = 0x00;
const BGNLIB: u8 = 0x01;
const LIBNAME: u8 = 0x02;
const UNITS: u8 = 0x03;
const ENDLIB: u8 = 0x04;
const BGNSTR: u8 = 0x05;
const STRNAME: u8 = 0x06;
const ENDSTR: u8 = 0x07;
const BOUNDARY: u8 = 0x08;
const PATH: u8 = 0x09;
const SREF: u8 = 0x0A;
const AREF: u8 = 0x0B;
const TEXT: u8 = 0x0C;
const LAYER: u8 = 0x0D;
const DATATYPE: u8 = 0x0E;
const WIDTH: u8 = 0x0F;
const XY: u8 = 0x10;
const ENDEL: u8 = 0x11;
const SNAME: u8 = 0x12;
const COLROW: u8 = 0x13;
const TEXTTYPE: u8 = 0x16;
const STRING: u8 = 0x19;
const STRANS: u8 = 0x1A;
const MAG: u8 = 0x1B;
const ANGLE: u8 = 0x1C;
const BOX: u8 = 0x2D;
const BOXTYPE: u8 = 0x2E;

impl Library {
    pub fn load(path: &str) -> Result<Library, GdsError> {
        let bytes = std::fs::read(path).map_err(|e| GdsError(format!("{path}: {e}")))?;
        Library::parse(&bytes)
    }

    pub fn save(&self, path: &str) -> Result<(), GdsError> {
        std::fs::write(path, self.to_bytes()).map_err(|e| GdsError(format!("{path}: {e}")))
    }

    pub fn parse(b: &[u8]) -> Result<Library, GdsError> {
        let mut lib = Library::default();
        let mut cell: Option<Cell> = None;
        let mut e = Eb::default();
        let mut i = 0usize;
        while i + 4 <= b.len() {
            let len = u16::from_be_bytes([b[i], b[i + 1]]) as usize;
            if len < 4 || i + len > b.len() {
                break;
            }
            let rtype = b[i + 2];
            let data = &b[i + 4..i + len];
            match rtype {
                LIBNAME => lib.name = ascii(data),
                UNITS => {
                    if data.len() >= 16 {
                        lib.user_unit = real8(&data[0..8]);
                        lib.db_unit = real8(&data[8..16]);
                    }
                }
                BGNSTR => cell = Some(Cell::default()),
                STRNAME => {
                    if let Some(c) = cell.as_mut() {
                        c.name = ascii(data);
                    }
                }
                ENDSTR => {
                    if let Some(c) = cell.take() {
                        lib.cells.push(c);
                    }
                }
                BOUNDARY | PATH | SREF | AREF | TEXT | BOX => {
                    e = Eb::default();
                    e.rtype = rtype;
                }
                LAYER => e.layer = be_i16(data),
                DATATYPE | BOXTYPE | TEXTTYPE => e.datatype = be_i16(data),
                WIDTH => e.width = be_i32(data),
                SNAME => e.sname = ascii(data),
                STRING => e.text = ascii(data),
                COLROW => {
                    e.cols = be_i16(data);
                    e.rows = be_i16(&data[2.min(data.len())..]);
                }
                STRANS => e.reflect = !data.is_empty() && (data[0] & 0x80) != 0,
                MAG => e.mag = real8(data),
                ANGLE => e.angle = real8(data),
                XY => e.pts = parse_xy(data),
                ENDEL => {
                    if let (Some(c), Some(el)) = (cell.as_mut(), e.build()) {
                        c.elements.push(el);
                    }
                }
                HEADER | BGNLIB | ENDLIB => {}
                _ => {}
            }
            i += len;
        }
        Ok(lib)
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut o = Vec::new();
        rec(&mut o, HEADER, 0x02, &600i16.to_be_bytes());
        rec(&mut o, BGNLIB, 0x02, &[0u8; 24]); // 12 i16 date fields
        rec(&mut o, LIBNAME, 0x06, str_bytes(&self.name).as_slice());
        let mut u = Vec::new();
        u.extend_from_slice(&enc_real8(self.user_unit));
        u.extend_from_slice(&enc_real8(self.db_unit));
        rec(&mut o, UNITS, 0x05, &u);
        for c in &self.cells {
            rec(&mut o, BGNSTR, 0x02, &[0u8; 24]);
            rec(&mut o, STRNAME, 0x06, str_bytes(&c.name).as_slice());
            for el in &c.elements {
                write_elem(&mut o, el);
            }
            rec(&mut o, ENDSTR, 0x00, &[]);
        }
        rec(&mut o, ENDLIB, 0x00, &[]);
        o
    }
}

#[derive(Default)]
struct Eb {
    rtype: u8,
    layer: i16,
    datatype: i16,
    width: i32,
    sname: String,
    cols: i16,
    rows: i16,
    reflect: bool,
    mag: f64,
    angle: f64,
    pts: Vec<(i32, i32)>,
    text: String,
}

impl Eb {
    fn build(&self) -> Option<Element> {
        match self.rtype {
            BOUNDARY => Some(Element::Boundary {
                layer: self.layer,
                datatype: self.datatype,
                pts: self.pts.clone(),
            }),
            PATH => Some(Element::Path {
                layer: self.layer,
                datatype: self.datatype,
                width: self.width,
                pts: self.pts.clone(),
            }),
            SREF => self.pts.first().map(|&(x, y)| Element::Sref {
                sname: self.sname.clone(),
                x,
                y,
                reflect: self.reflect,
                mag: self.mag,
                angle: self.angle,
            }),
            AREF => Some(Element::Aref {
                sname: self.sname.clone(),
                cols: self.cols,
                rows: self.rows,
                pts: self.pts.clone(),
                reflect: self.reflect,
                mag: self.mag,
                angle: self.angle,
            }),
            BOX => Some(Element::Box {
                layer: self.layer,
                boxtype: self.datatype,
                pts: self.pts.clone(),
            }),
            TEXT => self.pts.first().map(|&(x, y)| Element::Text {
                layer: self.layer,
                texttype: self.datatype,
                x,
                y,
                string: self.text.clone(),
            }),
            _ => None,
        }
    }
}

fn write_elem(o: &mut Vec<u8>, el: &Element) {
    match el {
        Element::Boundary { layer, datatype, pts } => {
            rec(o, BOUNDARY, 0x00, &[]);
            rec(o, LAYER, 0x02, &layer.to_be_bytes());
            rec(o, DATATYPE, 0x02, &datatype.to_be_bytes());
            rec(o, XY, 0x03, &xy_bytes(pts));
            rec(o, ENDEL, 0x00, &[]);
        }
        Element::Path { layer, datatype, width, pts } => {
            rec(o, PATH, 0x00, &[]);
            rec(o, LAYER, 0x02, &layer.to_be_bytes());
            rec(o, DATATYPE, 0x02, &datatype.to_be_bytes());
            if *width != 0 {
                rec(o, WIDTH, 0x03, &width.to_be_bytes());
            }
            rec(o, XY, 0x03, &xy_bytes(pts));
            rec(o, ENDEL, 0x00, &[]);
        }
        Element::Sref { sname, x, y, .. } => {
            rec(o, SREF, 0x00, &[]);
            rec(o, SNAME, 0x06, str_bytes(sname).as_slice());
            rec(o, XY, 0x03, &xy_bytes(&[(*x, *y)]));
            rec(o, ENDEL, 0x00, &[]);
        }
        Element::Aref { sname, cols, rows, pts, .. } => {
            rec(o, AREF, 0x00, &[]);
            rec(o, SNAME, 0x06, str_bytes(sname).as_slice());
            let mut cr = Vec::new();
            cr.extend_from_slice(&cols.to_be_bytes());
            cr.extend_from_slice(&rows.to_be_bytes());
            rec(o, COLROW, 0x02, &cr);
            rec(o, XY, 0x03, &xy_bytes(pts));
            rec(o, ENDEL, 0x00, &[]);
        }
        Element::Box { layer, boxtype, pts } => {
            rec(o, BOX, 0x00, &[]);
            rec(o, LAYER, 0x02, &layer.to_be_bytes());
            rec(o, BOXTYPE, 0x02, &boxtype.to_be_bytes());
            rec(o, XY, 0x03, &xy_bytes(pts));
            rec(o, ENDEL, 0x00, &[]);
        }
        Element::Text { layer, texttype, x, y, string } => {
            rec(o, TEXT, 0x00, &[]);
            rec(o, LAYER, 0x02, &layer.to_be_bytes());
            rec(o, TEXTTYPE, 0x02, &texttype.to_be_bytes());
            rec(o, XY, 0x03, &xy_bytes(&[(*x, *y)]));
            rec(o, STRING, 0x06, str_bytes(string).as_slice());
            rec(o, ENDEL, 0x00, &[]);
        }
    }
}

// ---- primitives ---------------------------------------------------------------

fn rec(o: &mut Vec<u8>, rtype: u8, dtype: u8, data: &[u8]) {
    let len = (4 + data.len()) as u16;
    o.extend_from_slice(&len.to_be_bytes());
    o.push(rtype);
    o.push(dtype);
    o.extend_from_slice(data);
}

fn be_i16(d: &[u8]) -> i16 {
    if d.len() >= 2 {
        i16::from_be_bytes([d[0], d[1]])
    } else {
        0
    }
}
fn be_i32(d: &[u8]) -> i32 {
    if d.len() >= 4 {
        i32::from_be_bytes([d[0], d[1], d[2], d[3]])
    } else {
        0
    }
}

fn parse_xy(d: &[u8]) -> Vec<(i32, i32)> {
    let mut v = Vec::new();
    let mut i = 0;
    while i + 8 <= d.len() {
        let x = be_i32(&d[i..i + 4]);
        let y = be_i32(&d[i + 4..i + 8]);
        v.push((x, y));
        i += 8;
    }
    v
}

fn xy_bytes(pts: &[(i32, i32)]) -> Vec<u8> {
    let mut o = Vec::with_capacity(pts.len() * 8);
    for &(x, y) in pts {
        o.extend_from_slice(&x.to_be_bytes());
        o.extend_from_slice(&y.to_be_bytes());
    }
    o
}

fn ascii(d: &[u8]) -> String {
    let end = d.iter().position(|&c| c == 0).unwrap_or(d.len());
    String::from_utf8_lossy(&d[..end]).trim_end().to_string()
}

fn str_bytes(s: &str) -> Vec<u8> {
    let mut b = s.as_bytes().to_vec();
    if b.len() % 2 != 0 {
        b.push(0); // records must be even length
    }
    b
}

/// Decode an 8-byte GDS real (sign · mantissa · 16^(exp-64)).
fn real8(b: &[u8]) -> f64 {
    if b.len() < 8 {
        return 0.0;
    }
    let sign = if b[0] & 0x80 != 0 { -1.0 } else { 1.0 };
    let exp = (b[0] & 0x7f) as i32 - 64;
    let mut mant = 0.0f64;
    for &byte in &b[1..8] {
        mant = mant * 256.0 + byte as f64;
    }
    mant /= 2f64.powi(56);
    sign * mant * 16f64.powi(exp)
}

/// Encode a finite f64 as an 8-byte GDS real.
fn enc_real8(v: f64) -> [u8; 8] {
    let mut b = [0u8; 8];
    if v == 0.0 || !v.is_finite() {
        return b;
    }
    let sign = v < 0.0;
    let mut x = v.abs();
    let mut exp = 0i32;
    while x >= 1.0 {
        x /= 16.0;
        exp += 1;
    }
    while x < 1.0 / 16.0 {
        x *= 16.0;
        exp -= 1;
    }
    let mut mant = (x * 2f64.powi(56)).round() as u64;
    b[0] = (((exp + 64) as u8) & 0x7f) | if sign { 0x80 } else { 0 };
    for i in (1..8).rev() {
        b[i] = (mant & 0xff) as u8;
        mant >>= 8;
    }
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real8_round_trip() {
        for v in [1e-3, 1e-9, 1.0, 0.5, 2.0, 1e-6] {
            let d = real8(&enc_real8(v));
            assert!((d - v).abs() / v < 1e-9, "{v} -> {d}");
        }
    }

    #[test]
    fn library_round_trip() {
        let mut lib = Library::default();
        lib.name = "TOP".into();
        lib.cells.push(Cell {
            name: "box".into(),
            elements: vec![Element::Boundary {
                layer: 68,
                datatype: 20,
                pts: vec![(0, 0), (100, 0), (100, 50), (0, 50), (0, 0)],
            }],
        });
        let back = Library::parse(&lib.to_bytes()).unwrap();
        assert_eq!(back.name, "TOP");
        assert_eq!(back.cells.len(), 1);
        assert_eq!(back.cells[0].name, "box");
        match &back.cells[0].elements[0] {
            Element::Boundary { layer, pts, .. } => {
                assert_eq!(*layer, 68);
                assert_eq!(pts.len(), 5);
                assert_eq!(pts[2], (100, 50));
            }
            _ => panic!("expected boundary"),
        }
    }
}
