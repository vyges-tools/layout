// SPDX-License-Identifier: Apache-2.0
//! Draw a scene once; emit it as **SVG or PNG**.
//!
//! Every tool in the suite that produces a picture wants both, for reasons that do not overlap.
//! SVG is exact, diffable, and greppable, so it is what belongs in a repo or a CI artifact. PNG
//! is what you paste into a slide, a web page, or a message — and for a dense drawing it is
//! *much* smaller, because a vector file grows with the number of shapes while a raster file is
//! bounded by its pixel count.
//!
//! Before this existed, `vyges-gds-view` carried its own SVG and PNG writers and the next tool
//! that needed a drawing was about to carry a second copy. The primitives here are deliberately
//! boring — rectangles, lines, text — because that is what every one of those drawings is made
//! of, and anything richer would push callers back to writing their own.
//!
//! Placed in `vyges-layout` rather than in a tool because this crate is already the shared leaf:
//! pure Rust, no C toolchain, and already a dependency of the engines that draw. A renderer in a
//! tool crate could not be reached by its siblings; a renderer in the CLI would invert the
//! dependency graph.

mod font;
mod png;
mod svg;

pub use png::to_png;
pub use svg::to_svg;

/// 8-bit RGB.
pub type Rgb = (u8, u8, u8);

/// Horizontal placement of text against its anchor point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Anchor {
    #[default]
    Start,
    Middle,
    End,
}

impl Anchor {
    fn svg(self) -> &'static str {
        match self {
            Anchor::Start => "start",
            Anchor::Middle => "middle",
            Anchor::End => "end",
        }
    }
}

/// One drawable. Coordinates are in output pixels, y growing **down** (the SVG convention);
/// callers flip their own axes before they get here, since only they know which way is up.
#[derive(Debug, Clone)]
pub enum Shape {
    Rect {
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        fill: Option<Rgb>,
        /// 0..=1. Applied to the fill only; strokes are drawn solid so an outline stays legible
        /// over whatever is behind it.
        fill_opacity: f64,
        stroke: Option<Rgb>,
        stroke_width: f64,
    },
    Line {
        x1: f64,
        y1: f64,
        x2: f64,
        y2: f64,
        stroke: Rgb,
        width: f64,
        dashed: bool,
    },
    /// `y` is the text **baseline**, matching SVG.
    Text {
        x: f64,
        y: f64,
        text: String,
        size: f64,
        fill: Rgb,
        anchor: Anchor,
        bold: bool,
    },
}

impl Shape {
    /// Convenience for the common filled-and-outlined box.
    pub fn rect(x: f64, y: f64, w: f64, h: f64, c: Rgb, fill_opacity: f64, stroke_width: f64) -> Shape {
        Shape::Rect {
            x,
            y,
            w,
            h,
            fill: Some(c),
            fill_opacity,
            stroke: Some(c),
            stroke_width,
        }
    }

    pub fn text(x: f64, y: f64, text: impl Into<String>, size: f64, fill: Rgb) -> Shape {
        Shape::Text {
            x,
            y,
            text: text.into(),
            size,
            fill,
            anchor: Anchor::Start,
            bold: false,
        }
    }

    pub fn anchored(mut self, a: Anchor) -> Shape {
        if let Shape::Text { anchor, .. } = &mut self {
            *anchor = a;
        }
        self
    }

    pub fn bolded(mut self) -> Shape {
        if let Shape::Text { bold, .. } = &mut self {
            *bold = true;
        }
        self
    }
}

/// A page: a size, a background, and shapes in paint order (first drawn is furthest back).
#[derive(Debug, Clone)]
pub struct Scene {
    pub width: f64,
    pub height: f64,
    pub background: Rgb,
    pub shapes: Vec<Shape>,
    /// Goes in the SVG `<title>`; ignored by the raster back-end, which has nowhere to put it.
    pub title: String,
}

impl Default for Scene {
    fn default() -> Self {
        Scene {
            width: 800.0,
            height: 600.0,
            background: (255, 255, 255),
            shapes: Vec::new(),
            title: String::new(),
        }
    }
}

impl Scene {
    pub fn new(width: f64, height: f64) -> Scene {
        Scene {
            width,
            height,
            ..Default::default()
        }
    }

    pub fn with_background(mut self, c: Rgb) -> Scene {
        self.background = c;
        self
    }

    pub fn with_title(mut self, t: impl Into<String>) -> Scene {
        self.title = t.into();
        self
    }

    pub fn push(&mut self, s: Shape) -> &mut Scene {
        self.shapes.push(s);
        self
    }

    /// Render to SVG.
    pub fn to_svg(&self) -> String {
        svg::to_svg(self)
    }

    /// Render to PNG at `scale` device pixels per scene unit. `scale` above 1 is how a small
    /// drawing stays legible when someone drops it into a slide at full width.
    pub fn to_png(&self, scale: f64) -> Vec<u8> {
        png::to_png(self, scale)
    }
}

/// Advance width of one glyph cell, in units of the nominal text size.
///
/// The 5x7 cell plus one column of spacing is 6 wide and 7 tall, so a glyph advances 6/7 of the
/// size. Both back-ends use this so `Anchor::Middle` puts text in the same place in each.
pub(crate) const ADVANCE: f64 = 6.0 / 7.0;

/// Width of `text` at `size`, in output units.
pub fn text_width(text: &str, size: f64) -> f64 {
    text.chars().count() as f64 * size * ADVANCE
}

/// Left edge of `text` given its anchor point.
pub(crate) fn anchor_x(x: f64, text: &str, size: f64, a: Anchor) -> f64 {
    let w = text_width(text, size);
    match a {
        Anchor::Start => x,
        Anchor::Middle => x - w / 2.0,
        Anchor::End => x - w,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn sample() -> Scene {
        let mut s = Scene::new(200.0, 100.0).with_background((250, 250, 252));
        s.push(Shape::rect(10.0, 10.0, 80.0, 40.0, (78, 121, 167), 0.3, 1.5));
        s.push(Shape::Line {
            x1: 10.0,
            y1: 60.0,
            x2: 190.0,
            y2: 60.0,
            stroke: (214, 39, 40),
            width: 2.0,
            dashed: true,
        });
        s.push(Shape::text(20.0, 30.0, "u_base", 10.0, (17, 17, 17)));
        s
    }

    #[test]
    fn the_two_back_ends_agree_on_where_centred_text_starts() {
        // If they disagree, the same scene reads differently in the two formats and the PNG
        // silently becomes a picture of something else.
        let w = text_width("abcd", 10.0);
        assert_eq!(anchor_x(100.0, "abcd", 10.0, Anchor::Middle), 100.0 - w / 2.0);
        assert_eq!(anchor_x(100.0, "abcd", 10.0, Anchor::End), 100.0 - w);
        assert_eq!(anchor_x(100.0, "abcd", 10.0, Anchor::Start), 100.0);
    }

    #[test]
    fn the_font_covers_what_a_drawing_actually_says() {
        // Missing glyphs render as blanks, which is the quiet kind of wrong: the label is still
        // there, just partly erased.
        let need = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789 .,:;-_/()[]<>=+*#%!?";
        let missing: Vec<char> = need
            .chars()
            .filter(|c| !font::FONT.iter().any(|(g, _)| g == c))
            .collect();
        assert!(missing.is_empty(), "font is missing {missing:?}");
    }

    #[test]
    fn a_glyph_is_not_blank() {
        // Guards the ASCII-art conversion: an off-by-one in the row parser produces a table of
        // structurally valid, entirely empty glyphs.
        let (_, a) = font::FONT.iter().find(|(c, _)| *c == 'A').unwrap();
        assert!(a.iter().any(|&col| col != 0), "'A' came out blank");
        let (_, sp) = font::FONT.iter().find(|(c, _)| *c == ' ').unwrap();
        assert!(sp.iter().all(|&col| col == 0), "space must be blank");
    }

    #[test]
    fn typographic_punctuation_transliterates_rather_than_vanishing() {
        // An em dash or middle dot that renders as a blank turns `soc — inst` into `soc   inst`,
        // which reads as though the separator was never there. The vector back-end keeps the real
        // character; only the raster path substitutes.
        for (from, to) in [
            ('\u{2014}', '-'),
            ('\u{00b7}', '.'),
            ('\u{00d7}', 'x'),
            ('\u{00b5}', 'u'),
            ('\u{2192}', '>'),
        ] {
            assert_eq!(font::fallback(from), to, "{from:?} should map to {to:?}");
            assert!(font::glyph(from).is_some(), "{from:?} has no drawable glyph");
        }
        // Something with no sensible substitute still falls through to a blank rather than to a
        // wrong glyph.
        assert!(font::glyph('\u{4e2d}').is_none());
    }

    #[test]
    fn descenders_survived_the_conversion() {
        // The art's trailing blank rows are data, not padding — a parser that strips them shifts
        // every glyph up a row and silently loses the tail of 'g', 'j', 'p', 'q', 'y'.
        for c in ['g', 'j', 'p', 'q', 'y'] {
            let (_, cols) = font::FONT.iter().find(|(g, _)| *g == c).unwrap();
            assert!(
                cols.iter().any(|col| col & (1 << 6) != 0),
                "'{c}' has no ink on the bottom row, so its descender was lost"
            );
        }
    }
}
