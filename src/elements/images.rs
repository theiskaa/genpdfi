//! Image support for genpdfi-rs.

use std::path;

use image::GenericImageView;

use crate::error::{Context as _, Error, ErrorKind};
use crate::{render, style};
use crate::{Alignment, Context, Element, Mm, Position, RenderResult, Rotation, Scale, Size};

/// An image to embed in the PDF.
///
/// *Only available if the `images` feature is enabled.*
///
/// This struct is a wrapper around the configurations [`printpdf::Image`][] exposes.
///
/// # Supported Formats
///
/// All formats supported by the [`image`][] should be supported by this crate.  The BMP, JPEG and
/// PNG formats are well tested and known to work.  Yet it is currently not possible to render
/// images with transparency, see [`printpdf` issue #98][].
///
/// Note that only the GIF, JPEG, PNG, PNM, TIFF and BMP formats are enabled by default.  If you
/// want to use other formats, you have to add the `image` crate as a dependency and activate the
/// required feature.
///
/// # Example
///
/// ```
/// use std::convert::TryFrom;
/// use genpdfi::elements;
/// let image = elements::Image::from_path("examples/images/test_image.jpg")
///       .expect("Failed to load test image")
///       .with_alignment(genpdfi::Alignment::Center) // Center the image on the page.
///       .with_scale(genpdfi::Scale::new(0.5, 2.0)); // Squeeze and then stretch upwards.
/// ```
///
/// [`image`]: https://lib.rs/crates/image
/// [`printpdf::Image`]: https://docs.rs/printpdf/latest/printpdf/types/plugins/graphics/two_dimensional/image/struct.Image.html
/// [`printpdf` issue #98]: https://github.com/fschutt/printpdf/issues/98
#[derive(Clone)]
pub struct Image {
    data: image::DynamicImage,

    /// Used for positioning if no absolute position is given.
    alignment: Alignment,

    /// The absolute position within the given area.
    ///
    /// If no position is set, we use the Alignment.
    position: Option<Position>,

    /// Scaling of the image, default is 1:1.
    scale: Scale,

    /// The number of degrees of clockwise rotation.
    rotation: Rotation,

    /// DPI override if you know better. Defaults to `printpdf`’s default of 300 dpi.
    dpi: Option<f32>,
}

impl Image {
    /// Creates a new image from an already loaded image.
    pub fn from_dynamic_image(data: image::DynamicImage) -> Result<Self, Error> {
        if data.color().has_alpha() {
            Err(Error::new(
                "Images with an alpha channel are not supported",
                ErrorKind::InvalidData,
            ))
        } else {
            Ok(Image {
                data,
                alignment: Alignment::default(),
                position: None,
                scale: Scale::default(),
                rotation: Rotation::default(),
                dpi: None,
            })
        }
    }

    fn from_image_reader<R>(reader: image::io::Reader<R>) -> Result<Self, Error>
    where
        R: std::io::BufRead,
        R: std::io::Read,
        R: std::io::Seek,
    {
        let image = reader
            .with_guessed_format()
            .context("Could not determine image format")?
            .decode()
            .context("Could not decode image")?;
        Self::from_dynamic_image(image)
    }

    /// Creates a new image from the given reader.
    pub fn from_reader<R>(reader: R) -> Result<Self, Error>
    where
        R: std::io::BufRead,
        R: std::io::Read,
        R: std::io::Seek,
    {
        Self::from_image_reader(image::io::Reader::new(reader))
    }

    /// Creates a new image by reading from the given path.
    pub fn from_path(path: impl AsRef<path::Path>) -> Result<Self, Error> {
        let path = path.as_ref();
        let reader = image::io::Reader::open(path)
            .with_context(|| format!("Could not read image from path {}", path.display()))?;
        Self::from_image_reader(reader)
    }

    /// Translates the image over to position.
    pub fn set_position(&mut self, position: impl Into<Position>) {
        self.position = Some(position.into());
    }

    /// Translates the image over to position and returns it.
    pub fn with_position(mut self, position: impl Into<Position>) -> Self {
        self.set_position(position);
        self
    }

    /// Scales the image.
    pub fn set_scale(&mut self, scale: impl Into<Scale>) {
        self.scale = scale.into();
    }

    /// Scales the image and returns it.
    pub fn with_scale(mut self, scale: impl Into<Scale>) -> Self {
        self.set_scale(scale);
        self
    }

    /// Sets the alignment to use for this image.
    pub fn set_alignment(&mut self, alignment: impl Into<Alignment>) {
        self.alignment = alignment.into();
    }

    /// Sets the alignment to use for this image and returns it.
    pub fn with_alignment(mut self, alignment: impl Into<Alignment>) -> Self {
        self.set_alignment(alignment);
        self
    }

    /// Determines the offset from left-side based on provided Alignment.
    fn get_offset(&self, width: Mm, max_width: Mm) -> Position {
        let horizontal_offset = match self.alignment {
            Alignment::Left => Mm::default(),
            Alignment::Center => (max_width - width) / 2.0,
            Alignment::Right => max_width - width,
        };
        Position::new(horizontal_offset, 0)
    }

    /// Calculates a guess for the size of the image based on the dpi/pixel-count/scale.
    fn get_size(&self) -> Size {
        let mmpi: f32 = 25.4; // millimeters per inch
                              // Assume 300 DPI to be consistent with printpdf.
        let dpi: f32 = self.dpi.unwrap_or(300.0);
        let (px_width, px_height) = self.data.dimensions();
        let (scale_width, scale_height): (f32, f32) = (self.scale.x, self.scale.y);
        Size::new(
            mmpi * ((scale_width * px_width as f32) / dpi),
            mmpi * ((scale_height * px_height as f32) / dpi),
        )
    }

    /// Sets the clockwise rotation of the image around the bottom left corner.
    pub fn set_clockwise_rotation(&mut self, rotation: impl Into<Rotation>) {
        self.rotation = rotation.into();
    }

    /// Sets the clockwise rotation of the image around the bottom left corner and then returns the
    /// image.
    pub fn with_clockwise_rotation(mut self, rotation: impl Into<Rotation>) -> Self {
        self.set_clockwise_rotation(rotation);
        self
    }

    /// Sets the expected DPI of the encoded image.
    pub fn set_dpi(&mut self, dpi: f32) {
        self.dpi = Some(dpi);
    }

    /// Sets the expected DPI of the encoded image and returns it.
    pub fn with_dpi(mut self, dpi: f32) -> Self {
        self.set_dpi(dpi);
        self
    }
}

impl Element for Image {
    fn render(
        &mut self,
        _context: &Context,
        area: render::Area<'_>,
        _style: style::Style,
    ) -> Result<RenderResult, Error> {
        let mut result = RenderResult::default();
        let true_size = self.get_size();
        let (bb_origin, bb_size) = bounding_box_offset_and_size(&self.rotation, &true_size);

        let mut position: Position = if let Some(position) = self.position {
            position
        } else {
            // Update the result size to be based on the bounding-box size/offset.
            result.size = bb_size;

            // No position override given; so we calculate the Alignment offset based on
            // the area-size and width of the bounding box.
            self.get_offset(bb_size.width, area.size().width)
        };

        // Fix the position with the bounding-box's origin which was changed from
        // (0,0) when it was rotated in any way.
        position += bb_origin;

        // Insert/render the image with the overridden/calculated position.
        area.add_image(&self.data, position, self.scale, self.rotation, self.dpi);

        // Always false as we can't safely do this unless we want to try to do "sub-images".
        // This is technically possible with the `image` package, but it is potentially more
        // work than necessary. I'd rather support an "Auto-Scale" method to fit to area.
        result.has_more = false;

        Ok(result)
    }
}

/// Given the Size of a box (width/height), compute the bounding-box size and offset when
/// rotated some degrees.  The offset is the distance from the top-left corner of the bounding box
/// to the (originally) lower-left corner of the image.
#[allow(clippy::manual_range_contains)]
fn bounding_box_offset_and_size(rotation: &Rotation, size: &Size) -> (Position, Size) {
    // alpha = rotation, beta = 90 - rotation
    let alpha = rotation.degrees.to_radians();
    let beta = (90.0 - rotation.degrees).to_radians();

    // s* = sin of *
    let sa = alpha.sin();
    let sb = beta.sin();

    // Bounding box calculation, based on
    // https://math.stackexchange.com/questions/1628657/dimensions-of-a-rectangle-containing-a-rotated-rectangle
    let width = (size.width.0 * sb).abs() + (size.height.0 * sa).abs();
    let height = (size.height.0 * sb).abs() + (size.width.0 * sa).abs();
    let bb_size = Size::new(width, height);

    // Offset calculation -- to follow the calculations, consider the rotated rectangles, their
    // bounding boxes and the triangles between them
    let bb_position = if rotation.degrees < -180.0 {
        unreachable!(
            "Rotations must be in the range -180.0..=180.0, but got: {}",
            rotation.degrees
        );
    } else if rotation.degrees <= -90.0 {
        Position::new(size.width.0 * alpha.cos().abs(), 0)
    } else if rotation.degrees <= 0.0 {
        Position::new(0, size.height.0 * alpha.cos())
    } else if rotation.degrees <= 90.0 {
        Position::new(size.height.0 * beta.cos(), bb_size.height.0)
    } else if rotation.degrees <= 180.0 {
        Position::new(bb_size.width.0, size.width.0 * beta.cos())
    } else {
        unreachable!(
            "Rotations must be in the range -180.0..=180.0, but got: {}",
            rotation.degrees
        );
    };

    (bb_position, bb_size)
}

#[cfg(test)]
mod tests {
    use super::bounding_box_offset_and_size;
    use crate::{Position, Rotation, Size};
    use float_cmp::approx_eq;

    macro_rules! assert_approx_eq {
        ($typ:ty, $lhs:expr, $rhs:expr) => {
            let left = $lhs;
            let right = $rhs;
            assert!(
                approx_eq!($typ, left, right, epsilon = 100.0 * f32::EPSILON, ulps = 10),
                "assertion failed: `(left approx_eq right)`
  left: `{:?}`,
 right: `{:?}`",
                left,
                right
            );
        };
    }

    fn test_position(size: Size, rotation: f32, position: Position) {
        println!("rotation = {}", rotation);
        let rotation = Rotation::from(rotation);
        assert_approx_eq!(
            Position,
            position,
            bounding_box_offset_and_size(&rotation, &size).0
        );
    }

    #[test]
    fn test_bounding_box_size_square_0_deg() {
        let size = Size::new(100, 100);
        for rotation in &[-180.0, -90.0, 0.0, 90.0, 180.0] {
            println!("rotation = {}", rotation);
            let rotation = Rotation::from(*rotation);
            assert_approx_eq!(Size, size, bounding_box_offset_and_size(&rotation, &size).1);
        }
    }

    #[test]
    fn test_bounding_box_size_square_30_deg() {
        let size = Size::new(100, 100);
        let bb_width = (60.0f32.to_radians().sin() + 30.0f32.to_radians().sin()) * size.width.0;
        let bb_size = Size::new(bb_width, bb_width);
        for rotation in &[-150.0, -120.0, -30.0, -60.0, 30.0, 60.0, 120.0, 150.0] {
            println!("rotation = {}", rotation);
            let rotation = Rotation::from(*rotation);
            assert_approx_eq!(
                Size,
                bb_size,
                bounding_box_offset_and_size(&rotation, &size).1
            );
        }
    }

    #[test]
    fn test_bounding_box_size_square_45_deg() {
        let size = Size::new(100, 100);
        let bb_width = (2.0f32 * size.width.0.powf(2.0)).sqrt();
        let bb_size = Size::new(bb_width, bb_width);
        for rotation in &[-135.0, -45.0, 45.0, 135.0] {
            println!("rotation = {}", rotation);
            let rotation = Rotation::from(*rotation);
            assert_approx_eq!(
                Size,
                bb_size,
                bounding_box_offset_and_size(&rotation, &size).1
            );
        }
    }

    #[test]
    fn test_bounding_box_position_square_30_deg() {
        let size = Size::new(100, 100);
        let bb_width =
            30.0f32.to_radians().sin() * size.width.0 + 60.0f32.to_radians().sin() * size.height.0;

        let w30 = 30.0f32.to_radians().cos() * size.width.0;
        let w60 = 60.0f32.to_radians().cos() * size.width.0;

        test_position(size, -150.0, Position::new(w30, 0));
        test_position(size, -120.0, Position::new(w60, 0));
        test_position(size, -60.0, Position::new(0, w60));
        test_position(size, -30.0, Position::new(0, w30));
        test_position(size, 30.0, Position::new(w60, bb_width));
        test_position(size, 60.0, Position::new(w30, bb_width));
        test_position(size, 120.0, Position::new(bb_width, bb_width - w60));
        test_position(size, 150.0, Position::new(bb_width, bb_width - w30));
    }

    #[test]
    fn test_bounding_box_position_square_45_deg() {
        let size = Size::new(100, 100);
        let bb_width = (2.0f32 * size.width.0.powf(2.0)).sqrt();

        test_position(size, -135.0, Position::new(bb_width / 2.0, 0));
        test_position(size, -45.0, Position::new(0, bb_width / 2.0));
        test_position(size, 45.0, Position::new(bb_width / 2.0, bb_width));
        test_position(size, 135.0, Position::new(bb_width, bb_width / 2.0));
    }

    #[test]
    fn test_bounding_box_position_square_90_deg() {
        let size = Size::new(100, 100);
        test_position(size, -180.0, Position::new(100, 0));
        test_position(size, -90.0, Position::new(0, 0));
        test_position(size, 0.0, Position::new(0, 100));
        test_position(size, 90.0, Position::new(100, 100));
        test_position(size, 180.0, Position::new(100, 0));
    }

    #[test]
    fn test_bounding_box_size_rectangle_0_deg() {
        let size = Size::new(200, 100);
        for rotation in &[-180.0, 0.0, 180.0] {
            println!("rotation = {}", rotation);
            let rotation = Rotation::from(*rotation);
            assert_approx_eq!(Size, size, bounding_box_offset_and_size(&rotation, &size).1);
        }
    }

    #[test]
    fn test_bounding_box_size_rectangle_30_deg() {
        let size = Size::new(200, 100);
        let bb_width =
            60.0f32.to_radians().sin() * size.width.0 + 30.0f32.to_radians().sin() * size.height.0;
        let bb_height =
            60.0f32.to_radians().sin() * size.height.0 + 30.0f32.to_radians().sin() * size.width.0;
        let bb_size = Size::new(bb_width, bb_height);
        for rotation in &[-150.0, -30.0, 30.0, 150.0] {
            println!("rotation = {}", rotation);
            let rotation = Rotation::from(*rotation);
            assert_approx_eq!(
                Size,
                bb_size,
                bounding_box_offset_and_size(&rotation, &size).1
            );
        }
    }

    #[test]
    fn test_bounding_box_size_rectangle_45_deg() {
        let size = Size::new(200, 100);
        let bb_width = 45.0f32.to_radians().sin() * (size.width.0 + size.height.0);
        let bb_size = Size::new(bb_width, bb_width);
        for rotation in &[-135.0, -45.0, 45.0, 135.0] {
            println!("rotation = {}", rotation);
            let rotation = Rotation::from(*rotation);
            assert_approx_eq!(
                Size,
                bb_size,
                bounding_box_offset_and_size(&rotation, &size).1
            );
        }
    }

    #[test]
    fn test_bounding_box_size_rectangle_60_deg() {
        let size = Size::new(200, 100);
        let bb_width =
            30.0f32.to_radians().sin() * size.width.0 + 60.0f32.to_radians().sin() * size.height.0;
        let bb_height =
            30.0f32.to_radians().sin() * size.height.0 + 60.0f32.to_radians().sin() * size.width.0;
        let bb_size = Size::new(bb_width, bb_height);
        for rotation in &[-120.0, -60.0, 60.0, 120.0] {
            println!("rotation = {}", rotation);
            let rotation = Rotation::from(*rotation);
            assert_approx_eq!(
                Size,
                bb_size,
                bounding_box_offset_and_size(&rotation, &size).1
            );
        }
    }

    #[test]
    fn test_bounding_box_size_rectangle_90_deg() {
        let size = Size::new(200, 100);
        let bb_size = Size::new(100, 200);
        for rotation in &[-90.0, 90.0] {
            println!("rotation = {}", rotation);
            let rotation = Rotation::from(*rotation);
            assert_approx_eq!(
                Size,
                bb_size,
                bounding_box_offset_and_size(&rotation, &size).1
            );
        }
    }

    #[test]
    fn test_bounding_box_position_rectangle_30_deg() {
        let size = Size::new(200, 100);
        let bb_width =
            30.0f32.to_radians().sin() * size.width.0 + 60.0f32.to_radians().sin() * size.height.0;
        let bb_height =
            30.0f32.to_radians().sin() * size.height.0 + 60.0f32.to_radians().sin() * size.width.0;

        let h30 = 30.0f32.to_radians().cos() * size.height.0;
        let h60 = 60.0f32.to_radians().cos() * size.height.0;
        let w30 = 30.0f32.to_radians().cos() * size.width.0;
        let w60 = 60.0f32.to_radians().cos() * size.width.0;

        test_position(size, -150.0, Position::new(w30, 0));
        test_position(size, -120.0, Position::new(w60, 0));
        test_position(size, -60.0, Position::new(0, h60));
        test_position(size, -30.0, Position::new(0, h30));
        test_position(size, 30.0, Position::new(h60, bb_width));
        test_position(size, 60.0, Position::new(h30, bb_height));
        test_position(size, 120.0, Position::new(bb_width, bb_height - h60));
        test_position(size, 150.0, Position::new(bb_height, bb_width - h30));
    }

    #[test]
    fn test_bounding_box_position_rectangle_45_deg() {
        let size = Size::new(200, 100);
        let bb_width = 45.0f32.to_radians().sin() * (size.width.0 + size.height.0);

        test_position(size, -135.0, Position::new(2.0 * bb_width / 3.0, 0));
        test_position(size, -45.0, Position::new(0, bb_width / 3.0));
        test_position(size, 45.0, Position::new(bb_width / 3.0, bb_width));
        test_position(size, 135.0, Position::new(bb_width, 2.0 * bb_width / 3.0));
    }

    #[test]
    fn test_bounding_box_position_rectangle_90_deg() {
        let size = Size::new(200, 100);
        test_position(size, -180.0, Position::new(200, 0));
        test_position(size, -90.0, Position::new(0, 0));
        test_position(size, 0.0, Position::new(0, 100));
        test_position(size, 90.0, Position::new(100, 200));
        test_position(size, 180.0, Position::new(200, 0));
    }
}
