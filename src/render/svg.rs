// SPDX-License-Identifier: Apache-2.0
//! Scene → SVG. Self-contained: no external stylesheet, no font file, nothing to fetch, so the
//! output works from a `file://` URL and survives being committed to a repository.

use super::{Rgb, Scene, Shape, CREDIT_COLOR, CREDIT_INSET, CREDIT_SIZE};
use std::fmt::Write as _;

fn hex((r, g, b): Rgb) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// Escape the five characters that would otherwise change the document's structure.
///
/// Instance names in a real design carry `<`, `>` and `&` often enough that skipping this
/// produces a file that silently fails to parse in a browser.
fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&apos;"),
            _ => o.push(c),
        }
    }
    o
}

pub fn to_svg(s: &Scene) -> String {
    let mut o = String::new();
    let _ = write!(
        o,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{:.0}\" height=\"{:.0}\" \
         viewBox=\"0 0 {:.0} {:.0}\" \
         font-family=\"ui-sans-serif,system-ui,-apple-system,sans-serif\">\n",
        s.width, s.height, s.width, s.height
    );
    if !s.title.is_empty() {
        let _ = write!(o, "<title>{}</title>\n", esc(&s.title));
    }
    let _ = write!(
        o,
        "<rect width=\"100%\" height=\"100%\" fill=\"{}\"/>\n",
        hex(s.background)
    );

    for shape in &s.shapes {
        match shape {
            Shape::Rect {
                x,
                y,
                w,
                h,
                fill,
                fill_opacity,
                stroke,
                stroke_width,
            } => {
                let _ = write!(o, "<rect x=\"{x:.1}\" y=\"{y:.1}\" width=\"{w:.1}\" height=\"{h:.1}\"");
                match fill {
                    Some(c) => {
                        let _ = write!(o, " fill=\"{}\" fill-opacity=\"{fill_opacity:.2}\"", hex(*c));
                    }
                    None => {
                        let _ = write!(o, " fill=\"none\"");
                    }
                }
                if let Some(c) = stroke {
                    let _ = write!(o, " stroke=\"{}\" stroke-width=\"{stroke_width:.1}\"", hex(*c));
                }
                let _ = write!(o, "/>\n");
            }
            Shape::Line {
                x1,
                y1,
                x2,
                y2,
                stroke,
                width,
                dashed,
            } => {
                let _ = write!(
                    o,
                    "<line x1=\"{x1:.1}\" y1=\"{y1:.1}\" x2=\"{x2:.1}\" y2=\"{y2:.1}\" \
                     stroke=\"{}\" stroke-width=\"{width:.1}\"{}/>\n",
                    hex(*stroke),
                    if *dashed { " stroke-dasharray=\"4 2\"" } else { "" }
                );
            }
            Shape::Text {
                x,
                y,
                text,
                size,
                fill,
                anchor,
                bold,
            } => {
                let _ = write!(
                    o,
                    "<text x=\"{x:.1}\" y=\"{y:.1}\" font-size=\"{size:.1}\" fill=\"{}\" \
                     text-anchor=\"{}\"{}>{}</text>\n",
                    hex(*fill),
                    anchor.svg(),
                    if *bold { " font-weight=\"600\"" } else { "" },
                    esc(text)
                );
            }
        }
    }
    // Last, so it is never painted over by the scene's own shapes.
    if let Some(c) = &s.credit {
        let _ = write!(
            o,
            "<text x=\"{:.1}\" y=\"{:.1}\" font-size=\"{CREDIT_SIZE:.1}\" fill=\"{}\" \
             text-anchor=\"end\">{}</text>\n",
            s.width - CREDIT_INSET,
            s.height - CREDIT_INSET,
            hex(CREDIT_COLOR),
            esc(c)
        );
    }
    o.push_str("</svg>\n");
    o
}

#[cfg(test)]
mod tests {
    use super::super::tests::sample;
    use super::*;

    #[test]
    fn it_is_a_well_formed_document() {
        let svg = to_svg(&sample());
        assert!(svg.starts_with("<svg"));
        assert!(svg.trim_end().ends_with("</svg>"));
    }

    #[test]
    fn nothing_is_fetched_from_outside_the_file() {
        // The claim "self-contained" is what makes the output safe to commit and to open
        // offline; a stylesheet or font reference would quietly break both.
        let svg = to_svg(&sample());
        assert!(!svg.contains("xlink:href"));
        assert!(!svg.contains("<image"));
        assert!(!svg.contains("@import"));
    }

    #[test]
    fn markup_in_a_label_cannot_break_the_document() {
        let mut s = sample();
        s.push(Shape::text(0.0, 0.0, "a<b>&\"c\"", 10.0, (0, 0, 0)));
        let svg = to_svg(&s);
        assert!(svg.contains("a&lt;b&gt;&amp;&quot;c&quot;"));
        assert!(!svg.contains("<b>"));
    }

    #[test]
    fn a_title_becomes_the_documents_title() {
        let s = sample().with_title("stack & co");
        assert!(to_svg(&s).contains("<title>stack &amp; co</title>"));
    }
}
