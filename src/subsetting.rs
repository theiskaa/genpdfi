//! Font subsetting module using the subsetter crate.
//!
//! This module provides functionality to create subset fonts that contain only
//! the glyphs actually used in a document, significantly reducing PDF file sizes.

use crate::error::{Error, ErrorKind};
use std::collections::HashSet;
use subsetter::{subset, GlyphRemapper};
use ttf_parser::Face;

/// Creates a subset of a font containing only the specified characters.
///
/// # Arguments
/// * `font_data` - The original font file data (TTF/OTF)
/// * `text` - The text containing all characters to include in the subset
///
/// # Returns
/// * `Ok(Vec<u8>)` - The subset font data
/// * `Err(Error)` - If subsetting fails
///
/// # Example
/// ```rust,no_run
/// use genpdfi::subsetting::subset_font;
///
/// let font_data = std::fs::read("font.ttf").unwrap();
/// let text = "Hello World ăâîșț";  // Romanian characters
/// let subset = subset_font(&font_data, text).unwrap();
///
/// // subset now contains a smaller font with only the used glyphs
/// assert!(subset.len() < font_data.len());
/// ```
pub fn subset_font(font_data: &[u8], text: &str) -> Result<Vec<u8>, Error> {
    let face = Face::parse(font_data, 0).map_err(|e| {
        Error::new(
            format!("Failed to parse font: {:?}", e),
            ErrorKind::InvalidFont,
        )
    })?;

    let mut remapper = GlyphRemapper::new();
    remapper.remap(0);

    for ch in text.chars() {
        if let Some(glyph_id) = face.glyph_index(ch) {
            remapper.remap(glyph_id.0);
        }
    }

    let result = subset(font_data, 0, &remapper).map_err(|e| {
        Error::new(
            format!("Font subsetting failed: {:?}", e),
            ErrorKind::InvalidFont,
        )
    })?;

    Ok(result)
}

/// Collects all unique characters from a string.
///
/// This is useful for determining which characters are actually used
/// in a document before creating a subset.
///
/// # Example
/// ```
/// use genpdfi::subsetting::collect_used_chars;
///
/// let text = "Hello World! Hello again!";
/// let chars = collect_used_chars(text);
/// assert_eq!(chars.len(), 13);  // H, e, l, o, space, W, r, d, !, a, g, i, n
/// ```
pub fn collect_used_chars(text: &str) -> HashSet<char> {
    text.chars().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_collect_used_chars() {
        let text = "Hello World!";
        let chars = collect_used_chars(text);

        assert!(chars.contains(&'H'));
        assert!(chars.contains(&'e'));
        assert!(chars.contains(&' '));
        assert!(chars.contains(&'!'));
        assert_eq!(chars.len(), 9); // H,e,l,o, ,W,r,d,!  (unique chars)
    }

    #[test]
    fn test_collect_used_chars_unicode() {
        let text = "ăâîșț";
        let chars = collect_used_chars(text);

        assert_eq!(chars.len(), 5);
        assert!(chars.contains(&'ă'));
        assert!(chars.contains(&'â'));
        assert!(chars.contains(&'î'));
        assert!(chars.contains(&'ș'));
        assert!(chars.contains(&'ț'));
    }
}
