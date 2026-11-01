//! Color themes and holiday themes, like Kumo's: eight sets of dark greys tinted toward one
//! hue (each with its own accent), and six holidays that switch themselves on in their month
//! with an accent, a tint, decorations and an animation.

use egui::Color32;
use egui_phosphor::regular as icon;

use super::theme::{mix, Palette};

const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

pub struct ColorTheme {
    /// Saved in the settings.
    pub id: &'static str,
    pub name: &'static str,
    pub accent: Color32,
    pub palette: Palette,
}

const fn theme(id: &'static str, name: &'static str, accent: u32, p: [u32; 7]) -> ColorTheme {
    ColorTheme {
        id,
        name,
        accent: rgb(accent),
        palette: Palette {
            window_bg: rgb(p[0]),
            panel: rgb(p[1]),
            card: rgb(p[2]),
            hover: rgb(p[3]),
            selected: rgb(p[4]),
            text_dim: rgb(p[5]),
            text_faint: rgb(p[6]),
        },
    }
}

/// Utako's own graphite, the record label's off-white as the accent.
pub const SUMO: ColorTheme = theme(
    "sumo",
    "Utako",
    0xe9e6df,
    [0x0e0e10, 0x161619, 0x1e1e22, 0x27272c, 0x303036, 0xa4a3a9, 0x6c6b72],
);

/// Every color theme, in the order they are shown. The others are Kumo's, so the apps match.
pub const THEMES: [ColorTheme; 8] = [
    SUMO,
    theme(
        "midnight",
        "Midnight",
        0x5b8cff,
        [0x070a12, 0x111827, 0x172033, 0x1e2940, 0x26334d, 0x9aa4b8, 0x6b7488],
    ),
    theme(
        "forest",
        "Forest",
        0x34d399,
        [0x070b09, 0x111a15, 0x16211b, 0x1c2a22, 0x23342a, 0x9fb0a6, 0x6d7f74],
    ),
    theme(
        "sakura",
        "Sakura",
        0xf472b6,
        [0x0d0709, 0x1b1116, 0x22161c, 0x2a1c23, 0x33222b, 0xb39aa6, 0x7f6872],
    ),
    theme(
        "ember",
        "Ember",
        0xf59e0b,
        [0x0d0906, 0x1b1610, 0x221c14, 0x2a2219, 0x332a1f, 0xb3a691, 0x7f7461],
    ),
    theme(
        "ocean",
        "Ocean",
        0x22d3ee,
        [0x050c0e, 0x0f1c1f, 0x142428, 0x1a2d32, 0x21373d, 0x93aeb3, 0x647d82],
    ),
    theme(
        "dusk",
        "Dusk",
        0xa78bfa,
        [0x0a0810, 0x161220, 0x1c1728, 0x231d31, 0x2b243b, 0xa79fb8, 0x766e88],
    ),
    // True black: on OLED screens black pixels are off.
    theme(
        "black",
        "Black",
        0xe9e6df,
        [0x000000, 0x0b0b0c, 0x121214, 0x19191c, 0x212124, 0xa1a1a8, 0x6e6e75],
    ),
];

/// Accent swatches offered next to the color picker.
pub const ACCENTS: [(&str, u32); 9] = [
    ("Off-white (Utako)", 0xe9e6df),
    ("Violet", 0x7c6cf2),
    ("Blue", 0x5b8cff),
    ("Teal", 0x22c3a6),
    ("Rose", 0xf2557a),
    ("Amber", 0xf59e0b),
    ("Orchid", 0xe879f9),
    ("Sky", 0x38bdf8),
    ("Lime", 0xa3e635),
];

pub fn color_theme(id: &str) -> &'static ColorTheme {
    THEMES.iter().find(|t| t.id == id).unwrap_or(&THEMES[0])
}

/// What moves over the window while a holiday theme is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    Fireworks,
    Hearts,
    Petals,
    Fireflies,
    Bats,
    Snow,
}

pub struct Holiday {
    pub id: &'static str,
    pub name: &'static str,
    /// On by itself from this day to that one (month, day), both included.
    pub from: (u32, u32),
    pub to: (u32, u32),
    pub accent: Color32,
    /// Mixed a little into the window's colours.
    pub tint: Color32,
    pub effect: Effect,
    /// Shown on the Themes button while it's on.
    pub glyph: &'static str,
}

pub const HOLIDAYS: [Holiday; 6] = [
    Holiday {
        id: "newyear",
        name: "New Year",
        from: (1, 1),
        to: (1, 7),
        accent: rgb(0xf5c542),
        tint: rgb(0x1e3a8a),
        effect: Effect::Fireworks,
        glyph: icon::CONFETTI,
    },
    Holiday {
        id: "valentine",
        name: "Valentine's Day",
        from: (2, 1),
        to: (2, 29),
        accent: rgb(0xf2557a),
        tint: rgb(0x9f1239),
        effect: Effect::Hearts,
        glyph: icon::HEART,
    },
    Holiday {
        id: "blossom",
        name: "Cherry blossoms",
        from: (4, 1),
        to: (4, 30),
        accent: rgb(0xf9a8d4),
        tint: rgb(0xbe185d),
        effect: Effect::Petals,
        glyph: icon::FLOWER,
    },
    Holiday {
        id: "summer",
        name: "Summer festival",
        from: (8, 1),
        to: (8, 31),
        accent: rgb(0x2dd4bf),
        tint: rgb(0x155e75),
        effect: Effect::Fireflies,
        glyph: icon::SPARKLE,
    },
    Holiday {
        id: "halloween",
        name: "Halloween",
        from: (10, 1),
        to: (10, 31),
        accent: rgb(0xf97316),
        tint: rgb(0x6b21a8),
        effect: Effect::Bats,
        glyph: icon::GHOST,
    },
    Holiday {
        id: "christmas",
        name: "Christmas",
        from: (12, 1),
        to: (12, 31),
        accent: rgb(0xef4444),
        tint: rgb(0x166534),
        effect: Effect::Snow,
        glyph: icon::TREE_EVERGREEN,
    },
];

/// The holiday on: `setting` is "auto" (by the date), "off", or a holiday's id (always).
pub fn holiday(setting: &str, month: u32, day: u32) -> Option<&'static Holiday> {
    match setting {
        "off" => None,
        "auto" | "" => HOLIDAYS.iter().find(|h| (month, day) >= h.from && (month, day) <= h.to),
        id => HOLIDAYS.iter().find(|h| h.id == id),
    }
}

/// The colours to draw with: the theme's, with the holiday's tint mixed in.
pub fn palette(theme: &ColorTheme, holiday: Option<&Holiday>) -> Palette {
    let p = theme.palette;
    let Some(h) = holiday else { return p };
    let t = h.tint;
    Palette {
        window_bg: mix(p.window_bg, t, 0.12),
        panel: mix(p.panel, t, 0.08),
        card: mix(p.card, t, 0.07),
        hover: mix(p.hover, t, 0.07),
        selected: mix(p.selected, t, 0.06),
        ..p
    }
}

/// Today (year, month, day) where you are.
pub fn today() -> (i64, u32, u32) {
    let day = crate::clock::local_day(crate::model::now_unix(), crate::clock::utc_offset());
    crate::clock::civil_from_days(day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn holidays_by_the_date() {
        assert_eq!(holiday("auto", 10, 30).map(|h| h.id), Some("halloween"));
        assert_eq!(holiday("auto", 1, 7).map(|h| h.id), Some("newyear"));
        assert_eq!(holiday("auto", 1, 8).map(|h| h.id), None);
        assert_eq!(holiday("auto", 12, 25).map(|h| h.id), Some("christmas"));
        assert_eq!(holiday("off", 12, 25).map(|h| h.id), None);
        assert_eq!(holiday("blossom", 6, 1).map(|h| h.id), Some("blossom"));
        // Unknown themes fall back to Utako's own.
        assert_eq!(color_theme("nope").id, "sumo");
    }

    #[test]
    fn a_holiday_only_tints() {
        let plain = palette(&SUMO, None);
        assert_eq!(plain, SUMO.palette);
        let spooky = palette(&SUMO, holiday("halloween", 1, 1));
        assert_ne!(spooky.panel, plain.panel);
        assert_eq!(spooky.text_dim, plain.text_dim);
    }
}
