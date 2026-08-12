//! Column-aligned output.
//!
//! Hand-rolled because a table crate would be more dependency than the two
//! tables this CLI prints.

/// Print a header and rows, sizing each column to its widest cell.
pub fn print<const N: usize>(headers: [&str; N], rows: &[[String; N]]) {
    let mut widths: [usize; N] = std::array::from_fn(|i| display_width(headers[i]));

    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            widths[i] = widths[i].max(display_width(cell));
        }
    }

    println!("{}", line(&headers.map(str::to_string), &widths));
    for row in rows {
        println!("{}", line(row, &widths));
    }
}

fn line<const N: usize>(cells: &[String; N], widths: &[usize; N]) -> String {
    let mut out = String::new();
    for (i, cell) in cells.iter().enumerate() {
        if i > 0 {
            out.push_str("  ");
        }
        out.push_str(cell);
        // The last column never needs padding, and trailing spaces make output
        // annoying to pipe.
        if i + 1 < N {
            for _ in 0..widths[i].saturating_sub(display_width(cell)) {
                out.push(' ');
            }
        }
    }
    out
}

/// App names come from folder names, so they can contain anything. Counting
/// chars rather than bytes keeps non-ASCII names aligned.
fn display_width(s: &str) -> usize {
    s.chars().count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns_are_sized_to_the_widest_cell() {
        let widths = [3usize, 5];
        let row = ["ab".to_string(), "cd".to_string()];
        assert_eq!(line(&row, &widths), "ab   cd");
    }

    #[test]
    fn the_last_column_has_no_trailing_padding() {
        let widths = [2usize, 10];
        let row = ["ab".to_string(), "cd".to_string()];
        assert_eq!(line(&row, &widths), "ab  cd");
    }

    #[test]
    fn non_ascii_names_stay_aligned() {
        // Counting bytes would over-pad "café" by one.
        assert_eq!(display_width("café"), 4);
        assert_eq!(display_width("cafe"), 4);
    }
}
