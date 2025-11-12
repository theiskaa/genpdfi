//! Fonts, font families and a font cache.
//!
//! Before you can use a font in a PDF document, you have to load the [`FontData`][] for it, either
//! from a file ([`FontData::load`][]) or from bytes ([`FontData::new`][]).  See the [`rusttype`][]
//! crate for the supported data formats.  Use the [`from_files`][] function to load a font family
//! from a set of files following the default naming conventions.
//!
//! The [`FontCache`][] caches all loaded fonts.  A [`Font`][] is a reference to a cached font in
//! the [`FontCache`][].  A [`FontFamily`][] is a collection of a regular, a bold, an italic and a
//! bold italic font (raw data or cached).
//!
//! Add fonts to a document's font cache by calling [`Document::add_font_family`][].  This method
//! returns a reference to the cached data that you then can use with the [`Style`][] struct to
//! change the font family of an element.
//!
//! There are two methods for using fonts in a PDF font:  You can either embed the font data into
//! the PDF file.  Or you can use one of the three built-in font families ([`Builtin`][]) that PDF
//! viewers are expected to support.  You can choose between the two methods when loading the font
//! ([`from_files`][], [`FontData::load`][], [`FontData::new`][]).
//!
//! If you choose a built-in font family, you still have to provide the font data so that `genpdfi`
//! has access to its glyph metrics.  Note that it is sufficient to use a font that is metrically
//! identical to the built-in font.  For example, you can use the Liberation fonts instad of the
//! proprietary Helvetica, Times and Courier fonts.
//!
//! Built-in fonts can only be used with characters that are supported by the [Windows-1252][]
//! encoding.
//!
//! **Note:**  The [`Font`][] and [`FontFamily<Font>`][`FontFamily`] structs are only valid for the
//! [`FontCache`][] they have been created with.  If you dont use the low-level [`render`][] module
//! directly, only use the [`Document::add_font_family`][] method to add fonts!
//!
//! # Internals
//!
//! There are two types of font data: A [`FontData`][] instance stores information about the glyph
//! metrics that is used to calculate the text size.  It can be loaded at any time using the
//! [`FontData::load`][] and [`FontData::new`][] methods.  Once the PDF document is rendered, a
//! [`printpdf::IndirectFontRef`][] is used to draw text in the PDF document.  Before a font can be
//! used in a PDF document, it has to be embedded using the [`FontCache::load_pdf_fonts`][] method.
//!
//! If you use the high-level interface provided by [`Document`][] to generate a PDF document, these
//! steps are done automatically.  You only have to manually populate the font cache if you use the
//! low-level interface in the [`render`][] module.
//!
//! [`render`]: ../render/
//! [`Document`]: ../struct.Document.html
//! [`Document::add_font_family`]: ../struct.Document.html#method.add_font_family
//! [`Style`]: ../style/struct.Style.html
//! [`from_files`]: fn.from_files.html
//! [`Builtin`]: enum.Builtin.html
//! [`FontCache`]: struct.FontCache.html
//! [`FontCache::load_pdf_fonts`]: struct.FontCache.html#method.load_pdf_fonts
//! [`FontData`]: struct.FontData.html
//! [`FontData::new`]: struct.FontData.html#method.new
//! [`FontData::load`]: struct.FontData.html#method.load
//! [`Font`]: struct.Font.html
//! [`FontFamily`]: struct.FontFamily.html
//! [`rusttype`]: https://docs.rs/rusttype
//! [`rusttype::Font`]: https://docs.rs/rusttype/0.8.3/rusttype/struct.Font.html
//! [`printpdf`]: https://docs.rs/printpdf
//! [`printpdf::IndirectFontRef`]: https://docs.rs/printpdf/0.3.2/printpdf/types/plugins/graphics/two_dimensional/font/struct.IndirectFontRef.html
//! [Windows-1252]: https://en.wikipedia.org/wiki/Windows-1252

use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::path;
use std::sync::Arc;

use crate::error::{Context as _, Error, ErrorKind};
use crate::render;
use crate::style::Style;
use crate::Mm;

/// Stores font data that can be referenced by a [`Font`][] or [`FontFamily`][].
///
/// If you use the high-level interface provided by [`Document`][], you don't have to access this
/// type.  See the [module documentation](index.html) for details on the internals.
///
/// [`Document`]: ../struct.Document.html
/// [`Font`]: struct.Font.html
/// [`FontFamily`]: struct.FontFamily.html
#[derive(Debug)]
pub struct FontCache {
    fonts: Vec<FontData>,
    pdf_fonts: Vec<printpdf::IndirectFontRef>,
    // We have to use an option because we first have to construct the FontCache before we can load
    // a font, but the default font is always loaded in new, so this options is always some
    // (outside of new).
    default_font_family: Option<FontFamily<Font>>,
    // Cache to deduplicate embedded fonts by their data pointer
    embedded_font_cache: HashMap<*const Vec<u8>, printpdf::IndirectFontRef>,
}

impl FontCache {
    /// Creates a new font cache with the given default font family.
    pub fn new(default_font_family: FontFamily<FontData>) -> FontCache {
        let mut font_cache = FontCache {
            fonts: Vec::new(),
            pdf_fonts: Vec::new(),
            default_font_family: None,
            embedded_font_cache: HashMap::new(),
        };
        font_cache.default_font_family = Some(font_cache.add_font_family(default_font_family));
        font_cache
    }

    /// Adds the given font to the cache and returns a reference to it.
    pub fn add_font(&mut self, font_data: FontData) -> Font {
        let is_builtin = match &font_data.raw_data {
            RawFontData::Builtin(_) => true,
            RawFontData::Embedded(_) => false,
        };
        let font = Font::new(self.fonts.len(), is_builtin, &font_data.rt_font);
        self.fonts.push(font_data);
        font
    }

    /// Adds the given font family to the cache and returns a reference to it.
    pub fn add_font_family(&mut self, family: FontFamily<FontData>) -> FontFamily<Font> {
        FontFamily {
            regular: self.add_font(family.regular),
            bold: self.add_font(family.bold),
            italic: self.add_font(family.italic),
            bold_italic: self.add_font(family.bold_italic),
        }
    }

    /// Embeds all loaded fonts into the document generated by the given renderer and caches a
    /// reference to them.
    pub fn load_pdf_fonts(&mut self, renderer: &render::Renderer) -> Result<(), Error> {
        self.pdf_fonts.clear();
        self.embedded_font_cache.clear(); // Clear cache for this document

        for font in &self.fonts {
            let pdf_font = match &font.raw_data {
                RawFontData::Builtin(builtin) => renderer.add_builtin_font(*builtin)?,
                RawFontData::Embedded(data) => {
                    let data_ptr = Arc::as_ptr(data);

                    // Check if we've already embedded this exact font data
                    if let Some(cached_font_ref) = self.embedded_font_cache.get(&data_ptr) {
                        cached_font_ref.clone()
                    } else {
                        let font_ref = renderer.add_embedded_font(data.as_ref())?;
                        self.embedded_font_cache.insert(data_ptr, font_ref.clone());
                        font_ref
                    }
                }
            };
            self.pdf_fonts.push(pdf_font);
        }
        Ok(())
    }

    /// Returns the default font family for this font cache.
    pub fn default_font_family(&self) -> FontFamily<Font> {
        self.default_font_family
            .expect("Invariant violated: no default font family for FontCache")
    }

    /// Returns a reference to the emebdded PDF font for the given font, if available.
    ///
    /// This method may only be called with [`Font`][] instances that have been created by this
    /// font cache.  PDF fonts are only avaiable if [`load_pdf_fonts`][] has been called.
    ///
    /// [`Font`]: struct.Font.html
    /// [`load_pdf_fonts`]: #method.load_pdf_fonts
    pub fn get_pdf_font(&self, font: Font) -> Option<&printpdf::IndirectFontRef> {
        self.pdf_fonts.get(font.idx)
    }

    /// Returns a reference to the Rusttype font for the given font, if available.
    ///
    /// This method may only be called with [`Font`][] instances that have been created by this
    /// font cache.
    ///
    /// [`Font`]: struct.Font.html
    pub fn get_rt_font(&self, font: Font) -> &rusttype::Font<'static> {
        &self.fonts[font.idx].rt_font
    }
}

/// The data for a font that is cached by a [`FontCache`][].
///
/// [`FontCache`]: struct.FontCache.html
#[derive(Clone, Debug)]
pub struct FontData {
    rt_font: rusttype::Font<'static>,
    raw_data: RawFontData,
}

impl FontData {
    /// Loads a font from the given data.
    ///
    /// The provided data must by readable by [`rusttype`][].  If `builtin` is set, a built-in PDF
    /// font is used instead of embedding the font in the PDF file (see the [module
    /// documentation](index.html) for more information).  In this case, the given font must be
    /// metrically identical to the built-in font.
    ///
    /// [`rusttype`]: https://docs.rs/rusttype
    pub fn new(data: Vec<u8>, builtin: Option<printpdf::BuiltinFont>) -> Result<FontData, Error> {
        let raw_data = if let Some(builtin) = builtin {
            RawFontData::Builtin(builtin)
        } else {
            RawFontData::Embedded(Arc::new(data.clone()))
        };
        let rt_font = rusttype::Font::from_bytes(data).context("Failed to read rusttype font")?;
        if rt_font.units_per_em() == 0 {
            Err(Error::new(
                "The font is not scalable",
                ErrorKind::InvalidFont,
            ))
        } else {
            Ok(FontData { rt_font, raw_data })
        }
    }

    /// Creates a new FontData instance that shares the same underlying font data.
    /// This method is optimized to avoid duplicating font data when creating multiple
    /// FontData instances from the same source.
    pub fn new_shared(
        shared_data: Arc<Vec<u8>>,
        builtin: Option<printpdf::BuiltinFont>,
    ) -> Result<FontData, Error> {
        let raw_data = if let Some(builtin) = builtin {
            RawFontData::Builtin(builtin)
        } else {
            RawFontData::Embedded(shared_data.clone())
        };
        let rt_font = rusttype::Font::from_bytes(shared_data.to_vec())
            .context("Failed to read rusttype font")?;
        if rt_font.units_per_em() == 0 {
            Err(Error::new(
                "The font is not scalable",
                ErrorKind::InvalidFont,
            ))
        } else {
            Ok(FontData { rt_font, raw_data })
        }
    }

    /// Loads the font at the given path.
    ///
    /// The path must point to a file that can be read by [`rusttype`][].  If `builtin` is set, a
    /// built-in PDF font is used instead of embedding the font in the PDF file (see the [module
    /// documentation](index.html) for more information).  In this case, the given font must be
    /// metrically identical to the built-in font.
    ///
    /// [`rusttype`]: https://docs.rs/rusttype
    pub fn load(
        path: impl AsRef<path::Path>,
        builtin: Option<printpdf::BuiltinFont>,
    ) -> Result<FontData, Error> {
        let data = fs::read(path.as_ref())
            .with_context(|| format!("Failed to open font file {}", path.as_ref().display()))?;
        FontData::new(data, builtin)
    }

    /// Gets the raw font data bytes (for embedded fonts only).
    ///
    /// # Returns
    /// * `Ok(&[u8])` - The raw font bytes for embedded fonts
    /// * `Err(Error)` - If this is a built-in font (which has no raw data to extract)
    pub fn get_data(&self) -> Result<&[u8], Error> {
        match &self.raw_data {
            RawFontData::Embedded(data) => Ok(data.as_ref()),
            RawFontData::Builtin(_) => Err(Error::new(
                "Cannot get raw data from built-in font".to_string(),
                ErrorKind::InvalidFont,
            )),
        }
    }

    /// Checks if this font has a glyph for the given character.
    ///
    /// # Arguments
    /// * `c` - The character to check
    ///
    /// # Returns
    /// * `true` if the font contains a glyph for this character
    /// * `false` if the character is missing (will render as .notdef/missing glyph)
    ///
    /// # Example
    /// ```rust,no_run
    /// # use genpdfi::fonts::FontData;
    /// # let font_data = FontData::load("font.ttf", None).unwrap();
    /// if font_data.has_glyph('ă') {
    ///     println!("Font supports Romanian characters!");
    /// }
    /// ```
    pub fn has_glyph(&self, c: char) -> bool {
        // In rusttype, glyph ID 0 is the .notdef glyph (missing character indicator)
        // If the glyph for a character has ID 0, the font doesn't support it
        self.rt_font.glyph(c).id().0 != 0
    }

    /// Analyzes glyph coverage for the given text.
    ///
    /// This method checks which characters in the text are supported by this font
    /// and returns detailed coverage statistics.
    ///
    /// # Arguments
    /// * `text` - The text to analyze
    ///
    /// # Returns
    /// A `GlyphCoverage` struct containing coverage statistics and missing characters
    ///
    /// # Example
    /// ```rust,no_run
    /// # use genpdfi::fonts::FontData;
    /// # let font_data = FontData::load("font.ttf", None).unwrap();
    /// let coverage = font_data.check_coverage("Hello ăâîșț!");
    /// println!("Coverage: {:.1}%", coverage.coverage_percent());
    /// if !coverage.is_complete() {
    ///     println!("Missing characters: {:?}", coverage.missing_chars());
    /// }
    /// ```
    pub fn check_coverage(&self, text: &str) -> GlyphCoverage {
        let mut missing_chars = Vec::new();
        let unique_chars: std::collections::HashSet<char> = text.chars().collect();

        for c in unique_chars.iter() {
            if !self.has_glyph(*c) {
                missing_chars.push(*c);
            }
        }

        GlyphCoverage {
            total_unique: unique_chars.len(),
            covered: unique_chars.len() - missing_chars.len(),
            missing_chars,
        }
    }
}

/// Statistics about glyph coverage for a given text.
///
/// This struct provides information about how well a font supports the characters
/// in a text string, useful for determining if font fallbacks are needed.
#[derive(Clone, Debug)]
pub struct GlyphCoverage {
    /// Total number of unique characters in the analyzed text
    total_unique: usize,
    /// Number of characters that have glyphs in the font
    covered: usize,
    /// List of characters that are missing from the font
    missing_chars: Vec<char>,
}

impl GlyphCoverage {
    /// Returns the percentage of characters covered by the font (0-100).
    pub fn coverage_percent(&self) -> f32 {
        if self.total_unique == 0 {
            100.0
        } else {
            (self.covered as f32 / self.total_unique as f32) * 100.0
        }
    }

    /// Returns true if all characters are covered (100% coverage).
    pub fn is_complete(&self) -> bool {
        self.missing_chars.is_empty()
    }

    /// Returns the list of missing characters.
    pub fn missing_chars(&self) -> &[char] {
        &self.missing_chars
    }

    /// Returns the number of covered characters.
    pub fn covered_count(&self) -> usize {
        self.covered
    }

    /// Returns the total number of unique characters analyzed.
    pub fn total_count(&self) -> usize {
        self.total_unique
    }
}

/// A font fallback chain for handling mixed-script documents.
///
/// This struct manages a primary font and a list of fallback fonts. When rendering text,
/// it automatically selects the appropriate font for each character based on glyph coverage.
///
/// # Example
/// ```rust,no_run
/// use genpdfi::fonts::{FontData, FontFallbackChain};
///
/// let primary = FontData::load("NotoSans.ttf", None).unwrap();
/// let cyrillic = FontData::load("NotoSansCyrillic.ttf", None).unwrap();
/// let emoji = FontData::load("NotoEmoji.ttf", None).unwrap();
///
/// let chain = FontFallbackChain::new(primary)
///     .with_fallback(cyrillic)
///     .with_fallback(emoji);
///
/// // Automatically uses the right font for each character
/// let font_for_a = chain.find_font_for_char('a');      // Uses primary (Noto Sans)
/// let font_for_я = chain.find_font_for_char('я');      // Uses cyrillic fallback
/// let font_for_emoji = chain.find_font_for_char('😀'); // Uses emoji fallback
/// ```
#[derive(Clone, Debug)]
pub struct FontFallbackChain {
    /// The primary font to try first
    primary: FontData,
    /// List of fallback fonts to try if primary doesn't have a character
    fallbacks: Vec<FontData>,
}

impl FontFallbackChain {
    /// Creates a new fallback chain with the given primary font.
    pub fn new(primary: FontData) -> Self {
        Self {
            primary,
            fallbacks: Vec::new(),
        }
    }

    /// Adds a fallback font to the chain.
    pub fn with_fallback(mut self, fallback: FontData) -> Self {
        self.fallbacks.push(fallback);
        self
    }

    /// Finds the best font in the chain for the given character.
    ///
    /// Returns a reference to the first font (starting with primary) that has
    /// a glyph for this character. If no font in the chain supports the character,
    /// returns the primary font (which will render the .notdef glyph).
    pub fn find_font_for_char(&self, c: char) -> &FontData {
        // Try primary first
        if self.primary.has_glyph(c) {
            return &self.primary;
        }

        // Try each fallback
        for fallback in &self.fallbacks {
            if fallback.has_glyph(c) {
                return fallback;
            }
        }

        // No font has it, return primary (will show .notdef)
        &self.primary
    }

    /// Returns the primary font.
    pub fn primary(&self) -> &FontData {
        &self.primary
    }

    /// Returns the list of fallback fonts.
    pub fn fallbacks(&self) -> &[FontData] {
        &self.fallbacks
    }

    /// Analyzes coverage across the entire fallback chain for the given text.
    ///
    /// Returns statistics showing which characters are covered by any font in the chain
    /// and which are still missing.
    pub fn check_coverage(&self, text: &str) -> GlyphCoverage {
        let unique_chars: std::collections::HashSet<char> = text.chars().collect();
        let mut missing_chars = Vec::new();

        for c in unique_chars.iter() {
            // Check if ANY font in chain has this character
            let has_glyph = self.primary.has_glyph(*c)
                || self.fallbacks.iter().any(|f| f.has_glyph(*c));

            if !has_glyph {
                missing_chars.push(*c);
            }
        }

        GlyphCoverage {
            total_unique: unique_chars.len(),
            covered: unique_chars.len() - missing_chars.len(),
            missing_chars,
        }
    }

    /// Segments text into chunks where each chunk uses a single font from the chain.
    ///
    /// This analyzes the text character by character, determining which font can render
    /// each character, and groups consecutive characters using the same font into segments.
    ///
    /// # Returns
    /// A vector of tuples (text_segment, font_data_ref) where each segment should be
    /// rendered with the corresponding font.
    ///
    /// # Example
    /// ```no_run
    /// # use genpdfi::fonts::{FontData, FontFallbackChain};
    /// # let primary = FontData::load("font.ttf", None).unwrap();
    /// # let fallback = FontData::load("fallback.ttf", None).unwrap();
    /// let chain = FontFallbackChain::new(primary).with_fallback(fallback);
    /// let segments = chain.segment_text("Hello мир!");
    /// // Returns: [("Hello ", &primary_font), ("мир", &fallback_font), ("!", &primary_font)]
    /// ```
    pub fn segment_text(&self, text: &str) -> Vec<(String, &FontData)> {
        let mut segments = Vec::new();
        let mut current_segment = String::new();
        let mut current_font: Option<&FontData> = None;

        for c in text.chars() {
            let font_for_char = self.find_font_for_char(c);

            // If font changed, flush current segment
            if let Some(current) = current_font {
                if !std::ptr::eq(current, font_for_char) {
                    if !current_segment.is_empty() {
                        segments.push((current_segment.clone(), current));
                        current_segment.clear();
                    }
                    current_font = Some(font_for_char);
                }
            } else {
                current_font = Some(font_for_char);
            }

            current_segment.push(c);
        }

        // Flush remaining segment
        if !current_segment.is_empty() {
            if let Some(font) = current_font {
                segments.push((current_segment, font));
            }
        }

        segments
    }
}

#[derive(Clone, Debug)]
enum RawFontData {
    Builtin(printpdf::BuiltinFont),
    Embedded(Arc<Vec<u8>>),
}

#[derive(Clone, Copy, Debug)]
enum FontStyle {
    Regular,
    Bold,
    Italic,
    BoldItalic,
}

impl FontStyle {
    fn name(&self) -> &'static str {
        match self {
            FontStyle::Regular => "Regular",
            FontStyle::Bold => "Bold",
            FontStyle::Italic => "Italic",
            FontStyle::BoldItalic => "BoldItalic",
        }
    }
}

impl fmt::Display for FontStyle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// A built-in font family.
///
/// A PDF viewer typically supports three font families that don't have to be embedded into the PDF
/// file:  Times, Helvetica and Courier.
///
/// See the [module documentation](index.html) for more information.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Builtin {
    /// The Times font family.
    Times,
    /// The Helvetica font family.
    Helvetica,
    /// The Courier font family.
    Courier,
}

impl Builtin {
    fn style(&self, style: FontStyle) -> printpdf::BuiltinFont {
        match self {
            Builtin::Times => match style {
                FontStyle::Regular => printpdf::BuiltinFont::TimesRoman,
                FontStyle::Bold => printpdf::BuiltinFont::TimesBold,
                FontStyle::Italic => printpdf::BuiltinFont::TimesItalic,
                FontStyle::BoldItalic => printpdf::BuiltinFont::TimesBoldItalic,
            },
            Builtin::Helvetica => match style {
                FontStyle::Regular => printpdf::BuiltinFont::Helvetica,
                FontStyle::Bold => printpdf::BuiltinFont::HelveticaBold,
                FontStyle::Italic => printpdf::BuiltinFont::HelveticaOblique,
                FontStyle::BoldItalic => printpdf::BuiltinFont::HelveticaBoldOblique,
            },
            Builtin::Courier => match style {
                FontStyle::Regular => printpdf::BuiltinFont::Courier,
                FontStyle::Bold => printpdf::BuiltinFont::CourierBold,
                FontStyle::Italic => printpdf::BuiltinFont::CourierOblique,
                FontStyle::BoldItalic => printpdf::BuiltinFont::CourierBoldOblique,
            },
        }
    }
}

/// A collection of fonts with different styles.
///
/// See the [module documentation](index.html) for details on the internals.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FontFamily<T: Clone + fmt::Debug> {
    /// The regular variant of this font family.
    pub regular: T,
    /// The bold variant of this font family.
    pub bold: T,
    /// The italic variant of this font family.
    pub italic: T,
    /// The bold italic variant of this font family.
    pub bold_italic: T,
}

impl<T: Clone + Copy + fmt::Debug + PartialEq> FontFamily<T> {
    /// Returns the font for the given style.
    pub fn get(&self, style: Style) -> T {
        if style.is_bold() && style.is_italic() {
            self.bold_italic
        } else if style.is_bold() {
            self.bold
        } else if style.is_italic() {
            self.italic
        } else {
            self.regular
        }
    }
}

/// A reference to a font cached by a [`FontCache`][].
///
/// See the [module documentation](index.html) for details on the internals.
///
/// [`FontCache`]: struct.FontCache.html
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Font {
    idx: usize,
    is_builtin: bool,
    scale: rusttype::Scale,
    line_height: Mm,
    glyph_height: Mm,
    ascent: Mm,
    descent: Mm,
}

impl Font {
    fn new(idx: usize, is_builtin: bool, rt_font: &rusttype::Font<'static>) -> Font {
        let units_per_em = rt_font.units_per_em();
        assert!(units_per_em != 0);

        let units_per_em = f32::from(units_per_em);
        let v_metrics = rt_font.v_metrics_unscaled();
        let glyph_height = (v_metrics.ascent - v_metrics.descent) / units_per_em;
        let scale = rusttype::Scale::uniform(glyph_height);

        let ascent = v_metrics.ascent / units_per_em;
        let descent = v_metrics.descent / units_per_em;
        let line_height = glyph_height + v_metrics.line_gap / units_per_em;

        Font {
            idx,
            is_builtin,
            scale,
            line_height: printpdf::Pt(f32::from(line_height)).into(),
            glyph_height: printpdf::Pt(f32::from(glyph_height)).into(),
            ascent: printpdf::Pt(f32::from(ascent)).into(),
            descent: printpdf::Pt(f32::from(descent)).into(),
        }
    }
    /// Returns whether this font is a built-in PDF font.
    pub fn is_builtin(&self) -> bool {
        self.is_builtin
    }

    /// Returns the line height for text with this font and the given font size.
    pub fn get_line_height(&self, font_size: u8) -> Mm {
        self.line_height * f32::from(font_size)
    }

    /// Returns the glyph height for text with this font and the given font size.
    pub fn glyph_height(&self, font_size: u8) -> Mm {
        self.glyph_height * f32::from(font_size)
    }

    /// Returns the ascent for text with this font and the given font size.
    pub fn ascent(&self, font_size: u8) -> Mm {
        self.ascent * f32::from(font_size)
    }

    /// Returns the descent for text with this font and the given font size.
    pub fn descent(&self, font_size: u8) -> Mm {
        self.descent * f32::from(font_size)
    }

    /// Returns the width of a character with this font and the given font size.
    ///
    /// The given [`FontCache`][] must be the font cache that loaded this font.
    ///
    /// [`FontCache`]: struct.FontCache.html
    pub fn char_width(&self, font_cache: &FontCache, c: char, font_size: u8) -> Mm {
        let advance_width = self.char_h_metrics(font_cache, c).advance_width;
        Mm::from(printpdf::Pt(f32::from(
            advance_width * f32::from(font_size),
        )))
    }

    /// Returns the width of the empty space between the origin of the glyph bounding
    /// box and the leftmost edge of the character, for a given font and font size.
    ///
    /// The given [`FontCache`][] must be the font cache that loaded this font.
    ///
    /// [`FontCache`]: struct.FontCache.html
    pub fn char_left_side_bearing(&self, font_cache: &FontCache, c: char, font_size: u8) -> Mm {
        let left_side_bearing = self.char_h_metrics(font_cache, c).left_side_bearing;
        Mm::from(printpdf::Pt(f32::from(
            left_side_bearing * f32::from(font_size),
        )))
    }

    fn char_h_metrics(&self, font_cache: &FontCache, c: char) -> rusttype::HMetrics {
        // If this is a built-in font, use standardized metrics instead of system font metrics
        if self.is_builtin {
            self.builtin_char_h_metrics(c)
        } else {
            font_cache
                .get_rt_font(*self)
                .glyph(c)
                .scaled(self.scale)
                .h_metrics()
        }
    }

    /// Returns standardized character metrics for built-in PDF fonts.
    /// These values are based on the Adobe Font Metrics (AFM) for standard PDF fonts.
    fn builtin_char_h_metrics(&self, c: char) -> rusttype::HMetrics {
        let advance_width = match c {
            // Standard character widths for Helvetica (in 1000ths of em)
            ' ' => 0.278,       // space
            '!' => 0.278,       // exclamation
            '"' => 0.355,       // quotation
            '#' => 0.556,       // hash
            '$' => 0.556,       // dollar
            '%' => 0.889,       // percent
            '&' => 0.667,       // ampersand
            '\'' => 0.191,      // apostrophe
            '(' => 0.333,       // left paren
            ')' => 0.333,       // right paren
            '*' => 0.389,       // asterisk
            '+' => 0.584,       // plus
            ',' => 0.278,       // comma
            '-' => 0.333,       // hyphen
            '.' => 0.278,       // period
            '/' => 0.278,       // slash
            '0'..='9' => 0.556, // digits
            ':' => 0.278,       // colon
            ';' => 0.278,       // semicolon
            '<' => 0.584,       // less than
            '=' => 0.584,       // equals
            '>' => 0.584,       // greater than
            '?' => 0.556,       // question
            '@' => 1.015,       // at sign
            'A' => 0.667,       // A
            'B' => 0.667,       // B
            'C' => 0.722,       // C
            'D' => 0.722,       // D
            'E' => 0.667,       // E
            'F' => 0.611,       // F
            'G' => 0.778,       // G
            'H' => 0.722,       // H
            'I' => 0.278,       // I
            'J' => 0.500,       // J
            'K' => 0.667,       // K
            'L' => 0.556,       // L
            'M' => 0.833,       // M
            'N' => 0.722,       // N
            'O' => 0.778,       // O
            'P' => 0.667,       // P
            'Q' => 0.778,       // Q
            'R' => 0.722,       // R
            'S' => 0.667,       // S
            'T' => 0.611,       // T
            'U' => 0.722,       // U
            'V' => 0.667,       // V
            'W' => 0.944,       // W
            'X' => 0.667,       // X
            'Y' => 0.667,       // Y
            'Z' => 0.611,       // Z
            '[' => 0.278,       // left bracket
            '\\' => 0.278,      // backslash
            ']' => 0.278,       // right bracket
            '^' => 0.469,       // caret
            '_' => 0.556,       // underscore
            '`' => 0.333,       // grave
            'a' => 0.556,       // a
            'b' => 0.556,       // b
            'c' => 0.500,       // c
            'd' => 0.556,       // d
            'e' => 0.556,       // e
            'f' => 0.278,       // f
            'g' => 0.556,       // g
            'h' => 0.556,       // h
            'i' => 0.222,       // i
            'j' => 0.222,       // j
            'k' => 0.500,       // k
            'l' => 0.222,       // l
            'm' => 0.833,       // m
            'n' => 0.556,       // n
            'o' => 0.556,       // o
            'p' => 0.556,       // p
            'q' => 0.556,       // q
            'r' => 0.333,       // r
            's' => 0.500,       // s
            't' => 0.278,       // t
            'u' => 0.556,       // u
            'v' => 0.500,       // v
            'w' => 0.722,       // w
            'x' => 0.500,       // x
            'y' => 0.500,       // y
            'z' => 0.500,       // z
            '{' => 0.334,       // left brace
            '|' => 0.260,       // pipe
            '}' => 0.334,       // right brace
            '~' => 0.584,       // tilde
            _ => 0.556,         // default width for unknown characters
        };

        rusttype::HMetrics {
            advance_width: advance_width,
            left_side_bearing: 0.0, // Standard left side bearing for most characters
        }
    }

    /// Returns the width of a string with this font and the given font size.
    ///
    /// The given [`FontCache`][] must be the font cache that loaded this font.
    ///
    /// [`FontCache`]: struct.FontCache.html
    pub fn str_width(&self, font_cache: &FontCache, s: &str, font_size: u8) -> Mm {
        let str_width: Mm = if self.is_builtin {
            // Use standardized metrics for built-in fonts
            s.chars()
                .map(|c| self.builtin_char_h_metrics(c).advance_width)
                .map(|w| Mm::from(printpdf::Pt(f32::from(w * f32::from(font_size)))))
                .sum()
        } else {
            // Use system font metrics for embedded fonts
            font_cache
                .get_rt_font(*self)
                .glyphs_for(s.chars())
                .map(|g| g.scaled(self.scale).h_metrics().advance_width)
                .map(|w| Mm::from(printpdf::Pt(f32::from(w * f32::from(font_size)))))
                .sum()
        };

        let kerning_width: Mm = self
            .kerning(font_cache, s.chars())
            .into_iter()
            .map(|val| val * f32::from(font_size))
            .map(|val| Mm::from(printpdf::Pt(f32::from(val))))
            .sum();
        str_width + kerning_width
    }

    /// Returns the kerning data for the given sequence of characters.
    ///
    /// The *i*-th value of the returned data is the amount of kerning to insert before the *i*-th
    /// character of the sequence.
    ///
    /// The given [`FontCache`][] must be the font cache that loaded this font.
    ///
    /// [`FontCache`]: struct.FontCache.html
    pub fn kerning<I>(&self, font_cache: &FontCache, iter: I) -> Vec<f32>
    where
        I: IntoIterator<Item = char>,
    {
        // Built-in PDF fonts already have their own (device) kerning information that the PDF
        // viewer applies automatically. Passing additional kerning adjustments – especially ones
        // derived from a *similar* but not identical system TTF – results in characters being
        // pushed apart instead of pulled together. Therefore we disable kerning completely for
        // built-in fonts and only return actual kerning values for embedded/system fonts.
        if self.is_builtin {
            // Return a zero adjustment for every glyph so the caller's `positions.zip(codepoints)`
            // iterator remains the correct length.
            iter.into_iter().map(|_| 0.0).collect()
        } else {
            let font = font_cache.get_rt_font(*self);
            font.glyphs_for(iter.into_iter())
                .scan(None, |last, g| {
                    let pos = if let Some(last) = last {
                        Some(font.pair_kerning(self.scale, *last, g.id()))
                    } else {
                        Some(0.0)
                    };
                    *last = Some(g.id());
                    pos
                })
                .collect()
        }
    }

    /// Returns the glyphs IDs for the given sequence of characters.
    ///
    /// The given [`FontCache`][] must be the font cache that loaded this font.
    ///
    /// [`FontCache`]: struct.FontCache.html
    pub fn glyph_ids<I>(&self, font_cache: &FontCache, iter: I) -> Vec<u16>
    where
        I: IntoIterator<Item = char>,
    {
        let font = font_cache.get_rt_font(*self);
        font.glyphs_for(iter.into_iter())
            .map(|g| g.id().0 as u16)
            .collect()
    }

    /// Calculate the metrics of a given font size for this font.
    pub fn metrics(&self, font_size: u8) -> Metrics {
        Metrics::new(
            self.line_height * f32::from(font_size),
            self.glyph_height * f32::from(font_size),
            self.ascent * f32::from(font_size),
            self.descent * f32::from(font_size),
        )
    }
}

fn from_file(
    dir: impl AsRef<path::Path>,
    name: &str,
    style: FontStyle,
    builtin: Option<Builtin>,
) -> Result<FontData, Error> {
    let builtin = builtin.map(|b| b.style(style));
    FontData::load(
        &dir.as_ref().join(format!("{}-{}.ttf", name, style)),
        builtin,
    )
}

/// Loads the font family at the given path with the given name.
///
/// This method assumes that at the given path, these files exist and are valid font files:
/// - `{name}-Regular.ttf`
/// - `{name}-Bold.ttf`
/// - `{name}-Italic.ttf`
/// - `{name}-BoldItalic.ttf`
///
/// If `builtin` is set, built-in PDF fonts are used instead of embedding the fonts in the PDF file
/// (see the [module documentation](index.html) for more information).  In this case, the given
/// fonts must be metrically identical to the built-in fonts.
pub fn from_files(
    dir: impl AsRef<path::Path>,
    name: &str,
    builtin: Option<Builtin>,
) -> Result<FontFamily<FontData>, Error> {
    let dir = dir.as_ref();
    Ok(FontFamily {
        regular: from_file(dir, name, FontStyle::Regular, builtin)?,
        bold: from_file(dir, name, FontStyle::Bold, builtin)?,
        italic: from_file(dir, name, FontStyle::Italic, builtin)?,
        bold_italic: from_file(dir, name, FontStyle::BoldItalic, builtin)?,
    })
}

/// The metrics of a font at a given scale.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Metrics {
    /// The line height of the font at a given scale.
    pub line_height: Mm,
    /// The glyph height of the font at a given scale.
    pub glyph_height: Mm,
    /// The ascent of the font at a given scale.
    pub ascent: Mm,
    /// The descent of the font at a given scale.
    pub descent: Mm,
}

impl Metrics {
    /// Create a new metrics instance with the given heights.
    pub fn new(line_height: Mm, glyph_height: Mm, ascent: Mm, descent: Mm) -> Metrics {
        Metrics {
            line_height,
            glyph_height,
            ascent,
            descent,
        }
    }

    /// Returns the maximum metrics from two metrics instances.
    pub fn max(&self, other: &Self) -> Self {
        Self {
            line_height: self.line_height.max(other.line_height),
            glyph_height: self.glyph_height.max(other.glyph_height),
            ascent: self.ascent.max(other.ascent),
            descent: self.descent.max(other.descent),
        }
    }
}
