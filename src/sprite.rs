use windows_sys::Win32::UI::WindowsAndMessaging::{CreateIconFromResourceEx, HICON};

type Pixels = &'static [(usize, usize)];

#[derive(Clone, Copy)]
pub enum Mood {
    Running,
    Straining,
    Alarmed,
    Unknown,
}

impl Mood {
    pub fn from_load(load: Option<u32>) -> Self {
        match load {
            Some(0..=69) => Self::Running,
            Some(70..=84) => Self::Straining,
            Some(_) => Self::Alarmed,
            None => Self::Unknown,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Running => "Running",
            Self::Straining => "Straining",
            Self::Alarmed => "Alarmed",
            Self::Unknown => "RAM unavailable",
        }
    }

    pub fn create_icon(self, step: bool) -> HICON {
        let (body, accent) = match self {
            Self::Running => (0xff63e8ad, 0xffc0ffe3),
            Self::Straining => (0xffffbf59, 0xffffedd0),
            Self::Alarmed => (0xffff6882, 0xffffd5df),
            Self::Unknown => (0xffa4b1bf, 0xffe0e7ef),
        };
        let face = 0xff172534;
        let mut pixels = [0u32; 256];
        let mut pixel = |x: usize, y: usize, color| pixels[y * 16 + x] = color;
        let y = 3 + usize::from(step && !matches!(self, Self::Unknown));
        for row in y..y + 8 {
            for x in 4..12 {
                if (row != y && row != y + 7) || (5..11).contains(&x) {
                    pixel(x, row, body);
                }
            }
        }
        for x in 6..10 {
            pixel(x, y, accent);
        }
        for x in [6, 9] {
            pixel(x, y + 3, face);
        }

        let (mouth, limbs, effects): (Pixels, Pixels, Pixels) = match (self, step) {
            (Self::Running, false) => (
                &[(6, 5), (7, 6), (8, 6), (9, 5)],
                &[
                    (3, 7),
                    (2, 8),
                    (12, 6),
                    (13, 5),
                    (6, 11),
                    (5, 12),
                    (4, 12),
                    (10, 11),
                    (11, 12),
                    (12, 13),
                    (13, 13),
                ],
                &[(0, 5), (1, 5), (2, 5), (0, 10), (1, 10)],
            ),
            (Self::Running, true) => (
                &[(6, 5), (7, 6), (8, 6), (9, 5)],
                &[
                    (3, 7),
                    (2, 6),
                    (12, 8),
                    (13, 8),
                    (6, 12),
                    (7, 13),
                    (8, 13),
                    (10, 12),
                    (9, 13),
                    (9, 14),
                    (10, 14),
                ],
                &[(0, 6), (1, 6), (1, 11), (2, 11), (3, 11)],
            ),
            (Self::Straining, _) => (
                &[(6, 6), (7, 5), (8, 5), (9, 6)],
                &[
                    (3, 9),
                    (2, 10),
                    (12, 9),
                    (13, 10),
                    (5, 11),
                    (5, 12),
                    (4, 13),
                    (5, 13),
                    (10, 11),
                    (10, 12),
                    (10, 13),
                    (11, 13),
                ],
                if step {
                    &[(13, 5), (13, 6), (14, 6)]
                } else {
                    &[(12, 2), (12, 3), (13, 3)]
                },
            ),
            (Self::Alarmed, _) => (
                &[(7, 5), (8, 5), (7, 6), (8, 6)],
                if step {
                    &[
                        (3, 8),
                        (2, 7),
                        (1, 7),
                        (12, 6),
                        (13, 5),
                        (14, 5),
                        (5, 12),
                        (6, 13),
                        (7, 13),
                        (10, 12),
                        (11, 13),
                        (12, 14),
                    ]
                } else {
                    &[
                        (3, 5),
                        (2, 4),
                        (1, 4),
                        (12, 7),
                        (13, 6),
                        (14, 6),
                        (5, 11),
                        (4, 12),
                        (3, 12),
                        (10, 11),
                        (10, 12),
                        (11, 13),
                    ]
                },
                &[(13, 0), (13, 1), (13, 3)],
            ),
            (Self::Unknown, _) => (&[(7, 5), (8, 5)], &[(6, 11), (9, 11)], &[]),
        };

        for &(x, row) in mouth {
            pixel(x, y + row, face);
        }
        for &(x, row) in limbs {
            pixel(x, row, body);
        }
        for &(x, row) in effects {
            pixel(x, row, accent);
        }

        // An icon resource uses a bottom-up 32-bit DIB followed by a one-bit mask.
        let mut dib = [0u8; 40 + 16 * 16 * 4 + 16 * 4];
        for (index, value) in [40u32, 16, 32, 0x0020_0001].into_iter().enumerate() {
            dib[index * 4..index * 4 + 4].copy_from_slice(&value.to_le_bytes());
        }
        let (color, mask) = dib[40..].split_at_mut(16 * 16 * 4);
        for (row, pixels) in pixels.chunks_exact(16).rev().enumerate() {
            for (column, &pixel) in pixels.iter().enumerate() {
                let offset = (row * 16 + column) * 4;
                color[offset..offset + 4].copy_from_slice(&pixel.to_le_bytes());
                if pixel == 0 {
                    mask[row * 4 + column / 8] |= 0x80 >> (column % 8);
                }
            }
        }
        unsafe {
            CreateIconFromResourceEx(dib.as_ptr(), dib.len() as u32, 1, 0x0003_0000, 16, 16, 0)
        }
    }
}
