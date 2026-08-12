//! QR codes in a terminal.
//!
//! Rendered with half-block characters so one text row carries two QR rows;
//! a full-block rendering is twice as tall and scrolls off an 80x24 terminal
//! for any realistic URL.

use qrcode::{types::Color, QrCode};

const UPPER: char = '\u{2580}'; // ▀
const LOWER: char = '\u{2584}'; // ▄
const FULL: char = '\u{2588}'; // █
const BLANK: char = ' ';

/// A scannable code, indented and with the quiet zone scanners need.
pub fn render(data: &str) -> String {
    let Ok(code) = QrCode::new(data.as_bytes()) else {
        // Nothing sensible to draw; the caller still prints the URL.
        return String::new();
    };

    let colors = code.to_colors();
    let width = code.width();

    // Four modules of quiet zone on every side, per the spec. Without it many
    // phone cameras simply will not lock on.
    const QUIET: usize = 4;
    let padded = width + QUIET * 2;
    let dark = |x: usize, y: usize| -> bool {
        if x < QUIET || y < QUIET || x >= width + QUIET || y >= width + QUIET {
            return false;
        }
        colors[(y - QUIET) * width + (x - QUIET)] == Color::Dark
    };

    let mut out = String::new();
    for row in (0..padded).step_by(2) {
        out.push_str("  ");
        for x in 0..padded {
            let top = dark(x, row);
            let bottom = row + 1 < padded && dark(x, row + 1);
            // Terminals render dark-on-light, so a dark module is a drawn
            // block: upper half, lower half, both, or neither.
            out.push(match (top, bottom) {
                (true, true) => FULL,
                (true, false) => UPPER,
                (false, true) => LOWER,
                (false, false) => BLANK,
            });
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_something_scannable_shaped() {
        let out = render("http://trip-planner.local");
        assert!(!out.is_empty());

        let lines: Vec<_> = out.lines().collect();
        assert!(lines.len() > 8, "a QR code should be more than a few rows");

        // Every row is the same width, or the code is skewed and unreadable.
        let widths: Vec<usize> = lines.iter().map(|l| l.chars().count()).collect();
        assert!(
            widths.windows(2).all(|w| w[0] == w[1]),
            "rows differ in width: {widths:?}"
        );
    }

    #[test]
    fn the_quiet_zone_is_present_on_every_side() {
        let out = render("http://a.local");
        let lines: Vec<&str> = out.lines().collect();

        assert!(
            lines[0].trim().is_empty(),
            "the first rows must be blank quiet zone"
        );
        assert!(
            lines[lines.len() - 1].trim().is_empty(),
            "the last rows must be blank quiet zone"
        );
        for line in &lines {
            assert!(
                line.starts_with("    "),
                "each row needs a left quiet zone: {line:?}"
            );
        }
    }

    #[test]
    fn a_longer_url_makes_a_bigger_code() {
        let small = render("http://a.local").lines().count();
        let large = render(&format!("http://{}.local", "x".repeat(200)))
            .lines()
            .count();
        assert!(large > small);
    }

    #[test]
    fn undrawable_data_returns_empty_rather_than_panicking() {
        // Beyond QR's capacity at any version.
        assert_eq!(render(&"x".repeat(10_000)), "");
    }
}
