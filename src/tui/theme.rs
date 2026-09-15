//! Theme definitions and color/style helpers shared by the TUI views.

use std::str::FromStr;

use tuirealm::ratatui::style::{Color, Modifier, Style};
use tuirealm::ratatui::widgets::BorderType;

use crate::config::Config;
use crate::core::{DayKind, Minutes};

#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    pub positive: Color,
    pub negative: Color,
    pub warning: Color,
    pub accent: Color,
    pub muted: Color,
    pub text: Color,
    pub bg_selected: Color,
    pub chip_vacation: Color,
    pub chip_flex: Color,
    pub chip_holiday: Color,
    pub chip_sick: Color,
    pub chip_absence: Color,
    pub projects: [Color; 10],
    pub border: BorderType,
}

fn rgb(hex: u32) -> Color {
    Color::Rgb(
        ((hex >> 16) & 0xff) as u8,
        ((hex >> 8) & 0xff) as u8,
        (hex & 0xff) as u8,
    )
}

impl Theme {
    /// Catppuccin Mocha-like.
    pub fn dark() -> Theme {
        Theme {
            positive: rgb(0xa6e3a1),
            negative: rgb(0xf38ba8),
            warning: rgb(0xf9e2af),
            accent: rgb(0x89b4fa),
            muted: rgb(0x6c7086),
            text: rgb(0xcdd6f4),
            bg_selected: rgb(0x313244),
            chip_vacation: rgb(0x94e2d5),
            chip_flex: rgb(0xcba6f7),
            chip_holiday: rgb(0xfab387),
            chip_sick: rgb(0xf38ba8),
            chip_absence: rgb(0xf5c2e7),
            projects: [
                rgb(0x89b4fa),
                rgb(0xa6e3a1),
                rgb(0xf9e2af),
                rgb(0xcba6f7),
                rgb(0x94e2d5),
                rgb(0xfab387),
                rgb(0xf5c2e7),
                rgb(0x74c7ec),
                rgb(0xeba0ac),
                rgb(0xb4befe),
            ],
            border: BorderType::Rounded,
        }
    }

    /// Solarized-light-like.
    pub fn light() -> Theme {
        Theme {
            positive: rgb(0x859900),
            negative: rgb(0xdc322f),
            warning: rgb(0xb58900),
            accent: rgb(0x268bd2),
            muted: rgb(0x93a1a1),
            text: rgb(0x073642),
            bg_selected: rgb(0xeee8d5),
            chip_vacation: rgb(0x2aa198),
            chip_flex: rgb(0x6c71c4),
            chip_holiday: rgb(0xcb4b16),
            chip_sick: rgb(0xdc322f),
            chip_absence: rgb(0xd33682),
            projects: [
                rgb(0x268bd2),
                rgb(0x859900),
                rgb(0xb58900),
                rgb(0x6c71c4),
                rgb(0x2aa198),
                rgb(0xcb4b16),
                rgb(0xd33682),
                rgb(0x0f7fb3),
                rgb(0xa8332f),
                rgb(0x586e75),
            ],
            border: BorderType::Rounded,
        }
    }

    pub fn from_config(cfg: &Config) -> Theme {
        let mut t = if cfg.theme == "light" {
            Theme::light()
        } else {
            Theme::dark()
        };
        for (k, v) in &cfg.theme_overrides {
            let Some(c) = parse_color(v) else { continue };
            match k.as_str() {
                "positive" => t.positive = c,
                "negative" => t.negative = c,
                "warning" => t.warning = c,
                "accent" => t.accent = c,
                "muted" => t.muted = c,
                "text" => t.text = c,
                "bg_selected" => t.bg_selected = c,
                "chip_vacation" => t.chip_vacation = c,
                "chip_flex" => t.chip_flex = c,
                "chip_holiday" => t.chip_holiday = c,
                "chip_sick" => t.chip_sick = c,
                "chip_absence" => t.chip_absence = c,
                _ => {}
            }
        }
        if supports_truecolor() {
            t
        } else {
            t.downgrade_for_terminal()
        }
    }

    pub fn project_color(&self, idx: u8) -> Color {
        self.projects[(idx as usize) % self.projects.len()]
    }

    pub fn kind_color(&self, k: &DayKind) -> Color {
        match k {
            DayKind::Work => self.text,
            DayKind::Vacation => self.chip_vacation,
            DayKind::Flex => self.chip_flex,
            DayKind::Holiday => self.chip_holiday,
            DayKind::Sick => self.chip_sick,
            DayKind::Absence { .. } => self.chip_absence,
        }
    }

    pub fn minutes_style(&self, m: Minutes) -> Style {
        let fg = if m.0 > 0 {
            self.positive
        } else if m.0 < 0 {
            self.negative
        } else {
            self.muted
        };
        Style::default().fg(fg).add_modifier(Modifier::BOLD)
    }

    /// Map every Rgb color to the nearest xterm-256 index.
    pub fn downgrade_for_terminal(mut self) -> Theme {
        let f = |c: Color| match c {
            Color::Rgb(r, g, b) => Color::Indexed(nearest_256(r, g, b)),
            other => other,
        };
        self.positive = f(self.positive);
        self.negative = f(self.negative);
        self.warning = f(self.warning);
        self.accent = f(self.accent);
        self.muted = f(self.muted);
        self.text = f(self.text);
        self.bg_selected = f(self.bg_selected);
        self.chip_vacation = f(self.chip_vacation);
        self.chip_flex = f(self.chip_flex);
        self.chip_holiday = f(self.chip_holiday);
        self.chip_sick = f(self.chip_sick);
        self.chip_absence = f(self.chip_absence);
        for p in self.projects.iter_mut() {
            *p = f(*p);
        }
        self
    }
}

fn nearest_256(r: u8, g: u8, b: u8) -> u8 {
    let q = |v: u8| -> u8 {
        if v < 48 {
            0
        } else if v < 115 {
            1
        } else {
            ((v as u16 - 35) / 40) as u8
        }
    };
    16 + 36 * q(r) + 6 * q(g) + q(b)
}

pub fn parse_color(s: &str) -> Option<Color> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix('#')
        && hex.len() == 6
        && let Ok(v) = u32::from_str_radix(hex, 16)
    {
        return Some(rgb(v));
    }
    Color::from_str(s).ok()
}

pub fn supports_truecolor() -> bool {
    matches!(
        std::env::var("COLORTERM").as_deref(),
        Ok("truecolor") | Ok("24bit")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, DEFAULT_TOML};
    use tuirealm::ratatui::style::Color;

    #[test]
    fn parses_hex_and_named() {
        assert_eq!(parse_color("#a6e3a1"), Some(Color::Rgb(0xa6, 0xe3, 0xa1)));
        assert_eq!(parse_color("red"), Some(Color::Red));
        assert_eq!(parse_color("nope"), None);
    }

    #[test]
    fn overrides_apply() {
        // SAFETY: no other threads read/write COLORTERM concurrently in this test binary's
        // relevant window; this pins the test to the truecolor path regardless of the
        // ambient environment the test runner happens to have.
        unsafe {
            std::env::set_var("COLORTERM", "truecolor");
        }
        let toml = DEFAULT_TOML.replace("# positive = \"#a6e3a1\"", "positive = \"#123456\"");
        let cfg = Config::from_toml(&toml).unwrap();
        let t = Theme::from_config(&cfg);
        assert_eq!(t.positive, Color::Rgb(0x12, 0x34, 0x56));
        assert_ne!(t.negative, t.positive);
    }

    #[test]
    fn project_colors_cycle() {
        let t = Theme::dark();
        assert_eq!(t.project_color(0), t.projects[0]);
        assert_eq!(t.project_color(13), t.projects[3]);
    }

    #[test]
    fn minutes_style_sign() {
        let t = Theme::dark();
        assert_eq!(t.minutes_style(Minutes(5)).fg, Some(t.positive));
        assert_eq!(t.minutes_style(Minutes(-5)).fg, Some(t.negative));
        assert_eq!(t.minutes_style(Minutes(0)).fg, Some(t.muted));
    }

    #[test]
    fn downgrade_maps_rgb_to_indexed() {
        let t = Theme::dark().downgrade_for_terminal();
        assert!(matches!(t.positive, Color::Indexed(_)));
    }
}
