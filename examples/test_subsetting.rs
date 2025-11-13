//! Test example for font subsetting functionality

use genpdfi::subsetting::subset_font;
use std::fs;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let font_path = "../tests/fonts/NotoSans.ttf";
    let font_data = fs::read(font_path)?;

    println!(
        "Original font size: {} bytes ({:.2} MB)",
        font_data.len(),
        font_data.len() as f64 / 1_048_576.0
    );

    let romanian_text = "ăâîșțĂÂÎȘȚ Hello World!";
    let subset_data = subset_font(&font_data, romanian_text)?;

    println!(
        "Subset font size: {} bytes ({:.2} KB)",
        subset_data.len(),
        subset_data.len() as f64 / 1024.0
    );
    println!(
        "Size reduction: {:.1}%",
        (1.0 - subset_data.len() as f64 / font_data.len() as f64) * 100.0
    );

    fs::write("subset_test.ttf", &subset_data)?;
    println!("Subset font saved to: subset_test.ttf");

    Ok(())
}
