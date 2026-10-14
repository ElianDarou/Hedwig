use std::{fs, path::Path};

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffff_u32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320_u32 & (0_u32.wrapping_sub(crc & 1)));
        }
    }
    !crc
}

fn adler32(bytes: &[u8]) -> u32 {
    let (mut a, mut b) = (1_u32, 0_u32);
    for &byte in bytes {
        a = (a + u32::from(byte)) % 65_521;
        b = (b + a) % 65_521;
    }
    (b << 16) | a
}

fn chunk(output: &mut Vec<u8>, name: &[u8; 4], data: &[u8]) {
    output.extend_from_slice(&(data.len() as u32).to_be_bytes());
    output.extend_from_slice(name);
    output.extend_from_slice(data);
    let mut checked = name.to_vec();
    checked.extend_from_slice(data);
    output.extend_from_slice(&crc32(&checked).to_be_bytes());
}

fn inside_circle(x: i32, y: i32, center_x: i32, center_y: i32, radius: i32) -> bool {
    (x - center_x).pow(2) + (y - center_y).pow(2) <= radius.pow(2)
}

fn main() {
    const SIZE: i32 = 512;
    let mut raw = Vec::with_capacity((SIZE * (SIZE * 4 + 1)) as usize);
    for y in 0..SIZE {
        raw.push(0);
        for x in 0..SIZE {
            let corner = 112;
            let outside_rounding = (x < corner && y < corner && !inside_circle(x, y, corner, corner, corner))
                || (x >= SIZE - corner && y < corner && !inside_circle(x, y, SIZE - corner - 1, corner, corner))
                || (x < corner && y >= SIZE - corner && !inside_circle(x, y, corner, SIZE - corner - 1, corner))
                || (x >= SIZE - corner && y >= SIZE - corner && !inside_circle(x, y, SIZE - corner - 1, SIZE - corner - 1, corner));
            let cloud = inside_circle(x, y, 206, 237, 92)
                || inside_circle(x, y, 292, 205, 116)
                || inside_circle(x, y, 356, 258, 76)
                || (145..=372).contains(&x) && (230..=322).contains(&y);
            let arrow = (244..=268).contains(&x) && (223..=335).contains(&y)
                || ((x - 256).abs() <= 62 && (y - 207).abs() <= 62 && (x - 256).abs() + (y - 207).abs() <= 62 && y <= 225);
            let rgba = if outside_rounding {
                [0, 0, 0, 0]
            } else if arrow {
                [201, 117, 76, 255]
            } else if cloud {
                [243, 239, 228, 255]
            } else {
                [23, 63, 58, 255]
            };
            raw.extend_from_slice(&rgba);
        }
    }

    let mut zlib = vec![0x78, 0x01];
    let mut remaining = raw.as_slice();
    while !remaining.is_empty() {
        let length = remaining.len().min(65_535);
        let final_block = length == remaining.len();
        zlib.push(u8::from(final_block));
        zlib.extend_from_slice(&(length as u16).to_le_bytes());
        zlib.extend_from_slice(&(!(length as u16)).to_le_bytes());
        zlib.extend_from_slice(&remaining[..length]);
        remaining = &remaining[length..];
    }
    zlib.extend_from_slice(&adler32(&raw).to_be_bytes());

    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut header = Vec::new();
    header.extend_from_slice(&(SIZE as u32).to_be_bytes());
    header.extend_from_slice(&(SIZE as u32).to_be_bytes());
    header.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(&mut png, b"IHDR", &header);
    chunk(&mut png, b"IDAT", &zlib);
    chunk(&mut png, b"IEND", &[]);

    let target = Path::new("desktop/src-tauri/icons/icon.png");
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::write(target, png).unwrap();
}
