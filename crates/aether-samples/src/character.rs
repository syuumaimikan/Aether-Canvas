//! Aether-chan: the sample character, painted in code.
//!
//! A bust-up anime-style girl in a sailor uniform, painted as separate
//! parts named the way the auto-rigger understands (see
//! `aether_rig::autorig`): back hair, uniform, neck, face, eyes (white,
//! iris clipped to it, lashes), brows, nose, mouth (line and inside),
//! blush, side locks, bangs, an ahoge and a hair ribbon. Every part has its
//! own line art, cel shading and highlights, so it stays whole while it
//! deforms.

use crate::paint::{Canvas, Color, Mode, Paint, Path, Stroke};
use aether_core::math::Vec2;
use aether_core::LayerId;
use aether_document::layer::Layer;
use aether_document::Document;

/// Canvas size.
pub const WIDTH: u32 = 768;
pub const HEIGHT: u32 = 1024;
/// The character's vertical axis.
const MID: f32 = 384.0;

// Palette.
const LINE: Color = Color::hex(0x3a2244);
const SKIN_TOP: Color = Color::hex(0xfff3ec);
const SKIN: Color = Color::hex(0xffe4d6);
const SKIN_SHADE: Color = Color::hex(0xf3c2c2);
const SKIN_LINE: Color = Color::hex(0xa8606a);
const HAIR_LIGHT: Color = Color::hex(0x8c84ec);
const HAIR: Color = Color::hex(0x6453c8);
const HAIR_DARK: Color = Color::hex(0x3a2c8a);
const HAIR_SHADE: Color = Color::hex(0xa99fe0);
const HAIR_SHINE: Color = Color::hex(0xc9c2ff);
const HAIR_LINE: Color = Color::hex(0x241858);
const IRIS_TOP: Color = Color::hex(0x6e2604);
const IRIS_MID: Color = Color::hex(0xd8611a);
const IRIS_LOW: Color = Color::hex(0xffcf5c);
const PUPIL: Color = Color::hex(0x2a0c04);
const LASH: Color = Color::hex(0x2a1638);
const BROW: Color = Color::hex(0x2e2266);
const SHIRT: Color = Color::hex(0xfbfaff);
const SHIRT_SHADE: Color = Color::hex(0xcfcce8);
const NAVY: Color = Color::hex(0x26356e);
const NAVY_LIGHT: Color = Color::hex(0x3a4d93);
const RED: Color = Color::hex(0xe23a50);
const RED_DARK: Color = Color::hex(0xa51f37);
const WHITE: Color = Color::hex(0xffffff);

/// The painted layers, bottom to top.
pub struct Parts {
    pub back_hair: LayerId,
    pub uniform: LayerId,
    pub neck: LayerId,
    pub scarf: LayerId,
    pub face: LayerId,
    pub cheek: LayerId,
    pub nose: LayerId,
    pub mouth_open: LayerId,
    pub mouth: LayerId,
    pub eye_white: [LayerId; 2],
    pub iris: [LayerId; 2],
    pub lash: [LayerId; 2],
    pub side_hair: [LayerId; 2],
    pub front_hair: LayerId,
    pub ahoge: LayerId,
    pub brow: [LayerId; 2],
    pub hair_ribbon: LayerId,
}

fn mirror_x(x: f32) -> f32 {
    2.0 * MID - x
}

/// Points of the right half mirrored onto the left, for symmetric outlines
/// traced clockwise from the top centre.
fn symmetric(right: &[(f32, f32)]) -> Vec<(f32, f32)> {
    let mut all = right.to_vec();
    all.extend(right.iter().rev().skip(1).map(|&(x, y)| (mirror_x(x), y)));
    all
}

fn face_outline() -> Path {
    let points = symmetric(&[
        (MID, 198.0),
        (452.0, 212.0),
        (500.0, 262.0),
        (514.0, 340.0),
        (506.0, 420.0),
        (480.0, 482.0),
        (432.0, 530.0),
        (MID, 550.0),
    ]);
    Path::new()
        .move_to(MID, 198.0)
        .smooth_through(&points[1..])
        .close()
}

fn paint_face() -> Canvas {
    let mut c = Canvas::new(WIDTH, HEIGHT);
    let face = face_outline();
    c.fill(&face, Paint::vertical(200.0, SKIN_TOP, 550.0, SKIN), Mode::Over);
    // Shadow the bangs cast on the forehead.
    let bang_shadow = bang_shadow();
    c.fill(&bang_shadow, SKIN_SHADE, Mode::Multiply);
    // Soft shading along the cheeks and under the jaw's edge.
    for side in [1.0f32, -1.0] {
        let x = |v: f32| MID + (v - MID) * side;
        let cheek_shade = Path::new()
            .move_to(x(516.0), 300.0)
            .smooth_through(&[
                (x(498.0), 360.0),
                (x(488.0), 430.0),
                (x(460.0), 490.0),
                (x(430.0), 540.0),
                (x(520.0), 540.0),
            ])
            .close();
        c.soft(&cheek_shade, 6.0, SKIN_SHADE.alpha(0.55), Mode::Multiply);
    }
    // A permanent hint of colour on the cheeks.
    for x in [304.0, 464.0] {
        c.soft(
            &Path::ellipse(x, 452.0, 30.0, 12.0),
            9.0,
            Color::hex(0xff9aaa).alpha(0.35),
            Mode::Atop,
        );
    }
    // Jaw line: tapered, darker at the chin.
    let jaw = Path::new().move_to(262.0, 420.0).smooth_through(&[
        (288.0, 482.0),
        (336.0, 530.0),
        (MID, 549.0),
        (432.0, 530.0),
        (480.0, 482.0),
        (506.0, 420.0),
    ]);
    c.stroke(
        &jaw,
        Stroke {
            width: 3.6,
            taper_in: 0.3,
            taper_out: 0.3,
        },
        SKIN_LINE,
        Mode::Over,
    );
    c
}

fn paint_neck() -> Canvas {
    let mut c = Canvas::new(WIDTH, HEIGHT);
    // Rounded at the top, where the face hides it until the head turns.
    let neck = Path::new()
        .move_to(MID, 430.0)
        .smooth_through(&[
            (404.0, 436.0),
            (416.0, 480.0),
            (420.0, 560.0),
            (432.0, 606.0),
            (452.0, 628.0),
        ])
        .line_to(316.0, 628.0)
        .smooth_through(&[
            (336.0, 606.0),
            (348.0, 560.0),
            (352.0, 480.0),
            (364.0, 436.0),
            (MID, 430.0),
        ])
        .close();
    c.fill(&neck, Paint::vertical(480.0, SKIN, 620.0, SKIN_TOP), Mode::Over);
    // The chin's shadow.
    let shadow = Path::new()
        .move_to(330.0, 470.0)
        .line_to(438.0, 470.0)
        .line_to(438.0, 548.0)
        .smooth_through(&[(410.0, 572.0), (MID, 580.0), (358.0, 572.0), (330.0, 548.0)])
        .close();
    c.fill(&shadow, SKIN_SHADE, Mode::Multiply);
    let sides = Path::new()
        .move_to(348.0, 520.0)
        .smooth_through(&[(344.0, 580.0), (330.0, 614.0)]);
    c.stroke(&sides, Stroke::pen(3.0), SKIN_LINE, Mode::Over);
    c.stroke(&sides.mirrored(MID), Stroke::pen(3.0), SKIN_LINE, Mode::Over);
    c
}

fn paint_uniform() -> Canvas {
    let mut c = Canvas::new(WIDTH, HEIGHT);
    let body = Path::new()
        .move_to(MID, 596.0)
        .smooth_through(&[
            (430.0, 602.0),
            (520.0, 628.0),
            (586.0, 662.0),
            (618.0, 720.0),
            (630.0, 840.0),
            (640.0, 1030.0),
        ])
        .line_to(128.0, 1030.0)
        .smooth_through(&[
            (138.0, 840.0),
            (150.0, 720.0),
            (182.0, 662.0),
            (248.0, 628.0),
            (338.0, 602.0),
            (MID, 596.0),
        ])
        .close();
    c.fill(
        &body,
        Paint::vertical(600.0, SHIRT, 1024.0, Color::hex(0xeceaf8)),
        Mode::Over,
    );
    // Folds and the shadow of the arms.
    for side in [1.0f32, -1.0] {
        let x = |v: f32| MID + (v - MID) * side;
        let arm_shadow = Path::new()
            .move_to(x(640.0), 700.0)
            .smooth_through(&[
                (x(596.0), 760.0),
                (x(580.0), 880.0),
                (x(592.0), 1030.0),
                (x(660.0), 1030.0),
            ])
            .close();
        c.fill(&arm_shadow, SHIRT_SHADE, Mode::Multiply);
        let crease = Path::new()
            .move_to(x(566.0), 760.0)
            .smooth_through(&[(x(574.0), 860.0), (x(570.0), 980.0)]);
        c.stroke(&crease, Stroke::pen(2.4), LINE.alpha(0.7), Mode::Atop);
        let fold = Path::new()
            .move_to(x(470.0), 880.0)
            .smooth_through(&[(x(486.0), 950.0), (x(482.0), 1010.0)]);
        c.stroke(&fold, Stroke::pen(2.0), SHIRT_SHADE, Mode::Atop);
        // Where the arm meets the body.
        let arm = Path::new().move_to(x(590.0), 700.0).smooth_through(&[
            (x(560.0), 780.0),
            (x(548.0), 900.0),
            (x(546.0), 1030.0),
        ]);
        c.stroke(&arm, Stroke::pen(3.0), LINE.alpha(0.8), Mode::Atop);
        let under_arm = Path::new()
            .move_to(x(590.0), 700.0)
            .smooth_through(&[
                (x(560.0), 780.0),
                (x(548.0), 900.0),
                (x(546.0), 1030.0),
                (x(520.0), 1030.0),
                (x(524.0), 880.0),
                (x(560.0), 740.0),
            ])
            .close();
        c.fill(&under_arm, SHIRT_SHADE.alpha(0.7), Mode::Multiply);
    }
    // Shade under the collar.
    let under_collar = Path::new()
        .move_to(160.0, 690.0)
        .smooth_through(&[
            (260.0, 760.0),
            (MID, 830.0),
            (508.0, 760.0),
            (608.0, 690.0),
            (608.0, 740.0),
            (MID, 870.0),
            (160.0, 740.0),
        ])
        .close();
    c.soft(&under_collar, 8.0, SHIRT_SHADE, Mode::Multiply);
    // The dickey under the collar's V.
    let dickey = Path::polygon(&[(340.0, 606.0), (428.0, 606.0), (MID, 700.0)]);
    c.fill(&dickey, Color::hex(0xf2f1fb), Mode::Atop);
    // Sailor collar: two navy flaps meeting in a V, white stripes.
    for side in [1.0f32, -1.0] {
        let x = |v: f32| MID + (v - MID) * side;
        let flap = Path::new()
            .move_to(x(424.0), 600.0)
            .smooth_through(&[(x(500.0), 616.0), (x(572.0), 650.0), (x(600.0), 690.0)])
            .smooth_through(&[(x(500.0), 750.0), (x(MID), 812.0)])
            .smooth_through(&[(x(404.0), 740.0), (x(424.0), 600.0)])
            .close();
        c.fill(&flap, Paint::vertical(600.0, NAVY_LIGHT, 812.0, NAVY), Mode::Atop);
        for inset in [14.0f32, 22.0] {
            let stripe = Path::new()
                .move_to(x(596.0 - inset), 688.0 + inset * 0.4)
                .smooth_through(&[
                    (x(500.0 - inset * 0.3), 744.0 + inset * 0.2),
                    (x(MID + inset * 0.4), 806.0 - inset * 1.2),
                ]);
            c.stroke(&stripe, Stroke::even(3.2), WHITE, Mode::Atop);
        }
        c.stroke(&flap, Stroke::even(2.6), LINE, Mode::Atop);
    }
    c.stroke(&body, Stroke::even(3.4), LINE, Mode::Over);
    c
}

fn paint_scarf() -> Canvas {
    let mut c = Canvas::new(WIDTH, HEIGHT);
    let knot_y = 800.0;
    for side in [1.0f32, -1.0] {
        let x = |v: f32| MID + (v - MID) * side;
        let tail = Path::new()
            .move_to(x(MID + 6.0), knot_y)
            .smooth_through(&[(x(412.0), 860.0), (x(424.0), 924.0)])
            .line_to(x(404.0), 912.0)
            .line_to(x(392.0), 930.0)
            .smooth_through(&[(x(388.0), 870.0), (x(MID - 4.0), knot_y + 6.0)])
            .close();
        c.fill(&tail, Paint::vertical(800.0, RED, 930.0, RED_DARK), Mode::Over);
        c.stroke(&tail, Stroke::even(2.4), LINE, Mode::Over);
        let wing = Path::new()
            .move_to(x(MID), knot_y - 4.0)
            .smooth_through(&[
                (x(420.0), 768.0),
                (x(452.0), 774.0),
                (x(456.0), 808.0),
                (x(424.0), 826.0),
                (x(MID), knot_y + 8.0),
            ])
            .close();
        c.fill(
            &wing,
            Paint::vertical(768.0, Color::hex(0xf35b6e), 826.0, RED),
            Mode::Over,
        );
        let fold = Path::new()
            .move_to(x(400.0), 796.0)
            .smooth_through(&[(x(428.0), 790.0), (x(446.0), 794.0)]);
        c.stroke(&fold, Stroke::pen(2.2), RED_DARK, Mode::Atop);
        c.stroke(&wing, Stroke::even(2.4), LINE, Mode::Over);
    }
    let knot = Path::ellipse(MID, knot_y + 2.0, 14.0, 16.0);
    c.fill(
        &knot,
        Paint::vertical(786.0, Color::hex(0xf35b6e), 818.0, RED_DARK),
        Mode::Over,
    );
    c.stroke(&knot, Stroke::even(2.4), LINE, Mode::Over);
    c
}

fn back_hair_outline() -> Path {
    let right = [
        (MID, 108.0),
        (470.0, 124.0),
        (534.0, 186.0),
        (560.0, 280.0),
        (566.0, 420.0),
        (584.0, 560.0),
        (606.0, 700.0),
        (618.0, 820.0),
        (604.0, 900.0),
    ];
    let mut p = Path::new().move_to(MID, 108.0).smooth_through(&right[1..]);
    // Jagged tips across the bottom.
    for &(tx, ty, vx, vy) in &[
        (590.0, 928.0, 572.0, 880.0),
        (552.0, 948.0, 532.0, 890.0),
        (508.0, 922.0, 490.0, 880.0),
    ] {
        p = p
            .quad_to(tx + 6.0, ty - 30.0, tx, ty)
            .quad_to(vx + 4.0, vy + 20.0, vx, vy);
    }
    p = p.line_to(mirror_x(490.0), 880.0);
    for &(tx, ty, vx, vy) in &[
        (508.0, 922.0, 532.0, 890.0),
        (552.0, 948.0, 572.0, 880.0),
        (590.0, 928.0, 604.0, 900.0),
    ] {
        let (tx, vx) = (mirror_x(tx), mirror_x(vx));
        p = p
            .quad_to(tx + 4.0, ty - 24.0, tx, ty)
            .quad_to(vx - 4.0, vy + 20.0, vx, vy);
    }
    let left: Vec<(f32, f32)> = right[..right.len() - 1]
        .iter()
        .rev()
        .map(|&(x, y)| (mirror_x(x), y))
        .collect();
    p.smooth_through(&left).close()
}

fn paint_back_hair() -> Canvas {
    let mut c = Canvas::new(WIDTH, HEIGHT);
    let hair = back_hair_outline();
    c.fill(&hair, Paint::vertical(110.0, HAIR, 940.0, HAIR_DARK), Mode::Over);
    // The inside of the hair, seen behind the neck.
    let inside = Path::new()
        .move_to(286.0, 420.0)
        .line_to(482.0, 420.0)
        .smooth_through(&[
            (500.0, 620.0),
            (520.0, 860.0),
            (MID, 900.0),
            (248.0, 860.0),
            (268.0, 620.0),
            (286.0, 420.0),
        ])
        .close();
    c.fill(&inside, Color::hex(0x2a1f6e), Mode::Atop);
    // Flowing strands.
    for side in [1.0f32, -1.0] {
        let x = |v: f32| MID + (v - MID) * side;
        for (a, b, d) in [
            (540.0, 300.0, 590.0),
            (552.0, 420.0, 600.0),
            (566.0, 520.0, 612.0),
        ] {
            let strand = Path::new()
                .move_to(x(a), b)
                .smooth_through(&[(x(a + 20.0), b + 180.0), (x(d), b + 360.0)]);
            c.stroke(&strand, Stroke::pen(3.0), HAIR_SHINE.alpha(0.55), Mode::Screen);
        }
        let dark = Path::new()
            .move_to(x(520.0), 400.0)
            .smooth_through(&[(x(548.0), 620.0), (x(560.0), 860.0)]);
        c.stroke(&dark, Stroke::pen(4.0), HAIR_LINE.alpha(0.5), Mode::Atop);
    }
    c.stroke(&hair, Stroke::even(3.2), HAIR_LINE, Mode::Over);
    c
}

/// A side lock hanging beside the right cheek (mirrored for the left).
fn side_lock(side: f32) -> Path {
    let x = |v: f32| MID + (v - MID) * side;
    Path::new()
        .move_to(x(470.0), 226.0)
        .smooth_through(&[
            (x(514.0), 250.0),
            (x(534.0), 340.0),
            (x(540.0), 470.0),
            (x(534.0), 600.0),
            (x(520.0), 700.0),
        ])
        .quad_to(x(514.0), 730.0, x(506.0), 748.0)
        .quad_to(x(504.0), 700.0, x(496.0), 672.0)
        .quad_to(x(494.0), 700.0, x(482.0), 716.0)
        .smooth_through(&[
            (x(492.0), 600.0),
            (x(500.0), 470.0),
            (x(498.0), 360.0),
            (x(486.0), 280.0),
            (x(470.0), 226.0),
        ])
        .close()
}

fn paint_side_hair(side: f32) -> Canvas {
    let mut c = Canvas::new(WIDTH, HEIGHT);
    let x = |v: f32| MID + (v - MID) * side;
    let lock = side_lock(side);
    c.fill(&lock, Paint::vertical(230.0, HAIR_LIGHT, 740.0, HAIR), Mode::Over);
    // Shade on the side facing the cheek.
    let shade = Path::new()
        .move_to(x(470.0), 226.0)
        .smooth_through(&[
            (x(488.0), 300.0),
            (x(500.0), 420.0),
            (x(496.0), 560.0),
            (x(486.0), 720.0),
            (x(460.0), 720.0),
            (x(460.0), 226.0),
        ])
        .close();
    c.fill(&shade, HAIR_SHADE, Mode::Multiply);
    let strand = Path::new()
        .move_to(x(512.0), 300.0)
        .smooth_through(&[(x(522.0), 460.0), (x(512.0), 640.0)]);
    c.stroke(&strand, Stroke::pen(3.4), HAIR_SHINE.alpha(0.8), Mode::Screen);
    let line = Path::new()
        .move_to(x(500.0), 330.0)
        .smooth_through(&[(x(508.0), 480.0), (x(504.0), 620.0)]);
    c.stroke(&line, Stroke::pen(2.2), HAIR_LINE.alpha(0.6), Mode::Atop);
    c.stroke(&lock, Stroke::even(3.0), HAIR_LINE, Mode::Over);
    c
}

/// The bangs' clumps from right to left: (tip, valley after it).
const CLUMPS: [((f32, f32), (f32, f32)); 6] = [
    ((500.0, 384.0), (478.0, 332.0)),
    ((452.0, 374.0), (432.0, 328.0)),
    ((410.0, 398.0), (392.0, 332.0)),
    ((366.0, 368.0), (350.0, 328.0)),
    ((322.0, 394.0), (304.0, 334.0)),
    ((282.0, 372.0), (262.0, 336.0)),
];

fn front_hair_outline() -> Path {
    let mut p = Path::new().move_to(250.0, 336.0).smooth_through(&[
        (252.0, 240.0),
        (290.0, 164.0),
        (MID, 128.0),
        (478.0, 164.0),
        (516.0, 240.0),
        (520.0, 330.0),
    ]);
    let mut from = (520.0f32, 330.0f32);
    for (tip, valley) in CLUMPS {
        // Each clump bulges down to a point that leans a little inward,
        // then its other edge runs back up into the next notch.
        p = p
            .cubic_to(
                from.0 + 2.0,
                from.1 + 26.0,
                tip.0 + 10.0,
                tip.1 - 20.0,
                tip.0,
                tip.1,
            )
            .cubic_to(
                tip.0 - 4.0,
                tip.1 - 22.0,
                valley.0 + 6.0,
                valley.1 + 20.0,
                valley.0,
                valley.1,
            );
        from = valley;
    }
    p.cubic_to(from.0 - 4.0, from.1 + 10.0, 250.0, 346.0, 250.0, 336.0)
        .close()
}

/// The shadow the bangs cast: their silhouette, lowered.
fn bang_shadow() -> Path {
    let mut p = Path::new()
        .move_to(240.0, 180.0)
        .line_to(528.0, 180.0)
        .line_to(528.0, 344.0);
    let mut from = (528.0f32, 344.0f32);
    for (tip, valley) in CLUMPS {
        let (tip, valley) = ((tip.0 - 4.0, tip.1 + 14.0), (valley.0 - 4.0, valley.1 + 16.0));
        p = p
            .cubic_to(from.0, from.1 + 24.0, tip.0 + 10.0, tip.1 - 20.0, tip.0, tip.1)
            .cubic_to(
                tip.0 - 4.0,
                tip.1 - 20.0,
                valley.0 + 6.0,
                valley.1 + 18.0,
                valley.0,
                valley.1,
            );
        from = valley;
    }
    p.line_to(240.0, 350.0).close()
}

fn paint_front_hair() -> Canvas {
    let mut c = Canvas::new(WIDTH, HEIGHT);
    let bangs = front_hair_outline();
    c.fill(
        &bangs,
        Paint::vertical(130.0, HAIR, 390.0, HAIR_LIGHT),
        Mode::Over,
    );
    // Shadow between the clumps and near the roots.
    let roots = Path::ellipse(MID, 150.0, 150.0, 50.0);
    c.soft(&roots, 14.0, HAIR_SHADE, Mode::Multiply);
    for (_, valley) in CLUMPS {
        let parting = Path::new()
            .move_to(valley.0 + 10.0, 196.0 + (valley.0 - MID).abs() * 0.12)
            .quad_to(valley.0 + 8.0, (valley.1 + 200.0) * 0.5, valley.0, valley.1 - 2.0);
        c.stroke(&parting, Stroke::pen(3.0), HAIR_LINE.alpha(0.5), Mode::Atop);
    }
    // The angel ring: a band following the crown, its lower edge broken
    // into the strands.
    let ring_y = |x: f32| 206.0 + ((x - MID) / 150.0).powi(2) * 40.0;
    let mut ring = Path::new().move_to(272.0, ring_y(272.0) - 4.0);
    let mut x = 272.0;
    while x < 496.0 {
        x += 8.0;
        ring = ring.line_to(
            x,
            ring_y(x) - 7.0 - 3.0 * ((x - 272.0) / 224.0 * std::f32::consts::PI).sin(),
        );
    }
    let mut k = 0;
    while x > 272.0 {
        x -= 8.0;
        let spike = if k % 3 == 1 { 12.0 } else { 3.0 };
        ring = ring.line_to(x, ring_y(x) + spike);
        k += 1;
    }
    c.fill(&ring.close(), HAIR_SHINE.alpha(0.85), Mode::Screen);
    c.stroke(&bangs, Stroke::even(3.0), HAIR_LINE, Mode::Over);
    c
}

fn paint_ahoge() -> Canvas {
    let mut c = Canvas::new(WIDTH, HEIGHT);
    let strand =
        Path::new()
            .move_to(378.0, 150.0)
            .smooth_through(&[(388.0, 96.0), (420.0, 68.0), (452.0, 76.0)]);
    c.stroke(
        &strand,
        Stroke {
            width: 17.0,
            taper_in: 0.0,
            taper_out: 0.9,
        },
        HAIR_LINE,
        Mode::Over,
    );
    c.stroke(
        &strand,
        Stroke {
            width: 11.0,
            taper_in: 0.0,
            taper_out: 0.9,
        },
        HAIR,
        Mode::Over,
    );
    let shine = Path::new()
        .move_to(386.0, 118.0)
        .smooth_through(&[(398.0, 90.0), (418.0, 78.0)]);
    c.stroke(&shine, Stroke::pen(3.0), HAIR_SHINE, Mode::Screen);
    c
}

fn paint_hair_ribbon() -> Canvas {
    let mut c = Canvas::new(WIDTH, HEIGHT);
    let (cx, cy) = (512.0, 214.0);
    for side in [1.0f32, -1.0] {
        let loop_ = Path::new()
            .move_to(cx, cy)
            .smooth_through(&[
                (cx + 26.0 * side, cy - 30.0),
                (cx + 48.0 * side, cy - 20.0),
                (cx + 42.0 * side, cy + 14.0),
                (cx, cy + 4.0),
            ])
            .close();
        c.fill(
            &loop_,
            Paint::vertical(cy - 30.0, Color::hex(0xf35b6e), cy + 14.0, RED),
            Mode::Over,
        );
        c.stroke(&loop_, Stroke::even(2.4), LINE, Mode::Over);
        let tail = Path::new()
            .move_to(cx, cy + 4.0)
            .smooth_through(&[(cx + 14.0 * side, cy + 40.0), (cx + 22.0 * side, cy + 70.0)])
            .line_to(cx + 8.0 * side, cy + 62.0)
            .smooth_through(&[(cx + 2.0 * side, cy + 30.0), (cx, cy + 4.0)])
            .close();
        c.fill(&tail, Paint::vertical(cy, RED, cy + 70.0, RED_DARK), Mode::Over);
        c.stroke(&tail, Stroke::even(2.2), LINE, Mode::Over);
    }
    let knot = Path::ellipse(cx, cy - 2.0, 10.0, 12.0);
    c.fill(&knot, RED_DARK, Mode::Over);
    c.stroke(&knot, Stroke::even(2.2), LINE, Mode::Over);
    c
}

/// The right eye's shapes (mirrored for the left): the eye white outline
/// and its centre.
fn eye_white_outline(side: f32) -> Path {
    let x = |v: f32| MID + (v - MID) * side;
    Path::new()
        .move_to(x(418.0), 400.0)
        .cubic_to(x(424.0), 366.0, x(466.0), 356.0, x(492.0), 386.0)
        .cubic_to(x(484.0), 418.0, x(440.0), 430.0, x(418.0), 400.0)
        .close()
}

fn paint_eye_white(side: f32) -> Canvas {
    let mut c = Canvas::new(WIDTH, HEIGHT);
    let x = |v: f32| MID + (v - MID) * side;
    let eye = eye_white_outline(side);
    c.fill(&eye, WHITE, Mode::Over);
    // The upper lid's shadow.
    let lid = Path::new()
        .move_to(x(410.0), 350.0)
        .line_to(x(500.0), 350.0)
        .line_to(x(500.0), 386.0)
        .cubic_to(x(470.0), 372.0, x(436.0), 376.0, x(410.0), 398.0)
        .close();
    c.fill(&lid, Color::hex(0xcfc8f2), Mode::Multiply);
    c
}

fn paint_iris(side: f32) -> Canvas {
    let mut c = Canvas::new(WIDTH, HEIGHT);
    let x = |v: f32| MID + (v - MID) * side;
    let (cx, cy) = (x(452.0), 396.0);
    let iris = Path::ellipse(cx, cy, 25.0, 32.0);
    c.fill(
        &iris,
        Paint::Linear {
            from: Vec2::new(0.0, cy - 32.0),
            to: Vec2::new(0.0, cy + 32.0),
            stops: vec![(0.0, IRIS_TOP), (0.5, IRIS_MID), (1.0, IRIS_LOW)],
        },
        Mode::Over,
    );
    // Shadow of the lashes across the top of the iris.
    c.fill(
        &Path::ellipse(cx, cy - 26.0, 30.0, 16.0),
        Color::hex(0x8a5a70),
        Mode::Multiply,
    );
    c.fill(&Path::ellipse(cx, cy + 2.0, 10.0, 15.0), PUPIL, Mode::Over);
    // A glow in the lower iris.
    c.soft(
        &Path::ellipse(cx, cy + 18.0, 15.0, 8.0),
        4.0,
        Color::hex(0xfff2a8).alpha(0.8),
        Mode::Screen,
    );
    c.stroke(
        &iris,
        Stroke::even(2.4),
        Color::hex(0x4a1606).alpha(0.9),
        Mode::Over,
    );
    // Catch lights.
    c.fill(
        &Path::ellipse(cx - 9.0 * side, cy - 13.0, 8.0, 10.0),
        WHITE,
        Mode::Over,
    );
    c.fill(
        &Path::ellipse(cx + 10.0 * side, cy + 14.0, 3.6, 3.6),
        WHITE,
        Mode::Over,
    );
    c.fill(
        &Path::ellipse(cx + 6.0 * side, cy - 16.0, 2.2, 2.2),
        WHITE.alpha(0.9),
        Mode::Over,
    );
    c
}

fn paint_lash(side: f32) -> Canvas {
    let mut c = Canvas::new(WIDTH, HEIGHT);
    let x = |v: f32| MID + (v - MID) * side;
    // Upper lash: heavy at the outer corner, thinning toward the nose.
    let upper =
        Path::new()
            .move_to(x(496.0), 388.0)
            .cubic_to(x(468.0), 352.0, x(426.0), 360.0, x(414.0), 398.0);
    c.stroke(
        &upper,
        Stroke {
            width: 8.0,
            taper_in: 0.05,
            taper_out: 0.55,
        },
        LASH,
        Mode::Over,
    );
    // Flicks at the outer corner.
    for (a, b, cc, d) in [(492.0, 382.0, 506.0, 368.0), (496.0, 390.0, 512.0, 386.0)] {
        let flick =
            Path::new()
                .move_to(x(a), b)
                .quad_to(x((a + cc) * 0.5 + 2.0), (b + d) * 0.5 - 2.0, x(cc), d);
        c.stroke(
            &flick,
            Stroke {
                width: 4.5,
                taper_in: 0.0,
                taper_out: 0.9,
            },
            LASH,
            Mode::Over,
        );
    }
    // Lower lash, just at the outer corner.
    let lower = Path::new()
        .move_to(x(488.0), 400.0)
        .quad_to(x(480.0), 414.0, x(462.0), 420.0);
    c.stroke(
        &lower,
        Stroke {
            width: 2.6,
            taper_in: 0.1,
            taper_out: 0.8,
        },
        LASH,
        Mode::Over,
    );
    // Double eyelid.
    let lid = Path::new()
        .move_to(x(484.0), 366.0)
        .quad_to(x(456.0), 350.0, x(428.0), 360.0);
    c.stroke(&lid, Stroke::pen(2.0), LASH.alpha(0.55), Mode::Over);
    c
}

fn paint_brow(side: f32) -> Canvas {
    let mut c = Canvas::new(WIDTH, HEIGHT);
    let x = |v: f32| MID + (v - MID) * side;
    let brow = Path::new()
        .move_to(x(412.0), 346.0)
        .quad_to(x(452.0), 324.0, x(494.0), 338.0);
    c.stroke(
        &brow,
        Stroke {
            width: 7.0,
            taper_in: 0.12,
            taper_out: 0.7,
        },
        BROW,
        Mode::Over,
    );
    c
}

fn paint_nose() -> Canvas {
    let mut c = Canvas::new(WIDTH, HEIGHT);
    let nose = Path::new()
        .move_to(388.0, 446.0)
        .quad_to(390.0, 456.0, 382.0, 462.0);
    c.stroke(&nose, Stroke::pen(2.6), SKIN_LINE, Mode::Over);
    c.soft(
        &Path::ellipse(378.0, 452.0, 4.0, 6.0),
        2.0,
        SKIN_SHADE,
        Mode::Over,
    );
    c
}

fn paint_mouth() -> Canvas {
    let mut c = Canvas::new(WIDTH, HEIGHT);
    let line = Path::new()
        .move_to(364.0, 492.0)
        .quad_to(MID, 502.0, 404.0, 490.0);
    c.stroke(
        &line,
        Stroke {
            width: 3.2,
            taper_in: 0.3,
            taper_out: 0.3,
        },
        Color::hex(0x8a3446),
        Mode::Over,
    );
    let lip = Path::new()
        .move_to(378.0, 507.0)
        .quad_to(MID, 509.0, 390.0, 507.0);
    c.stroke(&lip, Stroke::pen(2.0), SKIN_LINE.alpha(0.6), Mode::Over);
    c
}

fn paint_mouth_open() -> Canvas {
    let mut c = Canvas::new(WIDTH, HEIGHT);
    let inside = Path::new()
        .move_to(364.0, 493.0)
        .quad_to(MID, 500.0, 404.0, 491.0)
        .quad_to(402.0, 520.0, MID, 528.0)
        .quad_to(366.0, 520.0, 364.0, 493.0)
        .close();
    c.fill(&inside, Color::hex(0x8e2436), Mode::Over);
    c.fill(
        &Path::ellipse(388.0, 522.0, 12.0, 7.0),
        Color::hex(0xf28a98),
        Mode::Atop,
    );
    c.fill(
        &Path::polygon(&[(360.0, 480.0), (410.0, 480.0), (410.0, 498.0), (360.0, 500.0)]),
        WHITE,
        Mode::Atop,
    );
    c.stroke(&inside, Stroke::even(2.2), Color::hex(0x5a1424), Mode::Over);
    c
}

fn paint_cheek() -> Canvas {
    let mut c = Canvas::new(WIDTH, HEIGHT);
    for side in [1.0f32, -1.0] {
        let x = |v: f32| MID + (v - MID) * side;
        c.soft(
            &Path::ellipse(x(464.0), 452.0, 32.0, 13.0),
            6.0,
            Color::hex(0xff7e96).alpha(0.75),
            Mode::Over,
        );
        for k in 0..3 {
            let x0 = x(452.0 + k as f32 * 11.0);
            let hatch = Path::new().move_to(x0 + 4.0, 444.0).line_to(x0 - 3.0, 459.0);
            c.stroke(
                &hatch,
                Stroke::pen(2.2),
                Color::hex(0xf2506e).alpha(0.8),
                Mode::Over,
            );
        }
    }
    c
}

/// Paint Aether-chan into a new document. Returns the document and its
/// parts.
pub fn paint() -> (Document, Parts) {
    let mut doc = Document::empty(WIDTH, HEIGHT, "Aether-chan");
    let mut add = |name: &str, canvas: Canvas, clipping: bool, opacity: f32| -> LayerId {
        let id = doc.next_layer_id();
        let mut layer = Layer::raster(id, name, WIDTH, HEIGHT);
        if let Some(pm) = layer.pixmap_mut() {
            *pm = canvas.to_pixmap();
        }
        layer.clipping = clipping;
        layer.opacity = opacity;
        let _ = doc.layers.push_top(layer);
        id
    };
    let back_hair = add("Back hair", paint_back_hair(), false, 1.0);
    let neck = add("Neck", paint_neck(), false, 1.0);
    let uniform = add("Uniform", paint_uniform(), false, 1.0);
    let scarf = add("Uniform scarf", paint_scarf(), false, 1.0);
    let face = add("Face", paint_face(), false, 1.0);
    let cheek = add("Cheek", paint_cheek(), false, 1.0);
    let nose = add("Nose", paint_nose(), false, 1.0);
    let mouth_open = add("Mouth open", paint_mouth_open(), false, 1.0);
    let mouth = add("Mouth", paint_mouth(), false, 1.0);
    // Left and right as the character sees them: her left eye is on the
    // viewer's right.
    let mut eye_white = [LayerId(0); 2];
    let mut iris = [LayerId(0); 2];
    let mut lash = [LayerId(0); 2];
    for (i, (side, name)) in [(1.0f32, "L"), (-1.0, "R")].into_iter().enumerate() {
        eye_white[i] = add(&format!("Eye white {name}"), paint_eye_white(side), false, 1.0);
        iris[i] = add(&format!("Iris {name}"), paint_iris(side), true, 1.0);
        lash[i] = add(&format!("Eyelash {name}"), paint_lash(side), false, 1.0);
    }
    let side_hair = [
        add("Side hair L", paint_side_hair(1.0), false, 1.0),
        add("Side hair R", paint_side_hair(-1.0), false, 1.0),
    ];
    let ahoge = add("Ahoge", paint_ahoge(), false, 1.0);
    let front_hair = add("Front hair", paint_front_hair(), false, 1.0);
    // Brows show through the bangs, as anime draws them.
    let brow = [
        add("Brow L", paint_brow(1.0), false, 0.9),
        add("Brow R", paint_brow(-1.0), false, 0.9),
    ];
    let hair_ribbon = add("Hair ribbon", paint_hair_ribbon(), false, 1.0);
    doc.active_layer = face;
    (
        doc,
        Parts {
            back_hair,
            uniform,
            neck,
            scarf,
            face,
            cheek,
            nose,
            mouth_open,
            mouth,
            eye_white,
            iris,
            lash,
            side_hair,
            front_hair,
            ahoge,
            brow,
            hair_ribbon,
        },
    )
}
