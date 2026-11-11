//! The QR code of a device request: 25 by 25 modules, drawn as the board draws it (black on a
//! white tile, in both themes, so a phone camera reads it).
//!
//! The code opens the request on the approving device and holds NO pairing code (task 1734).

use rune::{Color, El, Shape, canvas};

/// The board's sample code (DW-A07), row by row: `#` is a dark module.
const BOARD: [&str; 25] = [
    "#######..###...##.#######",
    "#.....#.#.#.#..#.##.....#",
    "#.###.#...#####...#.###.#",
    "#.###.#..#..##..###.###.#",
    "#.###.#.#..#..#.###.###.#",
    "#.....#...........#.....#",
    "#######.##.#..#..########",
    ".##..#..##..##..#..##..##",
    "#####......#####......###",
    "..#.#.#.#.#..#.#.#.#.#..#",
    "...###...##...###...##...",
    "#...##...###...##...###..",
    ".#..#.#.#.#.#..#.#.#.#.#.",
    "####......#####......####",
    ".##..##..#..##..##..#..##",
    "#..#.##.#..#..#.##.#..#..",
    ".........................",
    "#..#..#.##.#..#..#.##.#..",
    "#######.##..##..#..##..##",
    "#.....#....#####......###",
    "#.###.#.#.#..#.#.#.#.#..#",
    "#.###.#..##...###...##...",
    "#.###.#..###...##...###..",
    "#.....#.#.#.#..#.#.#.#.#.",
    "#######...#####......####",
];

pub fn board_modules() -> Vec<Vec<bool>> {
    BOARD
        .iter()
        .map(|row| row.bytes().map(|b| b == b'#').collect())
        .collect()
}

/// The modules as one canvas `size` points across: each run of dark modules in a row is one rectangle.
pub fn code(modules: &[Vec<bool>], size: f32) -> El {
    let n = modules.len().max(1) as f32;
    let mut shape = Shape::new().view_box(n, n);
    for (y, row) in modules.iter().enumerate() {
        let mut x = 0;
        while x < row.len() {
            if !row[x] {
                x += 1;
                continue;
            }
            let start = x;
            while x < row.len() && row[x] {
                x += 1;
            }
            let (x0, x1, y0, y1) = (start as f32, x as f32, y as f32, y as f32 + 1.0);
            shape = shape.fill(vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]], Color::BLACK);
        }
    }
    canvas(shape)
        .key("qr-code")
        .role("image")
        .label("QR code that opens this sign-in request, without the code")
        .size(size, size)
}
