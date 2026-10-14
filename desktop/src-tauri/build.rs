use std::{fs::File, io::BufWriter, path::Path};

fn main() {
    let icon = Path::new("icons/icon.png");
    if !icon.exists() {
        std::fs::create_dir_all(icon.parent().expect("icon directory"))
            .expect("create icon directory");
        let file = File::create(icon).expect("create application icon");
        let mut encoder = png::Encoder::new(BufWriter::new(file), 512, 512);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().expect("write icon header");
        let mut pixels = Vec::with_capacity(512 * 512 * 4);
        for y in 0..512_i32 {
            for x in 0..512_i32 {
                let rounded = (x < 112 && y < 112 && (x - 112).pow(2) + (y - 112).pow(2) > 112_i32.pow(2))
                    || (x > 399 && y < 112 && (x - 399).pow(2) + (y - 112).pow(2) > 112_i32.pow(2))
                    || (x < 112 && y > 399 && (x - 112).pow(2) + (y - 399).pow(2) > 112_i32.pow(2))
                    || (x > 399 && y > 399 && (x - 399).pow(2) + (y - 399).pow(2) > 112_i32.pow(2));
                let rgba = if rounded {
                    [0, 0, 0, 0]
                } else {
                    [23, 63, 58, 255]
                };
                pixels.extend_from_slice(&rgba);
            }
        }
        writer.write_image_data(&pixels).expect("write application icon");
    }
    println!("cargo:rerun-if-changed=icons/icon.svg");
    tauri_build::build()
}
