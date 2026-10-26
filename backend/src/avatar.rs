//! Sticker avatars: a small vector drawing on a 100x100 canvas.
//!
//! Format (also what agents send through the `set_avatar` tool):
//! - `svg`: `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">…</svg>`
//!   using plain shapes, paths and gradients.
//!
//! The UI cuts a white die-cut border around the silhouette and adds a shadow,
//! so the art is just the character. Older 32x32 pixel avatars are converted
//! to SVG when read.

use std::fmt::Write as _;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

const MAX_BYTES: usize = 24_000;
const MAX_ELEMENTS: usize = 300;

const ELEMENTS: &[&str] = &[
    "svg", "g", "defs", "path", "circle", "ellipse", "rect", "polygon", "polyline", "line", "linearGradient",
    "radialGradient", "stop", "clipPath", "mask",
];

const ATTRIBUTES: &[&str] = &[
    "xmlns", "viewBox", "id", "d", "cx", "cy", "r", "rx", "ry", "x", "y", "width", "height", "points", "x1", "y1", "x2",
    "y2", "fx", "fy", "offset", "transform", "fill", "fill-opacity", "fill-rule", "stroke", "stroke-width",
    "stroke-opacity", "stroke-linecap", "stroke-linejoin", "stroke-miterlimit", "stroke-dasharray",
    "stroke-dashoffset", "opacity", "stop-color", "stop-opacity", "gradientUnits", "gradientTransform",
    "spreadMethod", "clip-path", "clip-rule", "clipPathUnits", "mask", "maskUnits", "maskContentUnits",
    "paint-order", "shape-rendering",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(from = "Stored")]
pub struct Avatar {
    pub svg: String,
}

/// Avatars as they may be stored: current SVG, or the old 32x32 pixel grid.
#[derive(Deserialize)]
#[serde(untagged)]
enum Stored {
    Svg { svg: String },
    Pixels { palette: Vec<String>, pixels: Vec<String> },
}

impl From<Stored> for Avatar {
    fn from(s: Stored) -> Self {
        match s {
            Stored::Svg { svg } => Avatar { svg },
            Stored::Pixels { palette, pixels } => Avatar { svg: pixels_to_svg(&palette, &pixels) },
        }
    }
}

/// Old pixel art as crisp rects, one per horizontal run of a color.
fn pixels_to_svg(palette: &[String], pixels: &[String]) -> String {
    const KEYS: &str = "0123456789abcdefghijklmnopqrstuvwxyz";
    let mut out = String::from(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><g transform="scale(3.125)" shape-rendering="crispEdges">"#);
    for (y, row) in pixels.iter().enumerate() {
        let row: Vec<char> = row.chars().collect();
        let mut x = 0;
        while x < row.len() {
            let ch = row[x];
            let run = row[x..].iter().take_while(|c| **c == ch).count();
            if let Some(color) = KEYS.find(ch).and_then(|i| palette.get(i)).filter(|c| valid_color(c)) {
                let _ = write!(out, r#"<rect x="{x}" y="{y}" width="{run}" height="1" fill="{color}"/>"#);
            }
            x += run;
        }
    }
    out.push_str("</g></svg>");
    out
}

fn valid_color(c: &str) -> bool {
    let h = c.trim_start_matches('#');
    c.starts_with('#') && matches!(h.len(), 3 | 6 | 8) && h.chars().all(|c| c.is_ascii_hexdigit())
}

impl Avatar {
    /// Validate an avatar supplied by an agent or the UI, returning it in canonical form:
    /// a bare `<svg viewBox="0 0 100 100">` root around the original drawing.
    pub fn validate(self) -> Result<Self> {
        let src = self.svg.trim();
        if src.len() > MAX_BYTES {
            bail!("the SVG is {} bytes; keep it under {MAX_BYTES} (a sticker is a few bold shapes)", src.len());
        }
        let doc = roxmltree::Document::parse(src).map_err(|e| anyhow::anyhow!("not valid SVG/XML: {e}"))?;
        let root = doc.root_element();
        if root.tag_name().name() != "svg" {
            bail!("the root element must be <svg>");
        }
        let view_box: Vec<f64> = root
            .attribute("viewBox")
            .unwrap_or_default()
            .split(|c: char| c.is_whitespace() || c == ',')
            .filter(|s| !s.is_empty())
            .filter_map(|s| s.parse().ok())
            .collect();
        if view_box != [0.0, 0.0, 100.0, 100.0] {
            bail!("the root <svg> must have viewBox=\"0 0 100 100\"");
        }
        let mut count = 0;
        let mut shapes = 0;
        for node in root.descendants() {
            if node.is_pi() {
                bail!("processing instructions aren't allowed");
            }
            if !node.is_element() {
                continue;
            }
            count += 1;
            let name = node.tag_name().name();
            if !ELEMENTS.contains(&name) {
                bail!(
                    "<{name}> isn't allowed. Use only {}; draw everything as shapes (no text, images or styles)",
                    ELEMENTS.join(", ")
                );
            }
            if ["path", "circle", "ellipse", "rect", "polygon", "polyline", "line"].contains(&name) {
                shapes += 1;
            }
            for attr in node.attributes() {
                let (key, value) = (attr.name(), attr.value());
                if attr.namespace().is_some_and(|ns| ns != "http://www.w3.org/2000/svg") || !ATTRIBUTES.contains(&key) {
                    bail!("attribute {key:?} on <{name}> isn't allowed; use presentation attributes like fill and stroke");
                }
                if key == "id" && value.starts_with("hb-") {
                    bail!("ids starting with \"hb-\" are reserved");
                }
                let lower = value.to_ascii_lowercase();
                if lower.contains("url(") && !lower.trim().starts_with("url(#") {
                    bail!("{key}={value:?}: only local references like url(#id) are allowed");
                }
                if lower.contains("javascript:") || lower.contains("data:") || lower.contains("http") {
                    bail!("{key}={value:?}: external references aren't allowed");
                }
            }
        }
        if count > MAX_ELEMENTS {
            bail!("the SVG has {count} elements; keep it under {MAX_ELEMENTS}");
        }
        if shapes == 0 {
            bail!("the SVG doesn't draw anything");
        }
        let inner = match (root.first_child(), root.last_child()) {
            (Some(first), Some(last)) => &src[first.range().start..last.range().end],
            _ => "",
        };
        Ok(Avatar { svg: format!(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">{inner}</svg>"#) })
    }

    /// Deterministic little sticker character derived from a seed string:
    /// a bold flat shape whose whole body is its face, sometimes wearing a hat.
    pub fn generate(seed: &str) -> Self {
        let mut rng = Rng::new(seed);
        let (base, shade) = *rng.pick(BODIES);
        let shape = rng.pick(SHAPES);
        let body = (shape.draw)(&mut rng);
        let mut out = String::new();

        // Flat body with a slightly darker lower edge: draw it in the shade color,
        // then again in the base color nudged up, clipped to the body.
        let _ = write!(
            out,
            r##"<defs><clipPath id="body">{body}</clipPath></defs><g fill="{shade}">{body}</g><g clip-path="url(#body)"><g fill="{base}" transform="translate(-1 -5)">{body}</g></g>"##
        );

        if shape.hat_top > 0.0 && rng.range(0, 100) < 55 {
            let (acc, acc_shade) = loop {
                let pick = *rng.pick(BODIES);
                if pick.0 != base {
                    break pick;
                }
            };
            let hat = rng.pick(HATS).replace("{a}", acc).replace("{A}", acc_shade);
            let _ = write!(out, r#"<g transform="translate(50 {})">{hat}</g>"#, shape.hat_top);
        }

        // Face.
        let (ey, gap, scale) = (shape.eye_y, shape.eye_gap, shape.eye_scale);
        let (lx, rx) = (50.0 - gap / 2.0, 50.0 + gap / 2.0);
        let (ew, eh) = (12.5 * scale, 14.0 * scale);
        let look = *rng.pick(&[(-1.0, -1.0), (1.0, -1.0), (0.0, -1.0), (-1.0, -0.3), (1.0, -0.3), (0.5, -0.9), (-0.5, -0.9)]);
        let tilt = look.0 * 10.0;
        let eye = |out: &mut String, cx: f64, look: (f64, f64)| {
            let (px, py) = (cx + look.0 * ew * 0.32, ey + look.1 * eh * 0.3);
            let _ = write!(
                out,
                r##"<ellipse cx="{cx:.1}" cy="{ey:.1}" rx="{ew:.1}" ry="{eh:.1}" fill="#fff"/><ellipse cx="{px:.1}" cy="{py:.1}" rx="{:.1}" ry="{:.1}" transform="rotate({tilt:.0} {px:.1} {py:.1})" fill="{INK}"/>"##,
                5.2 * scale,
                7.2 * scale
            );
        };
        let stroke = |out: &mut String, d: String| {
            let _ = write!(
                out,
                r#"<path d="{d}" fill="none" stroke="{INK}" stroke-width="{:.1}" stroke-linecap="round" stroke-linejoin="round"/>"#,
                4.2 * scale
            );
        };
        match rng.range(0, 100) {
            0..72 => {
                eye(&mut out, lx, look);
                eye(&mut out, rx, look);
            }
            72..86 => {
                // Wink: one googly eye, the other squeezed shut toward it.
                let left_open = rng.range(0, 2) == 0;
                let (open, shut, dir) = if left_open { (lx, rx, 1.0) } else { (rx, lx, -1.0) };
                eye(&mut out, open, (dir, -0.8));
                let (w, h) = (5.0 * scale, 6.0 * scale);
                stroke(&mut out, format!("M{:.1} {:.1} L{:.1} {ey:.1} L{:.1} {:.1}", shut - dir * w, ey - h, shut + dir * w, shut - dir * w, ey + h));
            }
            86..93 => {
                for cx in [lx, rx] {
                    let w = 7.0 * scale;
                    stroke(&mut out, format!("M{:.1} {:.1} Q{cx:.1} {:.1} {:.1} {:.1}", cx - w, ey + 3.0, ey - 9.0 * scale, cx + w, ey + 3.0));
                }
            }
            _ => {
                for cx in [lx, rx] {
                    let w = 7.0 * scale;
                    stroke(&mut out, format!("M{:.1} {:.1} Q{cx:.1} {:.1} {:.1} {:.1}", cx - w, ey - 2.0, ey + 7.0 * scale, cx + w, ey - 2.0));
                }
            }
        }
        if rng.range(0, 100) < 35 {
            let blush = if base == "#ff7eb6" { "#ff4f8b" } else { "#ff8fb1" };
            for cx in [lx - ew * 0.75, rx + ew * 0.75] {
                let _ = write!(out, r#"<ellipse cx="{cx:.1}" cy="{:.1}" rx="5" ry="3" fill="{blush}" opacity=".7"/>"#, ey + eh + 3.0);
            }
        }
        let my = ey + eh + 6.0 * scale;
        let w = 6.5 * scale;
        match rng.range(0, 5) {
            0 => stroke(&mut out, format!("M{:.1} {my:.1} Q50 {:.1} {:.1} {my:.1}", 50.0 - w, my + 6.0 * scale, 50.0 + w)),
            1 => stroke(&mut out, format!("M{:.1} {:.1} Q{:.1} {:.1} {:.1} {:.1}", 50.0 - w, my + 1.0, 50.0 + w * 0.2, my + 4.0, 50.0 + w, my - 3.0)),
            2 => stroke(&mut out, format!("M{:.1} {:.1} Q{:.1} {:.1} {:.1} {:.1}", 50.0 + w, my + 1.0, 50.0 - w * 0.2, my + 4.0, 50.0 - w, my - 3.0)),
            3 => {
                // Open mouth with a tongue.
                let (l, r, b) = (50.0 - w, 50.0 + w, my + 8.0 * scale);
                let _ = write!(
                    out,
                    r#"<path d="M{l:.1} {my:.1} H{r:.1} Q{r:.1} {b:.1} 50 {b:.1} Q{l:.1} {b:.1} {l:.1} {my:.1} Z" fill="{INK}" stroke="{INK}" stroke-width="2" stroke-linejoin="round"/><ellipse cx="50" cy="{:.1}" rx="{:.1}" ry="{:.1}" fill="{TONGUE}"/>"#,
                    my + 5.6 * scale,
                    3.6 * scale,
                    2.2 * scale
                );
            }
            _ => stroke(&mut out, format!("M{:.1} {my:.1} Q{:.1} {:.1} 50 {my:.1} Q{:.1} {:.1} {:.1} {my:.1}", 50.0 - w, 50.0 - w / 2.0, my + 4.0, 50.0 + w / 2.0, my + 4.0, 50.0 + w)),
        }
        Avatar { svg: format!(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">{out}</svg>"#) }
    }
}

// ---- generator ----------------------------------------------------------

const INK: &str = "#17171c";
const TONGUE: &str = "#ff7a8a";

/// Flat body colors and the shade used on their lower edge.
const BODIES: &[(&str, &str)] = &[
    ("#3d9bff", "#2b7fe0"), // blue
    ("#ffc93c", "#e5a812"), // yellow
    ("#ff5a5a", "#d93e3e"), // red
    ("#34d399", "#20b07c"), // green
    ("#8b5cf6", "#6d3fe0"), // purple
    ("#ff8a3d", "#e06a1f"), // orange
    ("#ff7eb6", "#e05a95"), // pink
    ("#2ccfcf", "#1aa8a8"), // teal
    ("#9a9aa6", "#787884"), // gray
    ("#b9ec5a", "#98cf3c"), // lime
];

struct Shape {
    /// Markup for the body silhouette, without a fill (the caller sets it).
    draw: fn(&mut Rng) -> String,
    eye_y: f64,
    eye_gap: f64,
    eye_scale: f64,
    /// Where a hat sits on the head, or 0 when the top is too pointy for one.
    hat_top: f64,
}

const SHAPES: &[Shape] = &[
    // squircle
    Shape {
        draw: |r| {
            let rx = 24 + r.range(0, 12);
            format!(r#"<rect x="10" y="22" width="80" height="70" rx="{rx}"/>"#)
        },
        eye_y: 52.0,
        eye_gap: 30.0,
        eye_scale: 1.05,
        hat_top: 23.0,
    },
    // triangle
    Shape {
        draw: |_| path(&rounded_polygon(&[(50.0, 8.0), (95.0, 88.0), (5.0, 88.0)], 15.0)),
        eye_y: 60.0,
        eye_gap: 26.0,
        eye_scale: 0.92,
        hat_top: 0.0,
    },
    // star
    Shape {
        draw: |r| {
            let rot = -90.0 + r.range(0, 9) as f64 - 4.0;
            let pts: Vec<(f64, f64)> = (0..10)
                .map(|i| {
                    let a = (rot + i as f64 * 36.0).to_radians();
                    let rad = if i % 2 == 0 { 48.0 } else { 26.0 };
                    (50.0 + rad * a.cos(), 54.0 + rad * a.sin())
                })
                .collect();
            path(&rounded_polygon(&pts, 6.0))
        },
        eye_y: 54.0,
        eye_gap: 22.0,
        eye_scale: 0.8,
        hat_top: 0.0,
    },
    // drop
    Shape {
        draw: |_| r#"<path d="M50 5 C58 18 88 40 88 62 A38 38 0 0 1 12 62 C12 40 42 18 50 5 Z"/>"#.into(),
        eye_y: 60.0,
        eye_gap: 28.0,
        eye_scale: 1.0,
        hat_top: 0.0,
    },
    // map pin
    Shape {
        draw: |_| r#"<path d="M50 97 C42 86 14 70 14 44 A36 36 0 1 1 86 44 C86 70 58 86 50 97 Z"/>"#.into(),
        eye_y: 44.0,
        eye_gap: 30.0,
        eye_scale: 1.0,
        hat_top: 0.0,
    },
    // gear
    Shape {
        draw: |r| {
            let rot = r.range(0, 45) as f64;
            let pts: Vec<(f64, f64)> = (0..32)
                .map(|i| {
                    let a = (rot + i as f64 * 11.25).to_radians();
                    let rad = if (i / 2) % 2 == 0 { 45.0 } else { 37.0 };
                    (50.0 + rad * a.cos(), 54.0 + rad * a.sin())
                })
                .collect();
            path(&rounded_polygon(&pts, 3.0))
        },
        eye_y: 52.0,
        eye_gap: 28.0,
        eye_scale: 1.0,
        hat_top: 0.0,
    },
    // ghost
    Shape {
        draw: |_| r#"<path d="M14 52 A36 36 0 0 1 86 52 V90 Q80 97 74 90 T62 90 T50 90 T38 90 T26 90 T14 90 Z"/>"#.into(),
        eye_y: 50.0,
        eye_gap: 30.0,
        eye_scale: 1.05,
        hat_top: 17.0,
    },
    // cloud
    Shape {
        draw: |_| {
            r#"<circle cx="30" cy="62" r="21"/><circle cx="70" cy="62" r="21"/><circle cx="50" cy="45" r="27"/><ellipse cx="50" cy="70" rx="40" ry="19"/>"#.into()
        },
        eye_y: 56.0,
        eye_gap: 28.0,
        eye_scale: 1.0,
        hat_top: 19.0,
    },
    // egg
    Shape {
        draw: |r| format!(r#"<ellipse cx="50" cy="55" rx="{}" ry="41"/>"#, 36 + r.range(0, 6)),
        eye_y: 50.0,
        eye_gap: 29.0,
        eye_scale: 1.0,
        hat_top: 15.0,
    },
];

/// Hats drawn around (0, 0) = the top of the head; `{a}`/`{A}` are an accent color and its shade.
const HATS: &[&str] = &[
    // sprout
    r##"<path d="M0 2 C0 -6 1 -10 3 -13" fill="none" stroke="#3aa437" stroke-width="3.5" stroke-linecap="round"/><path d="M2 -11 C4 -21 16 -22 20 -18 C16 -12 8 -9 2 -11 Z" fill="#4fc94a"/><path d="M1 -9 C-2 -17 -12 -19 -16 -15 C-12 -10 -5 -7 1 -9 Z" fill="#4fc94a"/>"##,
    // antenna
    r##"<path d="M0 4 V-10" stroke="{A}" stroke-width="4" stroke-linecap="round"/><circle cx="0" cy="-13" r="6.5" fill="{a}"/><circle cx="-2" cy="-15" r="2" fill="#fff" opacity=".6"/>"##,
    // bow
    r##"<g transform="translate(16 2) rotate(14)"><path d="M0 0 C-6 -10 -18 -10 -18 0 C-18 10 -6 10 0 0 Z" fill="{a}"/><path d="M0 0 C6 -10 18 -10 18 0 C18 10 6 10 0 0 Z" fill="{a}"/><circle r="5" fill="{A}"/></g>"##,
    // party hat
    r##"<path d="M-12 4 L0 -21 L12 4 Z" fill="{a}" stroke="{a}" stroke-width="4" stroke-linejoin="round"/><path d="M-8 -5 L8 -1 M-5 -13 L5 -10" stroke="#fff" stroke-width="3.5" stroke-linecap="round" opacity=".75"/><circle cx="0" cy="-22" r="4.5" fill="#ffd04a"/>"##,
    // crown
    r##"<path d="M-17 4 L-19 -15 L-8 -6 L0 -19 L8 -6 L19 -15 L17 4 Z" fill="#ffd04a" stroke="#ffd04a" stroke-width="3" stroke-linejoin="round"/><path d="M-17 1 H17" stroke="#e0a920" stroke-width="4" stroke-linecap="round"/><circle cx="0" cy="-6" r="2.6" fill="{a}"/>"##,
];

fn path(d: &str) -> String {
    format!(r#"<path d="{d}"/>"#)
}

/// A closed path through `pts` with each corner rounded off by a quadratic curve.
fn rounded_polygon(pts: &[(f64, f64)], r: f64) -> String {
    let n = pts.len();
    let toward = |a: (f64, f64), b: (f64, f64)| {
        let len = (b.0 - a.0).hypot(b.1 - a.1);
        let t = (r / len).min(0.5);
        (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
    };
    let mut d = String::new();
    for i in 0..n {
        let (prev, v, next) = (pts[(i + n - 1) % n], pts[i], pts[(i + 1) % n]);
        let (a, b) = (toward(v, prev), toward(v, next));
        let _ = write!(d, "{}{:.1} {:.1} Q{:.1} {:.1} {:.1} {:.1} ", if i == 0 { "M" } else { "L" }, a.0, a.1, v.0, v.1, b.0, b.1);
    }
    d.push('Z');
    d
}

struct Rng(u64);

impl Rng {
    fn new(seed: &str) -> Self {
        // FNV-1a
        let mut h: u64 = 0xcbf29ce484222325;
        for b in seed.bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        Rng(h | 1)
    }
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    /// Uniform in [lo, hi)
    fn range(&mut self, lo: u32, hi: u32) -> u32 {
        if hi <= lo {
            return lo;
        }
        lo + (self.next() % (hi - lo) as u64) as u32
    }
    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.range(0, items.len() as u32) as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn svg(inner: &str) -> Avatar {
        Avatar { svg: format!(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">{inner}</svg>"#) }
    }

    #[test]
    fn generated_avatars_are_valid() {
        for i in 0..500 {
            let a = Avatar::generate(&format!("seed-{i}"));
            a.clone().validate().unwrap_or_else(|e| panic!("seed-{i}: {e}\n{}", a.svg));
        }
    }

    #[test]
    fn validate_keeps_drawings_and_canonicalises_the_root() {
        let a = Avatar { svg: r##"<svg viewBox="0,0,100,100" width="512" xmlns="http://www.w3.org/2000/svg"><circle cx="50" cy="50" r="40" fill="#3d9bff"/></svg>"##.into() };
        assert_eq!(
            a.validate().unwrap().svg,
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><circle cx="50" cy="50" r="40" fill="#3d9bff"/></svg>"##
        );
        assert!(svg(r##"<defs><linearGradient id="g"><stop offset="0" stop-color="#fff"/></linearGradient></defs><rect width="10" height="10" fill="url(#g)"/>"##).validate().is_ok());
    }

    #[test]
    fn validate_rejects_unsafe_or_off_format_svg() {
        for bad in [
            r#"<script>alert(1)</script><circle r="5"/>"#,
            r#"<circle r="5" onload="alert(1)"/>"#,
            r#"<image href="https://example.com/x.png"/>"#,
            r#"<circle r="5" fill="url(https://example.com/#x)"/>"#,
            r#"<circle r="5" style="fill:red"/>"#,
            r#"<text>hi</text>"#,
            r#"<foreignObject/>"#,
            r#"<g/>"#,
        ] {
            assert!(svg(bad).validate().is_err(), "accepted {bad}");
        }
        let wrong_box = Avatar { svg: r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32"><circle r="5"/></svg>"#.into() };
        assert!(wrong_box.validate().is_err());
        let entity = Avatar { svg: r#"<!DOCTYPE svg [<!ENTITY x "y">]><svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><circle r="5"/></svg>"#.into() };
        assert!(entity.validate().is_err());
    }

    #[test]
    fn old_pixel_avatars_still_load() {
        let json = r##"{"palette":["#ff0000"],"pixels":["00..","..0."]}"##;
        let a: Avatar = serde_json::from_str(json).unwrap();
        assert!(a.svg.contains(r##"<rect x="0" y="0" width="2" height="1" fill="#ff0000"/>"##));
        assert!(a.svg.contains(r##"<rect x="2" y="1" width="1" height="1" fill="#ff0000"/>"##));
    }
}
