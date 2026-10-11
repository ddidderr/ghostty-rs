//! Row heuristics for the renderer (`renderer/row.zig`).

use ghostty_terminal::page::{Cell, ContentTag, Row, SemanticPrompt};
use ghostty_terminal::style::Style;
use ghostty_vt::color::{Palette, Rgb};

/// Returns true if the row of this pin should never have its background
/// color extended for filling padding space in the renderer. This is
/// a set of heuristics that help making our padding look better.
///
/// `styles` is the (possibly empty) style array of the render-state row;
/// when empty, every cell has the default style.
pub fn never_extend_bg(
    row: Row,
    cells: &[Cell],
    styles: &[Style],
    palette: &Palette,
    default_background: Rgb,
) -> bool {
    // Any semantic prompts should not have their background extended
    // because prompts often contain special formatting (such as
    // powerline) that looks bad when extended.
    match row.semantic_prompt() {
        SemanticPrompt::Prompt | SemanticPrompt::PromptContinuation => return true,
        SemanticPrompt::None => {}
    }

    for (x, cell) in cells.iter().enumerate() {
        // If any cell has a default background color then we don't
        // extend because the default background color probably looks
        // good enough as an extension.
        match cell.content_tag() {
            // If it is a background color cell, we check the color.
            ContentTag::BgColorPalette | ContentTag::BgColorRgb => {
                let s = style_at(*cell, styles, x);
                let Some(bg) = s.bg(cell, palette) else {
                    return true;
                };
                if bg == default_background {
                    return true;
                }
            }

            // If its a codepoint cell we can check the style.
            ContentTag::Codepoint | ContentTag::CodepointGrapheme => {
                // For codepoint containing, we also never extend bg
                // if any cell has a powerline glyph because these are
                // perfect-fit.
                if matches!(
                    cell.codepoint(),
                    // Powerline
                    0xE0B0..=0xE0C8 | 0xE0CA | 0xE0CC..=0xE0D2 | 0xE0D4
                ) {
                    return true;
                }

                // Never extend a cell that has a default background.
                // A default background is applied if there is no background
                // on the style or the explicitly set background
                // matches our default background.
                let s = style_at(*cell, styles, x);
                let Some(bg) = s.bg(cell, palette) else {
                    return true;
                };
                if bg == default_background {
                    return true;
                }
            }
        }
    }

    false
}

fn style_at(cell: Cell, styles: &[Style], x: usize) -> Style {
    if cell.has_styling() {
        styles.get(x).copied().unwrap_or_default()
    } else {
        Style::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ghostty_vt::color;

    const BG: Rgb = Rgb { r: 1, g: 2, b: 3 };

    #[test]
    fn default_background_cells_never_extend() {
        let cells = [Cell::init(u32::from('a'))];
        assert!(never_extend_bg(
            Row::default(),
            &cells,
            &[],
            &color::DEFAULT,
            BG
        ));
    }

    #[test]
    fn prompt_rows_never_extend() {
        let mut row = Row::default();
        row.set_semantic_prompt(SemanticPrompt::Prompt);
        assert!(never_extend_bg(row, &[], &[], &color::DEFAULT, BG));
    }

    #[test]
    fn powerline_never_extends() {
        let cells = [Cell::init(0xE0B0)];
        assert!(never_extend_bg(
            Row::default(),
            &cells,
            &[],
            &color::DEFAULT,
            BG
        ));
    }

    #[test]
    fn explicit_bg_color_cell_extends() {
        let mut c = Cell::default();
        c.set_content_tag(ContentTag::BgColorRgb);
        c.set_color_rgb(Rgb { r: 9, g: 9, b: 9 });
        assert!(!never_extend_bg(
            Row::default(),
            &[c],
            &[],
            &color::DEFAULT,
            BG
        ));
        // ... unless it equals the default background.
        c.set_color_rgb(BG);
        assert!(never_extend_bg(
            Row::default(),
            &[c],
            &[],
            &color::DEFAULT,
            BG
        ));
    }
}
