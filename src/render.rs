//! Low-level PDF rendering utilities.
//!
//! This module provides low-level abstractions over [`printpdf`][]:  A [`Renderer`][] creates a
//! document with one or more pages with different sizes.  A [`Page`][] has one or more layers, all
//! of the same size.  A [`Layer`][] can be used to access its [`Area`][].
//!
//! An [`Area`][] is a view on a full layer or on a part of a layer.  It can be used to print
//! lines and text.  For more advanced text formatting, you can create a [`TextSection`][] from an
//! [`Area`][].
//!
//! [`printpdf`]: https://docs.rs/printpdf/latest/printpdf
//! [`Renderer`]: struct.Renderer.html
//! [`Page`]: struct.Page.html
//! [`Layer`]: struct.Layer.html
//! [`Area`]: struct.Area.html
//! [`TextSection`]: struct.TextSection.html

use std::cell;
use std::io;
use std::ops;
use std::rc;

use crate::error::{Context as _, Error, ErrorKind};
use crate::fonts;
use crate::style::{Color, LineStyle, Style};
use crate::{Margins, Mm, Position, Size};

#[cfg(feature = "images")]
use crate::{Rotation, Scale};

/// A position relative to the top left corner of a layer.
struct LayerPosition(Position);

impl LayerPosition {
    pub fn from_area(area: &Area<'_>, position: Position) -> Self {
        Self(position + area.origin)
    }
}

/// A position relative to the bottom left corner of a layer (“user space” in PDF terms).
struct UserSpacePosition(Position);

impl UserSpacePosition {
    pub fn from_layer(layer: &Layer<'_>, position: LayerPosition) -> Self {
        Self(Position::new(
            position.0.x,
            layer.page.size.height - position.0.y,
        ))
    }
}

impl From<UserSpacePosition> for printpdf::Point {
    fn from(pos: UserSpacePosition) -> printpdf::Point {
        printpdf::Point::new(pos.0.x.into(), pos.0.y.into())
    }
}

impl ops::Deref for UserSpacePosition {
    type Target = Position;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Renders a PDF document with one or more pages.
///
/// This is a wrapper around a [`printpdf::PdfDocumentReference`][].
///
/// [`printpdf::PdfDocumentReference`]: https://docs.rs/printpdf/0.3.2/printpdf/types/pdf_document/struct.PdfDocumentReference.html
pub struct Renderer {
    doc: printpdf::PdfDocumentReference,
    // invariant: pages.len() >= 1
    pages: Vec<Page>,
}

impl Renderer {
    /// Creates a new PDF document renderer with one page of the given size and the given title.
    pub fn new(size: impl Into<Size>, title: impl AsRef<str>) -> Result<Renderer, Error> {
        let size = size.into();
        let (doc, page_idx, layer_idx) = printpdf::PdfDocument::new(
            title.as_ref(),
            size.width.into(),
            size.height.into(),
            "Layer 1",
        );
        let page_ref = doc.get_page(page_idx);
        let layer_ref = page_ref.get_layer(layer_idx);
        let page = Page::new(page_ref, layer_ref, size);

        Ok(Renderer {
            doc,
            pages: vec![page],
        })
    }

    /// Sets the PDF conformance for the generated PDF document.
    pub fn with_conformance(mut self, conformance: printpdf::PdfConformance) -> Self {
        self.doc = self.doc.with_conformance(conformance);
        self
    }

    /// Sets the creation date for the generated PDF document.
    pub fn with_creation_date(mut self, date: printpdf::OffsetDateTime) -> Self {
        self.doc = self.doc.with_creation_date(date);
        self
    }

    /// Sets the modification date for the generated PDF document.
    pub fn with_modification_date(mut self, date: printpdf::OffsetDateTime) -> Self {
        self.doc = self.doc.with_mod_date(date);
        self
    }

    /// Adds a new page with the given size to the document.
    pub fn add_page(&mut self, size: impl Into<Size>) {
        let size = size.into();
        let (page_idx, layer_idx) =
            self.doc
                .add_page(size.width.into(), size.height.into(), "Layer 1");
        let page_ref = self.doc.get_page(page_idx);
        let layer_ref = page_ref.get_layer(layer_idx);
        self.pages.push(Page::new(page_ref, layer_ref, size))
    }

    /// Returns the number of pages in this document.
    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    /// Returns a page of this document.
    pub fn get_page(&self, idx: usize) -> Option<&Page> {
        self.pages.get(idx)
    }

    /// Returns a mutable reference to a page of this document.
    pub fn get_page_mut(&mut self, idx: usize) -> Option<&mut Page> {
        self.pages.get_mut(idx)
    }

    /// Returns a mutable reference to the first page of this document.
    pub fn first_page(&self) -> &Page {
        &self.pages[0]
    }

    /// Returns the first page of this document.
    pub fn first_page_mut(&mut self) -> &mut Page {
        &mut self.pages[0]
    }

    /// Returns the last page of this document.
    pub fn last_page(&self) -> &Page {
        &self.pages[self.pages.len() - 1]
    }

    /// Returns a mutable reference to the last page of this document.
    pub fn last_page_mut(&mut self) -> &mut Page {
        let idx = self.pages.len() - 1;
        &mut self.pages[idx]
    }

    /// Loads the font from the given data, adds it to the generated document and returns a
    /// reference to it.
    pub fn add_builtin_font(
        &self,
        builtin: printpdf::BuiltinFont,
    ) -> Result<printpdf::IndirectFontRef, Error> {
        self.doc
            .add_builtin_font(builtin)
            .context("Failed to load PDF font")
    }

    /// Loads the font from the given data, adds it to the generated document and returns a
    /// reference to it.
    pub fn add_embedded_font(&self, data: &[u8]) -> Result<printpdf::IndirectFontRef, Error> {
        self.doc
            .add_external_font(data)
            .context("Failed to load PDF font")
    }

    /// Writes this PDF document to a writer.
    pub fn write(self, w: impl io::Write) -> Result<(), Error> {
        self.doc
            .save(&mut io::BufWriter::new(w))
            .context("Failed to save document")
    }
}

/// A page of a PDF document.
///
/// This is a wrapper around a [`printpdf::PdfPageReference`][].
///
/// [`printpdf::PdfPageReference`]: https://docs.rs/printpdf/0.3.2/printpdf/types/pdf_page/struct.PdfPageReference.html
pub struct Page {
    page: printpdf::PdfPageReference,
    size: Size,
    layers: Layers,
}

impl Page {
    fn new(
        page: printpdf::PdfPageReference,
        layer: printpdf::PdfLayerReference,
        size: Size,
    ) -> Page {
        Page {
            page,
            size,
            layers: Layers::new(layer),
        }
    }

    /// Adds a new layer with the given name to the page.
    pub fn add_layer(&mut self, name: impl Into<String>) {
        let layer = self.page.add_layer(name);
        self.layers.push(layer);
    }

    /// Returns the number of layers on this page.
    pub fn layer_count(&self) -> usize {
        self.layers.len()
    }

    /// Returns a layer of this page.
    pub fn get_layer(&self, idx: usize) -> Option<Layer<'_>> {
        self.layers.get(idx).map(|l| Layer::new(self, l))
    }

    /// Returns the first layer of this page.
    pub fn first_layer(&self) -> Layer<'_> {
        Layer::new(self, self.layers.first())
    }

    /// Returns the last layer of this page.
    pub fn last_layer(&self) -> Layer<'_> {
        Layer::new(self, self.layers.last())
    }

    fn next_layer(&self, layer: &printpdf::PdfLayerReference) -> Layer<'_> {
        let layer = self.layers.next(layer).unwrap_or_else(|| {
            let layer = self
                .page
                .add_layer(format!("Layer {}", self.layers.len() + 1));
            self.layers.push(layer)
        });
        Layer::new(self, layer)
    }
}

#[derive(Debug)]
struct Layers(cell::RefCell<Vec<rc::Rc<LayerData>>>);

impl Layers {
    pub fn new(layer: printpdf::PdfLayerReference) -> Self {
        Self(vec![LayerData::from(layer).into()].into())
    }

    pub fn len(&self) -> usize {
        self.0.borrow().len()
    }

    pub fn first(&self) -> rc::Rc<LayerData> {
        self.0.borrow().first().unwrap().clone()
    }

    pub fn last(&self) -> rc::Rc<LayerData> {
        self.0.borrow().last().unwrap().clone()
    }

    pub fn get(&self, idx: usize) -> Option<rc::Rc<LayerData>> {
        self.0.borrow().get(idx).cloned()
    }

    pub fn push(&self, layer: printpdf::PdfLayerReference) -> rc::Rc<LayerData> {
        let layer_data = rc::Rc::from(LayerData::from(layer));
        self.0.borrow_mut().push(layer_data.clone());
        layer_data
    }

    pub fn next(&self, layer: &printpdf::PdfLayerReference) -> Option<rc::Rc<LayerData>> {
        self.0
            .borrow()
            .iter()
            .skip_while(|l| l.layer.layer != layer.layer)
            .nth(1)
            .cloned()
    }
}

/// A layer of a page of a PDF document.
///
/// This is a wrapper around a [`printpdf::PdfLayerReference`][].
///
/// [`printpdf::PdfLayerReference`]: https://docs.rs/printpdf/0.3.2/printpdf/types/pdf_layer/struct.PdfLayerReference.html
#[derive(Clone)]
pub struct Layer<'p> {
    page: &'p Page,
    data: rc::Rc<LayerData>,
}

impl<'p> Layer<'p> {
    fn new(page: &'p Page, data: rc::Rc<LayerData>) -> Layer<'p> {
        Layer { page, data }
    }

    /// Returns the underlying `PdfLayerReference` for this layer.
    pub fn layer(&self) -> &printpdf::PdfLayerReference {
        &self.data.layer
    }

    /// Returns the next layer of this page.
    ///
    /// If this layer is not the last layer, the existing next layer is used.  If it is the last
    /// layer, a new layer is created and added to the page.
    pub fn next(&self) -> Layer<'p> {
        self.page.next_layer(&self.data.layer)
    }

    /// Returns a drawable area for this layer.
    pub fn area(&self) -> Area<'p> {
        Area::new(self.clone(), Position::default(), self.page.size)
    }

    #[cfg(feature = "images")]
    fn add_image(
        &self,
        image: &image::DynamicImage,
        position: LayerPosition,
        scale: Scale,
        rotation: Rotation,
        dpi: Option<f32>,
    ) {
        let dynamic_image = printpdf::Image::from_dynamic_image(image);
        let position = self.transform_position(position);
        let rotation = Some(printpdf::ImageRotation {
            angle_ccw_degrees: rotation.degrees,
            rotation_center_x: printpdf::Px(dynamic_image.image.width.0 / 2),
            rotation_center_y: printpdf::Px(dynamic_image.image.height.0 / 2),
        });
        dynamic_image.add_to_layer(
            self.data.layer.clone(),
            printpdf::ImageTransform {
                translate_x: Some(position.x.into()),
                translate_y: Some(position.y.into()),
                rotate: rotation,
                scale_x: Some(scale.x),
                scale_y: Some(scale.y),
                dpi
            }
        );
    }

    fn add_line_shape<I>(&self, points: I)
    where
        I: IntoIterator<Item = LayerPosition>,
    {
        let line_points: Vec<_> = points
            .into_iter()
            .map(|pos| (self.transform_position(pos).into(), false))
            .collect();
        let line = printpdf::Line {
            points: line_points,
            is_closed: false,
        };
        self.data.layer.add_line(line);
    }

    fn set_fill_color(&self, color: Option<Color>) {
        if self.data.update_fill_color(color) {
            self.data
                .layer
                .set_fill_color(color.unwrap_or(Color::Rgb(0, 0, 0)).into());
        }
    }

    fn set_outline_thickness(&self, thickness: Mm) {
        if self.data.update_outline_thickness(thickness) {
            self.data
                .layer
                .set_outline_thickness(printpdf::Pt::from(thickness).0);
        }
    }

    fn set_outline_color(&self, color: Color) {
        if self.data.update_outline_color(color) {
            self.data.layer.set_outline_color(color.into());
        }
    }

    fn set_text_cursor(&self, cursor: LayerPosition) {
        let cursor = self.transform_position(cursor);
        self.data
            .layer
            .set_text_cursor(cursor.x.into(), cursor.y.into());
    }

    fn begin_text_section(&self) {
        self.data.layer.begin_text_section();
    }

    fn end_text_section(&self) {
        self.data.layer.end_text_section();
    }

    fn add_line_break(&self) {
        self.data.layer.add_line_break();
    }

    fn set_line_height(&self, line_height: Mm) {
        self.data.layer.set_line_height(line_height.0);
    }

    fn set_font(&self, font: &printpdf::IndirectFontRef, font_size: u8) {
        self.data.layer.set_font(font, font_size.into());
    }

    fn write_positioned_codepoints<P, C>(&self, positions: P, codepoints: C)
    where
        P: IntoIterator<Item = i64>,
        C: IntoIterator<Item = u16>,
    {
        self.data
            .layer
            .write_positioned_codepoints(positions.into_iter().zip(codepoints.into_iter()));
    }

    /// Transforms the given position that is relative to the upper left corner of the layer to a
    /// position that is relative to the lower left corner of the layer (as used by `printpdf`).
    fn transform_position(&self, position: LayerPosition) -> UserSpacePosition {
        UserSpacePosition::from_layer(self, position)
    }

    /// Adds a link annotation to the layer.
    pub fn add_annotation(&mut self, annotation: printpdf::LinkAnnotation) {
        self.data.layer.add_link_annotation(annotation);
    }
}

#[derive(Debug)]
struct LayerData {
    layer: printpdf::PdfLayerReference,
    fill_color: cell::Cell<Color>,
    outline_color: cell::Cell<Color>,
    outline_thickness: cell::Cell<Mm>,
}

impl LayerData {
    pub fn update_fill_color(&self, color: Option<Color>) -> bool {
        let color = color.unwrap_or(Color::Rgb(0, 0, 0));
        self.fill_color.replace(color) != color
    }

    pub fn update_outline_color(&self, color: Color) -> bool {
        self.outline_color.replace(color) != color
    }

    pub fn update_outline_thickness(&self, thickness: Mm) -> bool {
        self.outline_thickness.replace(thickness) != thickness
    }
}

impl From<printpdf::PdfLayerReference> for LayerData {
    fn from(layer: printpdf::PdfLayerReference) -> Self {
        Self {
            layer,
            fill_color: Color::Rgb(0, 0, 0).into(),
            outline_color: Color::Rgb(0, 0, 0).into(),
            outline_thickness: Mm::from(printpdf::Pt(1.0)).into(),
        }
    }
}

/// A view on an area of a PDF layer that can be drawn on.
///
/// This struct provides access to the drawing methods of a [`printpdf::PdfLayerReference`][].  It
/// is defined by the layer that is drawn on and the origin and the size of the area.
///
/// [`printpdf::PdfLayerReference`]: https://docs.rs/printpdf/0.3.2/printpdf/types/pdf_layer/struct.PdfLayerReference.html
#[derive(Clone)]
pub struct Area<'p> {
    layer: Layer<'p>,
    origin: Position,
    size: Size,
}

impl<'p> Area<'p> {
    fn new(layer: Layer<'p>, origin: Position, size: Size) -> Area<'p> {
        Area {
            layer,
            origin,
            size,
        }
    }

    /// Returns a copy of this area on the next layer of the page.
    ///
    /// If this area is not on the last layer, the existing next layer is used.  If it is on the
    /// last layer, a new layer is created and added to the page.
    pub fn next_layer(&self) -> Self {
        let layer = self.layer.next();
        Self {
            layer,
            origin: self.origin,
            size: self.size,
        }
    }

    /// Reduces the size of the drawable area by the given margins.
    pub fn add_margins(&mut self, margins: impl Into<Margins>) {
        let margins = margins.into();
        self.origin.x += margins.left;
        self.origin.y += margins.top;
        self.size.width -= margins.left + margins.right;
        self.size.height -= margins.top + margins.bottom;
    }

    /// Returns the size of this area.
    pub fn size(&self) -> Size {
        self.size
    }

    /// Adds the given offset to the area, reducing the drawable area.
    pub fn add_offset(&mut self, offset: impl Into<Position>) {
        let offset = offset.into();
        self.origin.x += offset.x;
        self.origin.y += offset.y;
        self.size.width -= offset.x;
        self.size.height -= offset.y;
    }

    /// Sets the size of this area.
    pub fn set_size(&mut self, size: impl Into<Size>) {
        self.size = size.into();
    }

    /// Sets the width of this area.
    pub fn set_width(&mut self, width: Mm) {
        self.size.width = width;
    }

    /// Sets the height of this area.
    pub fn set_height(&mut self, height: Mm) {
        self.size.height = height;
    }

    /// Splits this area horizontally using the given weights.
    ///
    /// The returned vector has the same number of elements as the provided slice.  The width of
    /// the *i*-th area is *width \* weights[i] / total_weight*, where *width* is the width of this
    /// area, and *total_weight* is the sum of all given weights.
    pub fn split_horizontally(&self, weights: &[usize]) -> Vec<Area<'p>> {
        let total_weight: usize = weights.iter().sum();
        let factor = self.size.width / total_weight as f32;
        let widths = weights.iter().map(|weight| factor * *weight as f32);
        let mut offset = Mm(0.0);
        let mut areas = Vec::new();
        for width in widths {
            let mut area = self.clone();
            area.origin.x += offset;
            area.size.width = width;
            areas.push(area);
            offset += width;
        }
        areas
    }

    /// Inserts an image into the document.
    ///
    /// *Only available if the `images` feature is enabled.*
    ///
    /// The position is assumed to be relative to the upper left hand corner of the area.
    /// Your position will need to compensate for rotation/scale/dpi. Using [`Image`][]’s
    /// render functionality will do this for you and is the recommended way to
    /// insert an image into an Area.
    ///
    /// [`Image`]: ../elements/struct.Image.html
    #[cfg(feature = "images")]
    pub fn add_image(
        &self,
        image: &image::DynamicImage,
        position: Position,
        scale: Scale,
        rotation: Rotation,
        dpi: Option<f32>,
    ) {
        self.layer
            .add_image(image, self.position(position), scale, rotation, dpi);
    }

    /// Draws a line with the given points and the given line style.
    ///
    /// The points are relative to the upper left corner of the area.
    pub fn draw_line<I>(&self, points: I, line_style: LineStyle)
    where
        I: IntoIterator<Item = Position>,
    {
        self.layer.set_outline_thickness(line_style.thickness());
        self.layer.set_outline_color(line_style.color());
        self.layer
            .add_line_shape(points.into_iter().map(|pos| self.position(pos)));
    }

    /// Tries to draw the given string at the given position and returns `true` if the area was
    /// large enough to draw the string.
    ///
    /// The font cache must contain the PDF font for the font set in the style.  The position is
    /// relative to the upper left corner of the area.
    pub fn print_str<S: AsRef<str>>(
        &self,
        font_cache: &fonts::FontCache,
        position: Position,
        style: Style,
        s: S,
    ) -> Result<bool, Error> {
        if let Some(mut section) =
            self.text_section(font_cache, position, style.metrics(font_cache))
        {
            section.print_str(s, style)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Creates a new text section at the given position if the text section fits in this area.
    ///
    /// The given style is only used to calculate the line height of the section.  The position is
    /// relative to the upper left corner of the area.  The font cache must contain the PDF font
    /// for all fonts printed with the text section.
    pub fn text_section<'f>(
        &self,
        font_cache: &'f fonts::FontCache,
        position: Position,
        metrics: fonts::Metrics,
    ) -> Option<TextSection<'f, 'p>> {
        let mut area = self.clone();
        area.add_offset(position);
        TextSection::new(font_cache, area, metrics)
    }

    /// Returns a position relative to the top left corner of this area.
    fn position(&self, position: Position) -> LayerPosition {
        LayerPosition::from_area(self, position)
    }

    /// Adds a clickable link to the document.
    ///
    /// The font cache must contain the PDF font for the font set in the style.  The position is
    /// relative to the upper left corner of the area.
    pub fn add_link<S: AsRef<str>>(
        &self,
        font_cache: &fonts::FontCache,
        position: Position,
        style: Style,
        text: S,
        uri: S,
    ) -> Result<bool, Error> {
        if let Some(mut section) =
            self.text_section(font_cache, position, style.metrics(font_cache))
        {
            section.add_link(text, uri, style)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

/// A text section that is drawn on an area of a PDF layer.
pub struct TextSection<'f, 'p> {
    font_cache: &'f fonts::FontCache,
    area: Area<'p>,
    is_first: bool,
    metrics: fonts::Metrics,
    font: Option<(printpdf::IndirectFontRef, u8)>,
    current_x_offset: Mm,
    cumulative_kerning: Mm,
}

impl<'f, 'p> TextSection<'f, 'p> {
    fn new(
        font_cache: &'f fonts::FontCache,
        area: Area<'p>,
        metrics: fonts::Metrics,
    ) -> Option<TextSection<'f, 'p>> {
        if metrics.glyph_height > area.size.height {
            return None;
        }

        area.layer.begin_text_section();
        area.layer.set_line_height(metrics.line_height);

        Some(TextSection {
            font_cache,
            area,
            is_first: true,
            metrics,
            font: None,
            current_x_offset: Mm(0.0),
            cumulative_kerning: Mm(0.0),
        })
    }

    fn set_text_cursor(&self, x_offset: Mm) {
        let cursor = self
            .area
            .position(Position::new(x_offset, self.metrics.ascent));
        self.area.layer.set_text_cursor(cursor);
    }

    fn set_font(&mut self, font: &printpdf::IndirectFontRef, font_size: u8) {
        let font_is_set = self
            .font
            .as_ref()
            .map(|(font, font_size)| (font, *font_size))
            .map(|data| data == (font, font_size))
            .unwrap_or_default();
        if !font_is_set {
            self.font = Some((font.clone(), font_size));
            self.area.layer.set_font(font, font_size);
        }
    }

    /// Tries to add a new line and returns `true` if the area was large enough to fit the new
    /// line.
    #[must_use]
    pub fn add_newline(&mut self) -> bool {
        if self.metrics.line_height > self.area.size.height {
            false
        } else {
            self.area.layer.add_line_break();
            self.area.add_offset((0, self.metrics.line_height));
            true
        }
    }

    /// Prints the given string with the given style.
    ///
    /// The font cache for this text section must contain the PDF font for the given style.
    pub fn print_str(&mut self, s: impl AsRef<str>, style: Style) -> Result<(), Error> {
        let font = style.font(self.font_cache);
        let s = s.as_ref();

        if self.is_first {
            if let Some(first_c) = s.chars().next() {
                let x_offset = style.char_left_side_bearing(self.font_cache, first_c) * -1.0;
                self.set_text_cursor(x_offset);
            }
            self.is_first = false;
        }

        let kerning_positions = font.kerning(self.font_cache, s.chars());
        let positions = kerning_positions
            .clone()
            .into_iter()
            .map(|pos| (pos * -1000.0) as i64);
        let codepoints = if font.is_builtin() {
            encode_win1252(s)?
        } else {
            font.glyph_ids(&self.font_cache, s.chars())
        };

        let font = self
            .font_cache
            .get_pdf_font(font)
            .expect("Could not find PDF font in font cache");
        self.area.layer.set_fill_color(style.color());
        self.set_font(font, style.font_size());

        self.area
            .layer
            .write_positioned_codepoints(positions, codepoints);

        let kerning_sum = Mm(kerning_positions.iter().sum::<f32>());
        self.cumulative_kerning += kerning_sum;
        self.current_x_offset += style.text_width(self.font_cache, s);

        Ok(())
    }

    /// Adds a clickable link with the given text, URI, and style.
    ///
    /// The font cache for this text section must contain the PDF font for the given style.
    pub fn add_link(
        &mut self,
        text: impl AsRef<str>,
        uri: impl AsRef<str>,
        style: Style,
    ) -> Result<(), Error> {
        let font = style.font(self.font_cache);
        let text = text.as_ref();
        let uri = uri.as_ref();

        let kerning_positions: Vec<f32> = font.kerning(self.font_cache, text.chars());

        // Get current cursor position, including all accumulated offsets
        let current_pos = self.area.position(Position::new(
            self.current_x_offset + self.cumulative_kerning,
            0.0,
        ));

        let pdf_pos = self.area.layer.transform_position(current_pos);
        let text_width = style.text_width(self.font_cache, text);
        let rect = printpdf::Rect::new(
            printpdf::Mm(pdf_pos.x.0),                                     // left
            printpdf::Mm(pdf_pos.y.0 - font.ascent(style.font_size()).0),  // bottom
            printpdf::Mm(pdf_pos.x.0 + text_width.0),                      // right
            printpdf::Mm(pdf_pos.y.0 + font.descent(style.font_size()).0), // top
        );

        let annotation = printpdf::LinkAnnotation::new(
            rect,
            Some(printpdf::BorderArray::Solid([0.0, 0.0, 0.0])), // No border
            Some(printpdf::ColorArray::Transparent),             // Transparent color
            printpdf::Actions::uri(uri.to_string()),
            None,
        );
        self.area.layer.add_annotation(annotation);

        // Handle first character positioning
        if self.is_first {
            if let Some(first_c) = text.chars().next() {
                let x_offset = style.char_left_side_bearing(self.font_cache, first_c) * -1.0;
                self.set_text_cursor(x_offset);
            }
            self.is_first = false;
        }

        let positions = kerning_positions
            .clone()
            .into_iter()
            .map(|pos| (pos * -1000.0) as i64);

        let codepoints = if font.is_builtin() {
            encode_win1252(text)?
        } else {
            font.glyph_ids(&self.font_cache, text.chars())
        };

        let pdf_font = self
            .font_cache
            .get_pdf_font(font)
            .expect("Could not find PDF font in font cache");

        self.area.layer.set_fill_color(style.color());
        self.set_font(pdf_font, style.font_size());

        self.area
            .layer
            .write_positioned_codepoints(positions, codepoints);

        // Update position tracking
        let kerning_sum = Mm(kerning_positions.iter().sum::<f32>());
        self.cumulative_kerning += kerning_sum;
        self.current_x_offset += text_width;

        Ok(())
    }
}

impl<'f, 'p> Drop for TextSection<'f, 'p> {
    fn drop(&mut self) {
        self.area.layer.end_text_section();
    }
}

/// Encodes the given string using the Windows-1252 encoding for use with built-in PDF fonts,
/// returning an error if it contains unsupported characters.
fn encode_win1252(s: &str) -> Result<Vec<u16>, Error> {
    let bytes: Vec<_> = lopdf::Document::encode_text(Some("WinAnsiEncoding"), s)
        .into_iter()
        .map(u16::from)
        .collect();

    // Windows-1252 is a single-byte encoding, so one byte is one character.
    if bytes.len() != s.chars().count() {
        Err(Error::new(
            format!(
                "Tried to print a string with characters that are not supported by the \
                Windows-1252 encoding with a built-in font: {}",
                s
            ),
            ErrorKind::UnsupportedEncoding,
        ))
    } else {
        Ok(bytes)
    }
}
