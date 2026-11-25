//! User settings: edited in the Escape menu and saved between runs.
//!
//! Stored as simple `key = value` lines in
//! `%APPDATA%\walkabout\settings.cfg` (Windows) or
//! `~/.config/walkabout/settings.cfg` elsewhere. Unknown keys and bad
//! values are ignored, so an old or hand-edited file never stops the game.

use std::path::PathBuf;

use glam::Vec3;

/// Bumped when a saved value's meaning changes (see Settings::parse).
const SETTINGS_VERSION: u32 = 2;

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    // Controls
    /// Multiplier on the base mouse sensitivity.
    pub mouse_sensitivity: f32,
    pub invert_y: bool,
    pub fov_degrees: f32,

    // Graphics
    pub vsync: bool,
    pub grass: bool,
    /// How far from the player grass blades are drawn, in meters.
    pub grass_distance: f32,
    /// Multiplier on blade density (1 = one blade per 10 cm grid cell).
    pub grass_density: f32,
    /// Parallax "texture extrusion" on bricks, cobbles, etc.
    pub surface_detail: bool,
    /// Where the distance fog becomes fully opaque, in meters.
    pub view_distance: f32,
    pub show_fps: bool,
    /// Sandbox: every ability unlocked, regardless of skill points.
    pub unlock_all: bool,
    /// Sandbox Ascension level (with unlock_all).
    pub sandbox_ascension: f32,

    // World
    pub brightness: f32,
    /// Sun height above the horizon and compass direction, in degrees.
    pub sun_elevation: f32,
    pub sun_azimuth: f32,
    pub wind: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            mouse_sensitivity: 1.0,
            invert_y: false,
            fov_degrees: 70.0,
            vsync: true,
            grass: true,
            grass_distance: 26.0,
            grass_density: 1.0,
            surface_detail: true,
            view_distance: 450.0,
            show_fps: true,
            unlock_all: false,
            sandbox_ascension: 0.0,
            brightness: 0.9,
            sun_elevation: 38.0,
            sun_azimuth: 226.0,
            wind: 0.35,
        }
    }
}

/// Allowed range for every numeric setting (also used by the menu sliders).
pub mod range {
    use std::ops::RangeInclusive;
    pub const MOUSE_SENSITIVITY: RangeInclusive<f32> = 0.2..=3.0;
    pub const FOV: RangeInclusive<f32> = 50.0..=110.0;
    pub const GRASS_DISTANCE: RangeInclusive<f32> = 8.0..=45.0;
    pub const GRASS_DENSITY: RangeInclusive<f32> = 0.4..=1.5;
    pub const VIEW_DISTANCE: RangeInclusive<f32> = 100.0..=900.0;
    pub const BRIGHTNESS: RangeInclusive<f32> = 0.4..=1.6;
    pub const SUN_ELEVATION: RangeInclusive<f32> = 5.0..=85.0;
    pub const SUN_AZIMUTH: RangeInclusive<f32> = 0.0..=360.0;
    pub const WIND: RangeInclusive<f32> = 0.0..=1.0;
    pub const SANDBOX_ASCENSION: RangeInclusive<f32> = 0.0..=60.0;
}

impl Settings {
    /// Direction the sunlight travels, from the elevation/azimuth angles.
    pub fn sun_direction(&self) -> Vec3 {
        let (el, az) = (self.sun_elevation.to_radians(), self.sun_azimuth.to_radians());
        -Vec3::new(el.cos() * az.cos(), el.sin(), el.cos() * az.sin())
    }

    /// Sunlight gets warmer and dimmer as the sun nears the horizon.
    pub fn sun_color(&self) -> Vec3 {
        let high = ((self.sun_elevation - 5.0) / 40.0).clamp(0.0, 1.0);
        let warm = Vec3::new(1.0, 0.62, 0.38) * 2.2;
        let noon = Vec3::new(1.0, 0.92, 0.8) * 2.7;
        warm.lerp(noon, high)
    }

    pub fn file_path() -> PathBuf {
        let base = std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .unwrap_or_else(|| PathBuf::from("."));
        base.join("walkabout").join("settings.cfg")
    }

    /// Loads saved settings, falling back to defaults for anything missing.
    pub fn load() -> Self {
        match std::fs::read_to_string(Self::file_path()) {
            Ok(text) => Self::parse(&text),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) {
        let path = Self::file_path();
        let result = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|_| std::fs::write(&path, self.serialize()));
        match result {
            Ok(()) => log::info!("saved settings to {}", path.display()),
            Err(e) => log::warn!("could not save settings to {}: {e}", path.display()),
        }
    }

    pub fn serialize(&self) -> String {
        format!(
            "# 3D Walkabout settings\n\
             version = {}\n\
             mouse_sensitivity = {}\n\
             invert_y = {}\n\
             fov_degrees = {}\n\
             vsync = {}\n\
             grass = {}\n\
             grass_distance = {}\n\
             grass_density = {}\n\
             surface_detail = {}\n\
             view_distance = {}\n\
             show_fps = {}\n\
             unlock_all = {}\n\
             sandbox_ascension = {}\n\
             brightness = {}\n\
             sun_elevation = {}\n\
             sun_azimuth = {}\n\
             wind = {}\n",
            SETTINGS_VERSION,
            self.mouse_sensitivity,
            self.invert_y,
            self.fov_degrees,
            self.vsync,
            self.grass,
            self.grass_distance,
            self.grass_density,
            self.surface_detail,
            self.view_distance,
            self.show_fps,
            self.unlock_all,
            self.sandbox_ascension,
            self.brightness,
            self.sun_elevation,
            self.sun_azimuth,
            self.wind,
        )
    }

    pub fn parse(text: &str) -> Self {
        let mut s = Self::default();
        let mut version = 1;
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let (key, value) = (key.trim(), value.trim());
            let num = |r: std::ops::RangeInclusive<f32>, target: &mut f32| {
                if let Ok(v) = value.parse::<f32>()
                    && v.is_finite()
                {
                    *target = v.clamp(*r.start(), *r.end());
                }
            };
            let flag = |target: &mut bool| {
                if let Ok(v) = value.parse::<bool>() {
                    *target = v;
                }
            };
            match key {
                "version" => version = value.parse().unwrap_or(1),
                "mouse_sensitivity" => num(range::MOUSE_SENSITIVITY, &mut s.mouse_sensitivity),
                "invert_y" => flag(&mut s.invert_y),
                "fov_degrees" => num(range::FOV, &mut s.fov_degrees),
                "vsync" => flag(&mut s.vsync),
                "grass" => flag(&mut s.grass),
                "grass_distance" => num(range::GRASS_DISTANCE, &mut s.grass_distance),
                "grass_density" => num(range::GRASS_DENSITY, &mut s.grass_density),
                "surface_detail" => flag(&mut s.surface_detail),
                "view_distance" => num(range::VIEW_DISTANCE, &mut s.view_distance),
                "show_fps" => flag(&mut s.show_fps),
                "unlock_all" => flag(&mut s.unlock_all),
                "sandbox_ascension" => num(range::SANDBOX_ASCENSION, &mut s.sandbox_ascension),
                "brightness" => num(range::BRIGHTNESS, &mut s.brightness),
                "sun_elevation" => num(range::SUN_ELEVATION, &mut s.sun_elevation),
                "sun_azimuth" => num(range::SUN_AZIMUTH, &mut s.sun_azimuth),
                "wind" => num(range::WIND, &mut s.wind),
                _ => {}
            }
        }
        // Version 2 made the world much larger and the fog exponential, so a
        // view distance saved before that would look far too foggy.
        if version < 2 {
            s.view_distance = Self::default().view_distance;
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_the_file_format() {
        let s = Settings {
            mouse_sensitivity: 1.7,
            invert_y: true,
            grass: false,
            sun_azimuth: 90.0,
            ..Default::default()
        };
        assert_eq!(Settings::parse(&s.serialize()), s);
    }

    #[test]
    fn ignores_junk_and_clamps_out_of_range_values() {
        let s = Settings::parse("fov_degrees = 500\nvsync = maybe\nnonsense\nwind = NaN\nfoo = 1");
        assert_eq!(s.fov_degrees, 110.0);
        assert_eq!(s.vsync, Settings::default().vsync);
        assert_eq!(s.wind, Settings::default().wind);
    }

    #[test]
    fn old_files_keep_choices_but_get_the_new_view_distance() {
        let old = "grass_distance = 45\nview_distance = 160\nwind = 0.55\n";
        let s = Settings::parse(old);
        assert_eq!(s.grass_distance, 45.0);
        assert_eq!(s.wind, 0.55);
        assert_eq!(s.view_distance, Settings::default().view_distance);
        // Current files keep whatever view distance was chosen.
        let current = Settings { view_distance: 200.0, ..Default::default() };
        assert_eq!(Settings::parse(&current.serialize()).view_distance, 200.0);
    }

    #[test]
    fn default_sun_matches_the_original_scene() {
        let d = Settings::default().sun_direction();
        let original = Vec3::new(0.55, -0.62, 0.58).normalize();
        assert!(d.dot(original) > 0.999, "{d:?}");
    }
}
