use std::{fs, path::Path};

const SIZE: usize = 256;

fn main() {
    let mut raw = Vec::with_capacity((SIZE * 4 + 1) * SIZE);
    for y in 0..SIZE {
        raw.push(0); // PNG filter: none
        for x in 0..SIZE {
            let in_u = ((52..=88).contains(&x) || (168..=204).contains(&x))
                && (58..=151).contains(&y)
                || (52..=204).contains(&x)
                    && (132..=196).contains(&y)
                    && {
                        let dx = x as isize - 128;
                        let dy = y as isize - 132;
                        dx * dx + dy * dy <= 76 * 76
                    };
            let cutout = (88..=168).contains(&x) && y <= 151;
            let color = if in_u && !cutout {
                [245, 243, 237, 255]
            } else {
                [37, 58, 42, 255]
            };
            raw.extend_from_slice(&color);
        }
    }

    let mut zlib = vec![0x78, 0x01];
    for (index, block) in raw.chunks(65_535).enumerate() {
        zlib.push(u8::from((index + 1) * 65_535 >= raw.len()));
        let length = block.len() as u16;
        zlib.extend_from_slice(&length.to_le_bytes());
        zlib.extend_from_slice(&(!length).to_le_bytes());
        zlib.extend_from_slice(block);
    }
    zlib.extend_from_slice(&adler32(&raw).to_be_bytes());

    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut header = Vec::new();
    header.extend_from_slice(&(SIZE as u32).to_be_bytes());
    header.extend_from_slice(&(SIZE as u32).to_be_bytes());
    header.extend_from_slice(&[8, 6, 0, 0, 0]); // 8-bit RGBA
    chunk(&mut png, b"IHDR", &header);
    chunk(&mut png, b"IDAT", &zlib);
    chunk(&mut png, b"IEND", &[]);

    let output = Path::new("src-tauri/icons/icon.png");
    fs::write(output, png).expect("write icon.png");
    println!("generated {}", output.display());
}

fn chunk(output: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    output.extend_from_slice(&(data.len() as u32).to_be_bytes());
    output.extend_from_slice(kind);
    output.extend_from_slice(data);
    let mut crc_input = kind.to_vec();
    crc_input.extend_from_slice(data);
    output.extend_from_slice(&crc32(&crc_input).to_be_bytes());
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffff_u32;
    for byte in bytes {
        crc ^= *byte as u32;
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & (0_u32.wrapping_sub(crc & 1)));
        }
    }
    !crc
}

fn adler32(bytes: &[u8]) -> u32 {
    let (mut a, mut b) = (1_u32, 0_u32);
    for byte in bytes {
        a = (a + *byte as u32) % 65_521;
        b = (b + a) % 65_521;
    }
    (b << 16) | a
}
