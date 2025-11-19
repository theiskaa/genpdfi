//! Comprehensive test for subsetting with Unicode support
//! Based on history.md achievements

use genpdfi::subsetting::subset_font;

fn main() {
    // Test data from history.md - 8+ writing systems
    let test_cases = vec![
        ("Latin", "Hello World! ăâîșț"),
        ("Arabic", "مرحبا بالعالم"),
        ("Hebrew", "שלום עולם"),
        ("CJK", "你好世界 こんにちは世界 안녕하세요 세계"),
        ("Cyrillic", "Привет мир"),
        ("Greek", "Γεια σου κόσμε"),
        ("Thai", "สวัสดีชาวโลก"),
        ("Emoji", "👋🌍😊🎉"),
        ("Mixed", "Hello مرحبا שלום 你好 Привет Γεια สวัสดี 👋"),
    ];

    // Load a test font (we'll use a system font)
    let font_paths = vec![
        "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
        "/System/Library/Fonts/Helvetica.ttc",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    ];

    let font_data = font_paths.iter()
        .find_map(|path| std::fs::read(path).ok())
        .expect("No suitable font found");

    println!("Original font size: {} bytes", font_data.len());
    println!("\n=== Testing Font Subsetting ===\n");

    for (name, text) in test_cases {
        print!("Testing {}: \"{}\" ... ", name, text);

        match subset_font(&font_data, text) {
            Ok(subset_data) => {
                let reduction = 100.0 * (1.0 - (subset_data.len() as f64 / font_data.len() as f64));
                println!("✅ SUCCESS");
                println!("  Subset size: {} bytes", subset_data.len());
                println!("  Reduction: {:.1}%", reduction);
                println!("  Characters: {}", text.chars().count());
            }
            Err(e) => {
                println!("❌ FAILED: {:?}", e);
            }
        }
        println!();
    }

    // Performance test (from history.md: 190k chars/second)
    println!("\n=== Performance Test ===\n");
    let large_text = "Hello World! ".repeat(10000); // ~120k characters
    let char_count = large_text.chars().count();

    let start = std::time::Instant::now();
    match subset_font(&font_data, &large_text) {
        Ok(subset_data) => {
            let duration = start.elapsed();
            let chars_per_sec = char_count as f64 / duration.as_secs_f64();

            println!("Large text subsetting:");
            println!("  Characters processed: {}", char_count);
            println!("  Time: {:.2}s", duration.as_secs_f64());
            println!("  Speed: {:.0} chars/second", chars_per_sec);
            println!("  Subset size: {} bytes", subset_data.len());

            if chars_per_sec > 100_000.0 {
                println!("  ✅ Performance target met (>100k chars/sec)");
            } else {
                println!("  ⚠️  Below target performance");
            }
        }
        Err(e) => {
            println!("❌ Performance test failed: {:?}", e);
        }
    }
}
