//! On-screen UI (egui).
//!
//! One model drives everything you can do: `abilities.rs` describes every
//! tree and level (with its key), and each frame the game fills a [`Hud`]
//! with what's ready, cooling down or active. That model is drawn two ways:
//!
//! - **In play**, a HUD: the ability hotbar (bottom centre: key, cooldown,
//!   charges, active glow), the weapon bar above it, movement meters
//!   (bottom left) and skill points / experience (top right).
//! - **Paused** (Esc), one window with tabs: *Abilities & Weapons* (every
//!   tree, grouped into abilities, weapons and mastery; pick one to see its
//!   levels, keys, upgrade button, and for weapons a stats table) and
//!   *Settings*.

use egui::{Align2, Color32, FontId, Id, LayerId, Order, Pos2, Rect, RichText, Sense, Slider, Stroke, StrokeKind, vec2};

use crate::abilities::{Group, LOADOUT, Moveset, Progress, Skill};
use crate::settings::{Settings, range};
use crate::weapons::Weapon;

/// Width of the label column in the settings grids.
const LABEL_WIDTH: f32 = 140.0;

const ACCENT: Color32 = Color32::from_rgb(110, 200, 255);
const UNLOCKED: Color32 = Color32::from_rgb(140, 225, 150);
const WARM: Color32 = Color32::from_rgb(255, 180, 90);
/// Capstones (the fifth, strongest level of each tree).
const GOLD: Color32 = Color32::from_rgb(255, 205, 90);
const PANEL: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 150);

/// What an ability tile shows.
#[derive(Clone, Copy, PartialEq)]
pub enum SlotState {
    Ready,
    /// Recharging: fraction of the wait left (1 = just used).
    Cooldown(f32),
    /// Running or switched on: fraction left (1 for toggles).
    Active(f32),
    /// Nothing to do right now (e.g. no clone to swap with).
    Unavailable,
}

/// One tile on the ability hotbar.
pub struct Slot {
    pub key: String,
    pub name: String,
    pub state: SlotState,
    /// (current, max): charges ready, clones out, pads placed...
    pub charges: Option<(u32, u32)>,
}

/// One weapon on the weapon bar.
pub struct WeaponSlot {
    pub weapon: Weapon,
    pub unlocked: bool,
    pub selected: bool,
    /// Reload done, 0..1.
    pub ready: f32,
}

/// Movement read-outs.
#[derive(Default)]
pub struct Meters {
    /// m/s.
    pub speed: f32,
    pub momentum: Option<f32>,
    /// (label, fraction) for hover/levitation energy.
    pub energy: Option<(&'static str, f32)>,
    /// (left, max) rockets while gliding.
    pub rockets: Option<(u32, u32)>,
    /// Short states worth knowing: "Low gravity", "Wall run", ...
    pub status: Vec<&'static str>,
}

/// Multiplayer status for the menu and HUD.
pub struct NetView {
    /// Hosting or joining (connected or not).
    pub active: bool,
    pub is_host: bool,
    pub connected: bool,
    pub status: String,
    /// Automatic router setup: a line, and whether internet players can
    /// join (None while still checking).
    pub port_forwarding: (String, Option<bool>),
    pub public_address: Option<String>,
    /// Host: the code to give the other player.
    pub join_code: Option<String>,
    /// Your colour and the other player's (linear RGB).
    pub my_color: glam::Vec3,
    pub other_color: glam::Vec3,
    /// This computer's address on the local network.
    pub lan_ip: Option<String>,
    /// Signed in to Steam (as this name): Steam multiplayer is available.
    pub steam_name: Option<String>,
    /// The current game goes through Steam.
    pub over_steam: bool,
    /// The other player's Steam name, playing through Steam.
    pub peer_name: Option<String>,
    /// Hosting through Steam: your online friends to invite, which of them
    /// you've invited, and whether Steam's own invite window can open.
    pub steam_friends: Vec<crate::steam::SteamFriend>,
    pub invited: Vec<u64>,
    pub steam_overlay: bool,
}

/// The fight's score and phase, for the top of the screen.
pub struct MatchView {
    pub round: u32,
    pub phase: crate::arena::Phase,
    pub time_left: f32,
    /// Rounds won: [host (gold), joiner (teal)].
    pub scores: [u32; 2],
    pub winner: Option<u8>,
    /// Which player you are (0 host, 1 joiner).
    pub me: usize,
}

/// Everything the HUD and the Abilities tab show, filled in by the game.
pub struct Hud<'a> {
    pub found: usize,
    pub total: usize,
    pub kills: u32,
    pub slots: Vec<Slot>,
    pub weapons: Vec<WeaponSlot>,
    pub meters: Meters,
    pub net: NetView,
    pub match_view: Option<MatchView>,
    /// The other player's head on screen (-1..1 both ways) and health.
    pub enemy: Option<(glam::Vec2, f32)>,
    /// Out of the current round.
    pub dead: bool,
    /// Auto-update status, and whether a restart would apply it.
    pub update: Option<(String, bool)>,
    /// A short message (crystal found, ability unlocked...), seconds since
    /// it appeared and seconds left.
    pub toast: Option<(&'a str, f32, f32)>,
    /// Feedback effects, banners and guidance.
    pub fx: Fx,
    /// Each of the ten slots' key or button, as shown.
    pub slot_keys: Vec<String>,
    pub vitals: Option<VitalsView>,
    pub popups: Vec<ScreenText>,
    pub drone_bars: Vec<BarView>,
    /// The pause menu was opened from the title screen.
    pub on_title: bool,
}

/// Your health, for the bar at the bottom left.
pub struct VitalsView {
    pub health: f32,
    pub max: f32,
    pub shield: f32,
    pub max_shield: f32,
    /// Arcane Ward points (a magic barrier on top of everything).
    pub ward: f32,
    /// The pale "just lost" part of the bar.
    pub trail: f32,
    /// Health is coming back.
    pub regen: bool,
    pub heal_flash: f32,
    pub invulnerable: bool,
    pub adrenaline: bool,
    /// Second Wind is ready (when you have it).
    pub second_wind: Option<bool>,
}

/// A number or word floating over the world (positions in -1..1 screen
/// coordinates, y up).
pub struct ScreenText {
    pub ndc: glam::Vec2,
    pub text: String,
    pub color: Color32,
    pub age: f32,
    pub big: bool,
}

/// A drone's health bar over it.
pub struct BarView {
    pub ndc: glam::Vec2,
    pub health: f32,
    /// Shrinks with distance.
    pub scale: f32,
    pub bleeding: bool,
    pub burning: bool,
    pub stunned: bool,
}

/// A big announcement across the upper middle of the screen.
#[derive(Clone)]
pub struct Banner {
    pub title: String,
    pub subtitle: String,
    pub color: Color32,
    /// Seconds since shown, and how long it stays.
    pub age: f32,
    pub life: f32,
}

impl Banner {
    pub fn new(title: impl Into<String>, subtitle: impl Into<String>, color: Color32) -> Self {
        Self { title: title.into(), subtitle: subtitle.into(), color, age: 0.0, life: 2.6 }
    }
}

/// Short-lived feedback the HUD draws on top of everything.
#[derive(Default)]
pub struct Fx {
    /// Hit marker (1 = just hit, fades to 0), and a kill marker.
    pub hit: f32,
    pub kill: f32,
    /// Red flash at the screen edges after taking damage.
    pub damage: f32,
    /// Low health: the edges pulse.
    pub low_health: bool,
    pub banner: Option<Banner>,
    /// What to do next, shown top left.
    pub goals: Vec<String>,
    /// The tutorial's current step, while it runs.
    pub tutorial: Option<crate::tutorial::TutorialView>,
    /// Seconds since the game started (for pulses).
    pub time: f32,
    /// Air rushing past at high speed, 0..1 (streaks at the screen edges).
    pub speed: f32,
    /// A spell being cast.
    pub cast: Option<CastView>,
    /// Runes waiting to go off: spell, how long before it fizzles (1..0),
    /// boost.
    pub runes: Vec<(String, f32, f32)>,
    /// An ability half placed (the first barrier corner, the first rift
    /// gate): what's waiting on you.
    pub pending: Option<Pending>,
}

/// A spell being cast, for its bar.
#[derive(Clone)]
pub struct CastView {
    pub name: String,
    /// The bar, 0..1, then how far into overcharge (0..1).
    pub progress: f32,
    pub overcharge: f32,
    /// It is planted when the cast ends, to be set off with its key.
    pub rune: bool,
}

/// A multi-press ability in progress, shown under the crosshair.
#[derive(Clone)]
pub struct Pending {
    pub title: String,
    /// What the next press does.
    pub hint: String,
    /// Steps done and steps in all (e.g. 1 of 2).
    pub step: (u32, u32),
    /// Time left before it's cancelled, 0..1.
    pub left: f32,
}

pub const BANNER_GOLD: Color32 = GOLD;
pub const BANNER_BLUE: Color32 = ACCENT;
pub const BANNER_GREEN: Color32 = UNLOCKED;
pub const BANNER_RED: Color32 = Color32::from_rgb(255, 95, 80);

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Abilities,
    Items,
    Multiplayer,
    Settings,
}

/// The pause menu's own state between frames.
pub struct MenuState {
    pub tab: Tab,
    pub selected: Skill,
    /// Waiting for a key to bind this slot (0-9) to.
    pub binding: Option<usize>,
    /// Where the skill and item circles are panned to, and their zoom.
    pub skill_view: (egui::Vec2, f32),
    pub item_view: (egui::Vec2, f32),
    /// Multiplayer text fields.
    pub host_port: String,
    pub join_address: String,
    /// The item shown in detail on the Items tab.
    pub selected_item: usize,
    /// `Reset all players` was clicked once; a second click confirms.
    pub confirm_reset: bool,
    /// The Steam friend list: the search typed, and showing offline friends.
    pub friend_search: String,
    pub show_offline: bool,
    /// Right click (away from the slots) on the Skills tab: put the chosen
    /// skill on the right mouse button.
    pub assign_right: bool,
}

impl Default for MenuState {
    fn default() -> Self {
        Self {
            tab: Tab::Abilities,
            selected: Skill::Dash,
            binding: None,
            skill_view: (egui::Vec2::ZERO, 0.62),
            item_view: (egui::Vec2::ZERO, 0.5),
            host_port: crate::net::DEFAULT_PORT.to_string(),
            join_address: String::new(),
            selected_item: 0,
            confirm_reset: false,
            friend_search: String::new(),
            show_offline: false,
            assign_right: false,
        }
    }
}

/// What the player chose in the menu this frame.
#[derive(Clone, PartialEq, Eq)]
pub enum MenuAction {
    None,
    Resume,
    Quit,
    /// Start hosting on this port.
    Host(u16),
    /// Join the host at this address.
    Join(String),
    /// Leave or stop hosting.
    Disconnect,
    /// Host through Steam (friends join from their friends list).
    HostSteam,
    /// Join a Steam friend's game (their Steam ID).
    JoinSteam(u64),
    /// Open Steam's invite dialog.
    InviteSteam,
    /// Invite this Steam friend (their Steam ID).
    InviteFriend(u64),
    /// Play the tutorial again.
    Tutorial,
    /// Reset every player's progress and the score.
    ResetAll,
    /// Restart into the installed update.
    Restart,
    /// Back to the title screen.
    MainMenu,
}

/// The game's look for every egui widget: dark slate panels, rounded
/// corners, a sky-blue accent.
pub fn apply_theme(ctx: &egui::Context) {
    use egui::{CornerRadius, Shadow};
    let mut v = egui::Visuals::dark();
    let r = CornerRadius::same(7);
    v.window_corner_radius = CornerRadius::same(14);
    v.window_fill = Color32::from_rgba_unmultiplied(16, 20, 28, 242);
    v.panel_fill = v.window_fill;
    v.window_stroke = Stroke::new(1.0, Color32::from_rgba_unmultiplied(120, 170, 220, 45));
    v.window_shadow = Shadow { offset: [0, 10], blur: 40, spread: 2, color: Color32::from_black_alpha(140) };
    v.popup_shadow = Shadow { offset: [0, 4], blur: 16, spread: 0, color: Color32::from_black_alpha(120) };
    v.menu_corner_radius = r;
    v.extreme_bg_color = Color32::from_rgb(10, 13, 19);
    v.faint_bg_color = Color32::from_rgb(24, 29, 39);
    v.selection.bg_fill = Color32::from_rgb(38, 96, 140);
    v.selection.stroke = Stroke::new(1.0, ACCENT);
    v.hyperlink_color = ACCENT;
    v.slider_trailing_fill = true;
    let w = &mut v.widgets;
    for (state, bg, fg) in [
        (&mut w.noninteractive, Color32::from_rgb(22, 27, 36), Color32::from_gray(200)),
        (&mut w.inactive, Color32::from_rgb(34, 41, 54), Color32::from_gray(215)),
        (&mut w.hovered, Color32::from_rgb(46, 58, 78), Color32::WHITE),
        (&mut w.active, Color32::from_rgb(52, 74, 104), Color32::WHITE),
        (&mut w.open, Color32::from_rgb(40, 50, 66), Color32::WHITE),
    ] {
        state.corner_radius = r;
        state.bg_fill = bg;
        state.weak_bg_fill = bg;
        state.fg_stroke.color = fg;
    }
    w.noninteractive.bg_stroke = Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 14));
    w.inactive.bg_stroke = Stroke::NONE;
    w.hovered.bg_stroke = Stroke::new(1.0, Color32::from_rgba_unmultiplied(110, 200, 255, 110));
    w.active.bg_stroke = Stroke::new(1.0, ACCENT);
    ctx.set_theme(egui::Theme::Dark);
    ctx.set_visuals_of(egui::Theme::Dark, v);
    ctx.all_styles_mut(|s| {
        s.spacing.item_spacing = vec2(8.0, 6.0);
        s.spacing.button_padding = vec2(10.0, 5.0);
        s.spacing.slider_width = 170.0;
        s.spacing.window_margin = egui::Margin::same(16);
    });
}

/// What was chosen on the title screen.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TitleAction {
    None,
    Play,
    Multiplayer,
    Settings,
    Quit,
}

/// What the title screen shows about the save.
pub struct TitleInfo {
    /// Seconds since the title appeared (for animation).
    pub time: f32,
    /// There is progress to continue.
    pub has_progress: bool,
    pub crystals: (usize, usize),
    pub levels: u32,
    pub version: &'static str,
    pub update: Option<String>,
}

/// The front door: the valley behind, the name, and where to go.
pub fn draw_title(ctx: &egui::Context, info: &TitleInfo) -> TitleAction {
    let screen = ctx.content_rect();
    let painter = ctx.layer_painter(LayerId::new(Order::Background, Id::unique("title shade")));
    let appear = (info.time / 1.2).clamp(0.0, 1.0);
    let ease = 1.0 - (1.0 - appear).powi(3);
    // Darken the left side so the text reads over any view.
    horizontal_gradient(&painter, screen, Color32::from_black_alpha((200.0 * ease) as u8), Color32::TRANSPARENT, 0.62);
    vertical_fade(&painter, Rect::from_min_max(screen.left_bottom() - vec2(0.0, 160.0), screen.right_bottom()), (150.0 * ease) as u8);

    let left = screen.left() + (screen.width() * 0.08).max(48.0) - 30.0 * (1.0 - ease);
    let mut y = screen.top() + screen.height() * 0.2;
    let alpha = |c: Color32| c.gamma_multiply(ease);
    // The name, with a soft glow.
    let logo = FontId::proportional((screen.height() * 0.13).clamp(56.0, 110.0));
    for (off, a) in [(vec2(0.0, 4.0), 120u8), (vec2(0.0, 0.0), 0)] {
        if a > 0 {
            painter.text(Pos2::new(left, y) + off, Align2::LEFT_TOP, "OzLand", logo.clone(), Color32::from_black_alpha((a as f32 * ease) as u8));
        }
    }
    let logo_rect = painter.text(Pos2::new(left, y), Align2::LEFT_TOP, "OzLand", logo.clone(), alpha(Color32::from_rgb(245, 248, 255)));
    // An accent underline that draws itself in.
    let line_w = logo_rect.width() * ease;
    painter.rect_filled(Rect::from_min_size(Pos2::new(left + 4.0, logo_rect.bottom() + 2.0), vec2(line_w * 0.45, 4.0)), 2.0, alpha(GOLD));
    painter.rect_filled(Rect::from_min_size(Pos2::new(left + 4.0 + line_w * 0.47, logo_rect.bottom() + 2.0), vec2(line_w * 0.18, 4.0)), 2.0, alpha(ACCENT));
    y = logo_rect.bottom() + 18.0;
    painter.text(
        Pos2::new(left + 4.0, y),
        Align2::LEFT_TOP,
        "Explore the valley. Master impossible movement. Duel a friend.",
        FontId::proportional(18.0),
        alpha(Color32::from_gray(215)),
    );
    y += 64.0;

    let mut action = TitleAction::None;
    let entries = [
        (if info.has_progress { "Continue" } else { "Play" }, TitleAction::Play),
        ("Multiplayer", TitleAction::Multiplayer),
        ("Settings", TitleAction::Settings),
        ("Quit", TitleAction::Quit),
    ];
    egui::Area::new(Id::unique("title menu"))
        .fixed_pos(Pos2::new(left, y))
        .order(Order::Middle)
        .show(ctx, |ui| {
            for (i, (label, what)) in entries.into_iter().enumerate() {
                // Each entry slides in a little after the one above.
                let t = ((info.time - 0.35 - i as f32 * 0.08) / 0.5).clamp(0.0, 1.0);
                let e = 1.0 - (1.0 - t).powi(3);
                let (rect, response) = ui.allocate_exact_size(vec2(340.0, 50.0), Sense::click());
                let hover = ui.ctx().animate_bool(response.id, response.hovered());
                let p = ui.painter();
                let rect = rect.translate(vec2(-24.0 * (1.0 - e) + 10.0 * hover, 0.0));
                if hover > 0.0 {
                    p.rect_filled(rect.expand2(vec2(14.0, 0.0)), 8.0, Color32::from_white_alpha((18.0 * hover) as u8));
                    p.rect_filled(
                        Rect::from_min_size(rect.left_top() + vec2(-14.0, 10.0), vec2(4.0, rect.height() - 20.0)),
                        2.0,
                        GOLD.gamma_multiply(hover),
                    );
                }
                let primary = i == 0;
                let color = if primary { Color32::WHITE } else { Color32::from_gray(200) }.lerp_to_gamma(GOLD, hover * 0.35);
                p.text(
                    rect.left_center(),
                    Align2::LEFT_CENTER,
                    label,
                    FontId::proportional(if primary { 30.0 } else { 25.0 }),
                    color.gamma_multiply(e),
                );
                if response.clicked() {
                    action = what;
                }
            }
        });

    // Bottom: the save, the version and what to press.
    let bottom = screen.bottom() - 26.0;
    let mut status = String::new();
    if info.has_progress {
        status = format!("Skill crystals {}/{}  ·  {} ability levels", info.crystals.0, info.crystals.1, info.levels);
    }
    painter.text(Pos2::new(left + 4.0, bottom), Align2::LEFT_BOTTOM, status, FontId::proportional(14.0), alpha(Color32::from_gray(170)));
    let mut right = format!("v{}", info.version);
    if let Some(u) = &info.update {
        right = format!("{u}   {right}");
    }
    painter.text(screen.right_bottom() + vec2(-24.0, -26.0), Align2::RIGHT_BOTTOM, right, FontId::proportional(13.0), alpha(Color32::from_gray(150)));
    let pulse = 0.55 + 0.45 * (info.time * 2.4).sin();
    if info.time > 1.4 {
        painter.text(
            screen.center_bottom() + vec2(0.0, -26.0),
            Align2::CENTER_BOTTOM,
            "Press Enter to play",
            FontId::proportional(14.0),
            Color32::from_white_alpha((150.0 * pulse) as u8),
        );
    }
    action
}

/// Black over everything (fades between screens).
pub fn draw_fade(ctx: &egui::Context, alpha: f32) {
    if alpha <= 0.001 {
        return;
    }
    ctx.layer_painter(LayerId::new(Order::Debug, Id::unique("fade")))
        .rect_filled(ctx.content_rect(), 0.0, Color32::from_black_alpha((alpha.clamp(0.0, 1.0) * 255.0) as u8));
}

/// A left-to-right gradient covering `rect` up to `reach` of its width.
fn horizontal_gradient(painter: &egui::Painter, rect: Rect, from: Color32, to: Color32, reach: f32) {
    let mut mesh = egui::Mesh::default();
    let x1 = rect.left() + rect.width() * reach;
    mesh.colored_vertex(rect.left_top(), from);
    mesh.colored_vertex(Pos2::new(x1, rect.top()), to);
    mesh.colored_vertex(Pos2::new(x1, rect.bottom()), to);
    mesh.colored_vertex(rect.left_bottom(), from);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(mesh);
}

/// Transparent at the top of `rect`, black (`alpha`) at the bottom.
fn vertical_fade(painter: &egui::Painter, rect: Rect, alpha: u8) {
    let mut mesh = egui::Mesh::default();
    let (top, bottom) = (Color32::TRANSPARENT, Color32::from_black_alpha(alpha));
    mesh.colored_vertex(rect.left_top(), top);
    mesh.colored_vertex(rect.right_top(), top);
    mesh.colored_vertex(rect.right_bottom(), bottom);
    mesh.colored_vertex(rect.left_bottom(), bottom);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(mesh);
}

/// A frame of `color` at the screen edges, fading to nothing `depth` in.
fn vignette(painter: &egui::Painter, screen: Rect, color: Color32, depth: f32) {
    let inner = screen.shrink(depth);
    let outer = [screen.left_top(), screen.right_top(), screen.right_bottom(), screen.left_bottom()];
    let inner = [inner.left_top(), inner.right_top(), inner.right_bottom(), inner.left_bottom()];
    let mut mesh = egui::Mesh::default();
    for i in 0..4 {
        mesh.colored_vertex(outer[i], color);
        mesh.colored_vertex(inner[i], Color32::TRANSPARENT);
    }
    for i in 0..4u32 {
        let (o0, i0, o1, i1) = (i * 2, i * 2 + 1, (i * 2 + 2) % 8, (i * 2 + 3) % 8);
        mesh.add_triangle(o0, o1, i1);
        mesh.add_triangle(o0, i1, i0);
    }
    painter.add(mesh);
}

/// A dark band behind a banner, fading out at both ends.
fn band(painter: &egui::Painter, rect: Rect, alpha: f32) {
    let solid = Color32::from_black_alpha((150.0 * alpha) as u8);
    let fade = rect.width() * 0.25;
    let xs = [rect.left(), rect.left() + fade, rect.right() - fade, rect.right()];
    let cs = [Color32::TRANSPARENT, solid, solid, Color32::TRANSPARENT];
    let mut mesh = egui::Mesh::default();
    for (x, c) in xs.iter().zip(cs) {
        mesh.colored_vertex(Pos2::new(*x, rect.top()), c);
        mesh.colored_vertex(Pos2::new(*x, rect.bottom()), c);
    }
    for i in 0..3u32 {
        let b = i * 2;
        mesh.add_triangle(b, b + 2, b + 3);
        mesh.add_triangle(b, b + 3, b + 1);
    }
    painter.add(mesh);
}

/// Draws the UI for one frame: the HUD while playing, the menu while paused.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    ctx: &egui::Context,
    settings: &mut Settings,
    progress: &mut Progress,
    menu: &mut MenuState,
    hud: &Hud,
    moves: &Moveset,
    menu_open: bool,
    fps: f32,
) -> MenuAction {
    if !menu_open {
        draw_hud(ctx, settings, progress, hud, fps);
        return MenuAction::None;
    }

    // Dim the game behind the menu.
    ctx.layer_painter(LayerId::new(Order::Background, Id::unique("menu dim")))
        .rect_filled(ctx.content_rect(), 0.0, Color32::from_black_alpha(150));

    let mut action = MenuAction::None;
    egui::Window::new(if hud.on_title { "OzLand" } else { "Menu (the world keeps going)" })
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
        .collapsible(false)
        .resizable(false)
        .fixed_size([860.0, 0.0])
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                for (tab, name) in [
                    (Tab::Abilities, "Skills"),
                    (Tab::Items, "Items"),
                    (Tab::Multiplayer, "Multiplayer"),
                    (Tab::Settings, "Settings"),
                ] {
                    ui.selectable_value(&mut menu.tab, tab, RichText::new(name).size(17.0));
                }
            });
            ui.separator();
            match menu.tab {
                Tab::Abilities => abilities_tab(ui, settings, progress, menu, hud, moves),
                Tab::Items => items_tab(ui, settings, progress, menu, moves),
                Tab::Multiplayer => {
                    if let Some(a) = multiplayer_tab(ui, menu, hud) {
                        action = a;
                    }
                }
                Tab::Settings => settings_tab(ui, settings),
            }
            ui.separator();
            ui.horizontal(|ui| {
                let resume = if hud.on_title { "Back" } else { "Resume" };
                if ui.button(RichText::new(resume).strong()).clicked() {
                    action = MenuAction::Resume;
                }
                if menu.tab == Tab::Settings && ui.button("Reset to defaults").clicked() {
                    let (unlock, ascension) = (settings.unlock_all, settings.sandbox_ascension);
                    *settings = Settings::default();
                    (settings.unlock_all, settings.sandbox_ascension) = (unlock, ascension);
                }
                let hint = if hud.on_title { "Esc to go back · saved automatically" } else { "Esc to resume · saved automatically" };
                ui.label(RichText::new(hint).small().weak());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Quit").clicked() {
                        action = MenuAction::Quit;
                    }
                    if !hud.on_title && ui.button("Main menu").clicked() {
                        action = MenuAction::MainMenu;
                    }
                    if !hud.on_title && ui.button("Tutorial").clicked() {
                        action = MenuAction::Tutorial;
                    }
                });
            });
        });
    action
}

/// The level shown for `tree` (sandbox shows everything unlocked).
/// A skill's rank as shown (sandbox: everything at the top).
fn shown_rank(skill: Skill, settings: &Settings, progress: &Progress) -> u32 {
    if settings.unlock_all { skill.max_rank() } else { progress.rank(skill) }
}

fn plural(n: u32, word: &str) -> String {
    format!("{n} {word}{}", if n == 1 { "" } else { "s" })
}

/// The colour of a region of a tree.
fn group_color(group: Group) -> Color32 {
    match group {
        Group::Combat => Color32::from_rgb(255, 125, 95),
        Group::Body => Color32::from_rgb(120, 225, 140),
        Group::Motion => ACCENT,
        Group::Control => Color32::from_rgb(200, 130, 255),
        Group::Arcane => Color32::from_rgb(255, 120, 215),
        Group::Mastery => GOLD,
    }
}

/// A node's short mark: initials of its name.
fn initials(name: &str) -> String {
    let words: Vec<&str> = name.split([' ', '\'']).filter(|w| w.len() > 1).collect();
    match words.as_slice() {
        [one] => one.chars().take(2).collect(),
        [a, b, ..] => [a.chars().next(), b.chars().next()].into_iter().flatten().collect(),
        [] => "?".into(),
    }
}

/// One node of a tree drawing.
struct GraphNode {
    name: String,
    /// Short text inside the circle.
    mark: String,
    group: Group,
    owned: bool,
    /// Can be bought right now.
    available: bool,
    /// Behind a trial (gold ring).
    capstone: bool,
    /// Ranks: (have, of) as pips under the circle.
    ranks: Option<(u32, u32)>,
    /// Equipped (skills): a bright outer ring.
    equipped: bool,
    tooltip: String,
    /// Indices of nodes it needs.
    needs: Vec<usize>,
    /// On a ring of its own outside everything (Ascension).
    outer: bool,
}

/// Distance between rings at zoom 1.
const RING: f32 = 82.0;

/// Draws a tree as a circle around "YOU": a coloured slice per group,
/// rings outward by depth, lines to what each node needs. Drag, WASD or the
/// arrow keys look around; the wheel zooms. Returns the node clicked.
fn draw_graph(ui: &mut egui::Ui, nodes: &[GraphNode], view: &mut (egui::Vec2, f32), selected: Option<usize>, height: f32) -> Option<usize> {
    let groups: Vec<Group> = nodes.iter().map(|n| n.group).collect();
    let needs: Vec<Vec<usize>> = nodes.iter().map(|n| n.needs.clone()).collect();
    let outer: Vec<bool> = nodes.iter().map(|n| n.outer).collect();
    let layout = crate::abilities::radial_layout(&groups, &needs, &outer);
    let rings = layout.iter().map(|l| l.1).fold(1.0, f32::max);

    let size = vec2(ui.available_width(), height);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
    // Looking around: drag, keys, wheel.
    if response.dragged() {
        view.0 += response.drag_delta();
    }
    let (dt, keys, scroll) = ui.input(|i| {
        let k = |a: egui::Key, b: egui::Key| (i.key_down(a) || i.key_down(b)) as i32 as f32;
        let keys = vec2(
            k(egui::Key::A, egui::Key::ArrowLeft) - k(egui::Key::D, egui::Key::ArrowRight),
            k(egui::Key::W, egui::Key::ArrowUp) - k(egui::Key::S, egui::Key::ArrowDown),
        );
        (i.stable_dt.min(0.1), keys, i.smooth_scroll_delta.y)
    });
    view.0 += keys * 520.0 * dt;
    if response.hovered() && scroll != 0.0 {
        view.1 = (view.1 * (scroll * 0.002).exp()).clamp(0.35, 2.0);
    }
    if keys != egui::Vec2::ZERO || response.dragged() {
        ui.ctx().request_repaint();
    }
    let zoom = view.1;
    let center = rect.center() + view.0;
    // Each ring far enough out that its nodes have room within their
    // group's slice (and always further than the ring inside it).
    let n = nodes.len().max(1) as f32;
    let mut radius = vec![0.0f32; rings as usize + 2];
    for d in 1..radius.len() {
        let mut needed = d as f32 * RING;
        for g in &groups {
            let slice = std::f32::consts::TAU * groups.iter().filter(|x| *x == g).count() as f32 / n;
            let count = (0..nodes.len()).filter(|&i| groups[i] == *g && layout[i].1 as usize == d).count() as f32;
            needed = needed.max(count * 58.0 / slice);
        }
        radius[d] = needed.max(radius[d - 1] + RING * 0.8);
    }
    let ring_r = |ring: f32| radius[(ring as usize).min(radius.len() - 1)];
    let at = |i: usize| {
        let (a, ring) = layout[i];
        center + vec2(a.cos(), a.sin()) * ring_r(ring) * zoom
    };
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 8.0, Color32::from_rgb(12, 15, 21));
    let time = ui.input(|i| i.time) as f32;
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));

    // Rings and the slice of each group.
    for r in 1..=rings as u32 {
        painter.circle_stroke(center, ring_r(r as f32) * zoom, Stroke::new(1.0, Color32::from_white_alpha(10)));
    }
    let mut seen: Vec<Group> = Vec::new();
    for (i, n) in nodes.iter().enumerate() {
        if seen.contains(&n.group) {
            continue;
        }
        seen.push(n.group);
        let angles: Vec<f32> = (0..nodes.len()).filter(|&j| nodes[j].group == n.group).map(|j| layout[j].0).collect();
        let mid = angles.iter().sum::<f32>() / angles.len() as f32;
        let _ = i;
        let p = center + vec2(mid.cos(), mid.sin()) * (ring_r(rings) + RING * 0.75) * zoom;
        painter.text(p, Align2::CENTER_CENTER, n.group.name().to_uppercase(), FontId::proportional(13.0 * zoom.max(0.6)), group_color(n.group).gamma_multiply(0.6));
    }

    // Links.
    for (i, n) in nodes.iter().enumerate() {
        let to = at(i);
        if n.needs.is_empty() && !n.outer {
            let c = if n.owned { group_color(n.group) } else { Color32::from_gray(60) };
            painter.line_segment([center, to], Stroke::new(2.0 * zoom.max(0.5), c.gamma_multiply(0.7)));
        }
        for &k in &n.needs {
            let (w, c) = match (nodes[k].owned, n.owned) {
                (true, true) => (3.0, group_color(n.group)),
                (true, false) => (2.0, Color32::from_gray(150)),
                _ => (1.5, Color32::from_gray(60)),
            };
            painter.line_segment([at(k), to], Stroke::new(w * zoom.max(0.5), c));
        }
    }
    painter.circle_filled(center, 22.0 * zoom, Color32::from_rgb(40, 48, 64));
    painter.circle_stroke(center, 22.0 * zoom, Stroke::new(2.0, Color32::WHITE));
    painter.text(center, Align2::CENTER_CENTER, "YOU", FontId::proportional(12.0 * zoom.max(0.6)), Color32::WHITE);

    // Nodes.
    let pointer = response.hover_pos();
    let mut hovered = None;
    for (i, n) in nodes.iter().enumerate() {
        let c = at(i);
        let r = if n.capstone { 17.0 } else { 14.0 } * zoom;
        let color = group_color(n.group);
        if n.available {
            let pulse = 0.5 + 0.5 * (time * 4.0).sin();
            painter.circle_filled(c, r + 6.0 * zoom, WARM.gamma_multiply(0.15 + 0.2 * pulse));
        }
        painter.circle_filled(c, r, if n.owned { color } else { Color32::from_rgb(26, 31, 41) });
        let ring = if n.capstone {
            Stroke::new(2.5, GOLD.gamma_multiply(if n.owned { 1.0 } else { 0.6 }))
        } else if n.available {
            Stroke::new(2.0, WARM)
        } else if n.owned {
            Stroke::new(1.5, Color32::WHITE.gamma_multiply(0.6))
        } else {
            Stroke::new(1.0, Color32::from_gray(80))
        };
        painter.circle_stroke(c, r, ring);
        if n.equipped {
            painter.circle_stroke(c, r + 3.0 * zoom, Stroke::new(2.0, Color32::WHITE.gamma_multiply(0.85)));
        }
        if selected == Some(i) {
            painter.circle_stroke(c, r + 6.5 * zoom, Stroke::new(2.0, WARM));
        }
        let text_color = if n.owned { Color32::from_rgb(15, 18, 24) } else if n.available { WARM } else { Color32::from_gray(140) };
        painter.text(c, Align2::CENTER_CENTER, &n.mark, FontId::proportional(11.0 * zoom.max(0.7)), text_color);
        let mut below = c.y + r + 2.0;
        if let Some((have, of)) = n.ranks.filter(|r| r.1 > 1) {
            for k in 0..of {
                let p = Pos2::new(c.x + (k as f32 - (of - 1) as f32 * 0.5) * 7.0 * zoom, below + 2.5);
                if k < have {
                    painter.circle_filled(p, 2.3 * zoom.max(0.7), color);
                } else {
                    painter.circle_stroke(p, 2.3 * zoom.max(0.7), Stroke::new(1.0, Color32::from_gray(110)));
                }
            }
            below += 6.0 * zoom;
        }
        if zoom >= 0.6 {
            let name: String = if n.name.chars().count() > 14 { format!("{}…", n.name.chars().take(13).collect::<String>()) } else { n.name.clone() };
            painter.text(
                Pos2::new(c.x, below),
                Align2::CENTER_TOP,
                name,
                FontId::proportional(9.5 * zoom.max(0.8)),
                if n.owned || n.available { Color32::from_gray(220) } else { Color32::from_gray(120) },
            );
        }
        if pointer.is_some_and(|p| p.distance(c) <= r + 3.0) {
            hovered = Some(i);
        }
    }
    painter.text(
        rect.left_bottom() + vec2(8.0, -6.0),
        Align2::LEFT_BOTTOM,
        "Drag, WASD or arrows to look around · wheel to zoom",
        FontId::proportional(10.5),
        Color32::from_gray(120),
    );
    if ui.put(Rect::from_min_size(rect.right_top() + vec2(-72.0, 6.0), vec2(64.0, 22.0)), egui::Button::new("Centre")).clicked() {
        view.0 = egui::Vec2::ZERO;
    }

    let hovered = hovered?;
    let tip = nodes[hovered].tooltip.clone();
    let name = nodes[hovered].name.clone();
    let clicked = response.clicked();
    response.on_hover_ui_at_pointer(|ui| {
        ui.set_max_width(260.0);
        ui.label(RichText::new(name).strong());
        ui.label(tip);
    });
    clicked.then_some(hovered)
}

/// A small key label, like a keyboard key.
fn key_chip(ui: &mut egui::Ui, key: &str) {
    if !key.is_empty() {
        ui.label(RichText::new(format!(" {key} ")).monospace().small().color(Color32::WHITE).background_color(Color32::from_gray(70)));
    }
}

/// The trial bar for something behind one.
fn trial_bar(ui: &mut egui::Ui, progress: &Progress, trial: &crate::abilities::Trial) {
    let done = progress.trial_done(trial);
    let shown = progress.stat(trial.stat).min(trial.goal);
    let unit = if trial.unit.is_empty() { String::new() } else { format!(" {}", trial.unit) };
    ui.add(
        egui::ProgressBar::new(progress.trial_progress(trial))
            .desired_width(250.0)
            .fill(if done { UNLOCKED } else { Color32::from_rgb(150, 110, 40) })
            .text(if done { "Trial complete".to_owned() } else { format!("{} ({shown:.0}/{:.0}{unit})", trial.description, trial.goal) }),
    );
}

fn abilities_tab(ui: &mut egui::Ui, settings: &mut Settings, progress: &mut Progress, menu: &mut MenuState, hud: &Hud, moves: &Moveset) {
    let slot_key = |i: usize| hud.slot_keys.get(i).cloned().unwrap_or_default();
    // Header: points, experience, sandbox.
    ui.horizontal(|ui| {
        let points = RichText::new(plural(progress.points, "skill point")).size(22.0).color(if progress.points > 0 { WARM } else { Color32::LIGHT_GRAY });
        ui.label(points);
        ui.add_space(12.0);
        ui.vertical(|ui| {
            ui.add(
                egui::ProgressBar::new(progress.xp / progress.xp_needed())
                    .desired_width(240.0)
                    .text(format!("XP {:.0} / {:.0}", progress.xp, progress.xp_needed())),
            );
            ui.label(
                RichText::new(format!(
                    "Crystals {} / {} · drones destroyed {} · skills have their own button; weapons and passives are Items · Tab closes",
                    hud.found, hud.total, hud.kills
                ))
                .small()
                .weak(),
            );
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.vertical(|ui| {
                ui.checkbox(&mut settings.unlock_all, "Sandbox: unlock everything");
                if settings.unlock_all {
                    ui.add(Slider::new(&mut settings.sandbox_ascension, range::SANDBOX_ASCENSION).fixed_decimals(0).text("Ascension"));
                }
            });
        });
    });

    // The ten slots: each has its own key; put a skill in one to use it.
    ui.horizontal(|ui| {
        let n = progress.loadout().len();
        ui.label(RichText::new(format!("Slots {n}/{LOADOUT}")).strong().color(if n >= LOADOUT { WARM } else { Color32::LIGHT_GRAY }));
        let mut clear = None;
        let mut over_slot = false;
        for i in 0..LOADOUT {
            let (rect, response) = ui.allocate_exact_size(vec2(64.0, 38.0), Sense::click());
            let p = ui.painter();
            let hover = response.hovered();
            over_slot |= hover;
            let binding = menu.binding == Some(i);
            let skill = progress.slots()[i];
            let color = skill.map_or(Color32::from_white_alpha(40), |s| group_color(s.group()));
            let fill = if binding {
                Color32::from_rgb(70, 60, 20)
            } else if hover && skill.is_some() {
                Color32::from_rgb(70, 40, 40)
            } else if skill.is_some() {
                Color32::from_rgb(30, 36, 48)
            } else {
                Color32::from_rgb(20, 24, 32)
            };
            p.rect_filled(rect, 6.0, fill);
            p.rect_stroke(rect, 6.0, Stroke::new(if binding { 2.0 } else { 1.5 }, if binding { WARM } else { color }), StrokeKind::Inside);
            // The slot's key, in a little keycap.
            let key = if binding { "?".to_owned() } else { slot_key(i) };
            let cap = Rect::from_min_size(rect.left_top() + vec2(3.0, 3.0), vec2(9.0 + key.chars().count() as f32 * 6.5, 15.0));
            p.rect_filled(cap, 3.0, if binding { WARM } else { Color32::from_gray(75) });
            p.text(cap.center(), Align2::CENTER_CENTER, &key, FontId::monospace(10.5), if binding { Color32::BLACK } else { Color32::WHITE });
            let label = match skill {
                Some(_) if hover => "clear".to_owned(),
                Some(s) => s.name().chars().take(10).collect(),
                None => "empty".to_owned(),
            };
            p.text(
                rect.center_bottom() + vec2(0.0, -4.0),
                Align2::CENTER_BOTTOM,
                label,
                FontId::proportional(9.5),
                if skill.is_some() { Color32::WHITE } else { Color32::from_gray(90) },
            );
            let tip = match skill {
                Some(s) => format!("Slot {} ({}): {}. Left click clears it, right click changes the key.", i + 1, slot_key(i), s.name()),
                None => format!("Slot {} ({}): empty. Pick a skill below and put it here; right click changes the key.", i + 1, slot_key(i)),
            };
            let response = response.on_hover_text(tip);
            if response.clicked() && skill.is_some() {
                clear = Some(i);
            }
            if response.secondary_clicked() {
                menu.binding = Some(i);
            }
        }
        if let Some(i) = clear {
            progress.clear_slot(i);
        }
        // Right click anywhere else: the chosen skill goes on the right button.
        if menu.binding.is_none() && !over_slot && ui.input(|i| i.pointer.secondary_clicked()) {
            menu.assign_right = true;
        }
    });
    if let Some(i) = menu.binding {
        ui.label(RichText::new(format!("Press a key or mouse button for slot {}... (Esc cancels)", i + 1)).color(WARM));
    } else {
        ui.label(
            RichText::new("Pick a skill, then just press the key or mouse button you want it on · right click a slot to change its key")
                .small()
                .weak(),
        );
    }
    // Saved loadouts.
    ui.horizontal(|ui| {
        ui.label(RichText::new("Saved loadouts").small().strong().color(Color32::GRAY));
        for i in 0..crate::abilities::PRESETS {
            let saved = progress.preset(i);
            let filled = saved.iter().any(Option::is_some);
            ui.add_space(6.0);
            let summary: Vec<String> = saved
                .iter()
                .enumerate()
                .filter_map(|(slot, s)| s.map(|s| format!("{}: {}", slot_key(slot), s.name())))
                .collect();
            let load = ui
                .add_enabled(filled, egui::Button::new(format!("Load {}", i + 1)))
                .on_hover_text(summary.join("\n"))
                .on_disabled_hover_text("Nothing saved here yet");
            if load.clicked() {
                progress.load_preset(i);
            }
            if ui.small_button("Save").on_hover_text(format!("Save the slots above as loadout {}", i + 1)).clicked() {
                progress.save_preset(i);
            }
        }
    });
    if settings.unlock_all {
        ui.label(RichText::new("Sandbox: every skill is unlocked at its top rank; put any of them in your slots").small().weak());
    }
    ui.separator();

    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(560.0);
            let nodes: Vec<GraphNode> = Skill::ALL
                .into_iter()
                .map(|s| {
                    let rank = shown_rank(s, settings, progress);
                    GraphNode {
                        name: s.name().to_owned(),
                        // In a slot: its key. Otherwise its initials.
                        mark: progress.slot_of(s).map_or_else(|| initials(s.name()), slot_key),
                        group: s.group(),
                        owned: rank > 0,
                        available: !settings.unlock_all && progress.can_upgrade(s),
                        capstone: s.trial().is_some(),
                        ranks: Some((rank, s.max_rank())),
                        equipped: progress.equipped(s),
                        tooltip: s.rank_info(rank.max(1)).description.to_owned(),
                        needs: s.needs().iter().map(|(n, _)| n.index()).collect(),
                        outer: false,
                    }
                })
                .collect();
            if let Some(i) = draw_graph(ui, &nodes, &mut menu.skill_view, Some(menu.selected.index()), 360.0) {
                menu.selected = Skill::ALL[i];
            }
        });
        ui.separator();
        ui.vertical(|ui| {
            let keys: Vec<String> = (0..LOADOUT).map(slot_key).collect();
            egui::ScrollArea::vertical()
                .id_salt("skill detail")
                .auto_shrink([false, false])
                .max_height(360.0)
                .min_scrolled_height(360.0)
                .show(ui, |ui| skill_detail(ui, menu.selected, settings, progress, &keys));
        });
    });
    let _ = moves;
}

/// The chosen skill: its ranks, what it needs, unlocking it, and which
/// slot (key) it goes in.
fn skill_detail(ui: &mut egui::Ui, skill: Skill, settings: &Settings, progress: &mut Progress, keys: &[String]) {
    let rank = shown_rank(skill, settings, progress);
    ui.label(RichText::new(skill.name()).size(21.0).strong());
    ui.label(RichText::new(format!("{} skill · rank {rank} of {}", skill.group().name(), skill.max_rank())).small().color(group_color(skill.group())));
    if let Some(t) = skill.cast_time() {
        ui.label(RichText::new(format!("Spell · {t:.1} s cast")).small().color(group_color(Group::Arcane)));
    }
    ui.add_space(4.0);
    // Which slot it's in: click a key to put it there.
    if rank > 0 {
        let here = progress.slot_of(skill);
        ui.label(match here {
            Some(i) => RichText::new(format!("In slot {}: press {}", i + 1, keys[i])).color(UNLOCKED),
            None => RichText::new("Not in a slot: pick one").color(WARM),
        });
        ui.label(RichText::new("Press any key or mouse button (not left click) to put it on that key.").small().color(ACCENT));
        if here.is_none()
            && let Some(empty) = progress.slots().iter().position(Option::is_none)
            && ui.button(format!("Equip in empty slot · {}", keys[empty])).clicked()
        {
            let _ = progress.place(skill, empty, settings.unlock_all);
        }
        ui.horizontal_wrapped(|ui| {
            for (i, key) in keys.iter().enumerate() {
                let occupant = progress.slots()[i];
                let selected = here == Some(i);
                let text = RichText::new(key).monospace().strong();
                let button = egui::Button::new(if selected { text.color(Color32::BLACK) } else { text })
                    .fill(if selected { UNLOCKED } else if occupant.is_some() { Color32::from_rgb(45, 45, 60) } else { Color32::from_rgb(30, 36, 48) })
                    .min_size(vec2(34.0, 24.0));
                let tip = match occupant {
                    Some(s) if s == skill => "Already here".to_owned(),
                    Some(s) => format!("Slot {}: replaces {}", i + 1, s.name()),
                    None => format!("Slot {}: empty", i + 1),
                };
                if ui.add(button).on_hover_text(tip).clicked() && !selected {
                    let _ = progress.place(skill, i, settings.unlock_all);
                }
            }
        });
        if here.is_some() && ui.small_button("Take out of its slot").clicked() {
            progress.unequip(skill);
        }
        ui.add_space(6.0);
    }
    for (i, r) in skill.ranks().iter().enumerate() {
        let n = i as u32 + 1;
        let have = n <= rank;
        let title = RichText::new(format!("{n}. {}", r.name)).strong();
        ui.label(if have { title.color(UNLOCKED) } else { title });
        let desc = RichText::new(r.description);
        ui.label(if have || n == rank + 1 { desc } else { desc.weak() });
        ui.add_space(4.0);
    }
    let needs = skill.needs();
    if !needs.is_empty() {
        ui.label(RichText::new("Needs").small().strong().color(Color32::GRAY));
        for &(s, r) in needs {
            let ok = shown_rank(s, settings, progress) >= r;
            let color = if ok { UNLOCKED } else { Color32::from_rgb(230, 110, 100) };
            ui.label(RichText::new(format!("{} {}", if ok { "Have:" } else { "Missing:" }, s.name())).color(color));
        }
        ui.add_space(4.0);
    }
    if rank < skill.max_rank()
        && let Some(t) = skill.trial()
    {
        ui.label(RichText::new(format!("Rank {} needs a trial", skill.max_rank())).small().weak());
        trial_bar(ui, progress, &t);
        ui.add_space(4.0);
    }
    if rank < skill.max_rank() && !settings.unlock_all {
        let cost = skill.cost(rank + 1);
        let label = if rank == 0 { format!("Learn · {}", plural(cost, "point")) } else { format!("Rank up · {}", plural(cost, "point")) };
        let blocker = progress.blocker(skill);
        let button = ui.add_enabled(blocker.is_none(), egui::Button::new(RichText::new(label).strong()).min_size(vec2(180.0, 30.0)));
        if let Some(why) = &blocker {
            ui.label(RichText::new(why).small().color(Color32::from_rgb(230, 150, 100)));
        }
        if button.clicked() {
            progress.upgrade(skill);
        }
    }
}

/// The item tree, as a circle like the skills: gear, movement, weapons,
/// health and building passives, Ascension on the outside.
fn items_tab(ui: &mut egui::Ui, settings: &Settings, progress: &mut Progress, menu: &mut MenuState, moves: &Moveset) {
    use crate::items::{ASCENSION, ITEMS};
    let sandbox = settings.unlock_all;
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("{} scrap", progress.scrap)).size(22.0).color(GOLD));
        ui.add_space(12.0);
        let owned = progress.items().len();
        ui.label(
            RichText::new(format!(
                "{owned} of {} items · Ascension {} · weapons and moves without their own button are items: always on once owned",
                ITEMS.len() - 1,
                progress.ascension
            ))
            .small()
            .weak(),
        );
    });
    ui.separator();
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(560.0);
            let nodes: Vec<GraphNode> = ITEMS
                .iter()
                .enumerate()
                .map(|(i, it)| GraphNode {
                    name: it.name.to_owned(),
                    mark: if i == ASCENSION { progress.ascension.to_string() } else { initials(it.name) },
                    group: it.group,
                    owned: sandbox || progress.has_item(i),
                    available: !sandbox && progress.item_blocker(i).is_none(),
                    capstone: it.trial.is_some() || i == ASCENSION,
                    ranks: None,
                    equipped: false,
                    tooltip: it.description.to_owned(),
                    needs: it.needs.to_vec(),
                    outer: i == ASCENSION,
                })
                .collect();
            if let Some(i) = draw_graph(ui, &nodes, &mut menu.item_view, Some(menu.selected_item), 450.0) {
                menu.selected_item = i;
            }
        });
        ui.separator();
        ui.vertical(|ui| {
            let i = menu.selected_item.min(ITEMS.len() - 1);
            let it = &ITEMS[i];
            ui.label(RichText::new(it.name).size(21.0).strong());
            ui.label(RichText::new(format!("{} item · passive", it.group.name())).small().color(group_color(it.group)));
            ui.add_space(6.0);
            ui.label(it.description);
            ui.add_space(8.0);
            if !it.needs.is_empty() {
                ui.label(RichText::new("Needs").small().strong().color(Color32::GRAY));
                for &n in it.needs {
                    let ok = sandbox || progress.has_item(n);
                    let color = if ok { UNLOCKED } else { Color32::from_rgb(230, 110, 100) };
                    ui.label(RichText::new(format!("{} {}", if ok { "Have:" } else { "Missing:" }, ITEMS[n].name)).color(color));
                }
                ui.add_space(4.0);
            }
            if let Some(t) = &it.trial
                && !progress.has_item(i)
            {
                trial_bar(ui, progress, t);
                ui.add_space(4.0);
            }
            if sandbox {
                ui.label(RichText::new("Sandbox is on").weak());
            } else if i != ASCENSION && progress.has_item(i) {
                ui.label(RichText::new("Owned").strong().color(UNLOCKED));
            } else {
                let cost = progress.item_cost(i);
                let label = if i == ASCENSION { format!("Ascend to {} · {cost} scrap", progress.ascension + 1) } else { format!("Buy · {cost} scrap") };
                let blocker = progress.item_blocker(i);
                let button = ui.add_enabled(blocker.is_none(), egui::Button::new(RichText::new(label).strong()).min_size(vec2(180.0, 30.0)));
                if let Some(why) = &blocker {
                    ui.label(RichText::new(why).small().color(Color32::from_rgb(230, 150, 100)));
                }
                if button.clicked() {
                    progress.buy_item(i);
                }
            }
            // Weapons: everything about them in one table.
            if it.group == Group::Combat {
                ui.add_space(10.0);
                ui.label(RichText::new("Weapons").strong());
                egui::Grid::new("weapon stats").striped(true).spacing([10.0, 5.0]).show(ui, |ui| {
                    for h in ["", "Weapon", "Damage", "/s"] {
                        ui.label(RichText::new(h).small().weak());
                    }
                    ui.end_row();
                    for w in Weapon::ALL {
                        let have = moves.has_weapon(w);
                        key_chip(ui, w.key());
                        let (damage, rate, effect) = w.stats(moves);
                        let text = |s: String| if have { RichText::new(s) } else { RichText::new(s).weak() };
                        ui.label(text(w.name().into())).on_hover_text(effect);
                        ui.label(text(damage));
                        ui.label(text(format!("{rate:.1}")));
                        ui.end_row();
                    }
                });
            }
        });
    });
}

fn ui_color(linear: glam::Vec3) -> Color32 {
    Color32::from(egui::Rgba::from_rgb(linear.x, linear.y, linear.z))
}

/// A colour swatch with a label.
fn swatch(ui: &mut egui::Ui, color: glam::Vec3, label: &str) {
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
        ui.painter().circle_filled(r.center(), 8.0, ui_color(color));
        ui.label(label);
    });
}

fn multiplayer_tab(ui: &mut egui::Ui, menu: &mut MenuState, hud: &Hud) -> Option<MenuAction> {
    let net = &hud.net;
    let mut action = None;
    ui.add_space(6.0);
    ui.label(RichText::new(if net.active { net.status.as_str() } else { "Playing alone" }).size(17.0).color(
        if net.connected { UNLOCKED } else if net.active { WARM } else { Color32::LIGHT_GRAY },
    ));
    if net.active && net.over_steam {
        if net.is_host && !net.connected {
            ui.add_space(4.0);
            ui.label(RichText::new("Invite a Steam friend").strong());
            if net.steam_overlay && ui.button("Open Steam's invite window").clicked() {
                action = Some(MenuAction::InviteSteam);
            }
            let online = net.steam_friends.iter().filter(|f| f.online).count();
            let offline = net.steam_friends.len() - online;
            ui.horizontal(|ui| {
                ui.label("Search:");
                ui.add(egui::TextEdit::singleline(&mut menu.friend_search).desired_width(180.0).hint_text("a friend's name"));
                ui.checkbox(&mut menu.show_offline, format!("Show offline ({offline})"));
            });
            ui.label(RichText::new(format!("{online} online, {offline} offline")).small().weak());
            let search = menu.friend_search.trim().to_lowercase();
            let shown: Vec<&crate::steam::SteamFriend> = net
                .steam_friends
                .iter()
                .filter(|f| (f.online || menu.show_offline || !search.is_empty()) && f.name.to_lowercase().contains(&search))
                .collect();
            if shown.is_empty() {
                ui.label(RichText::new(if search.is_empty() { "None of your Steam friends are online right now." } else { "No friend by that name." }).weak());
            }
            // About half the screen tall (the menu window grows to fit it).
            let height = (ui.ctx().content_rect().height() * 0.45).clamp(260.0, 560.0);
            egui::ScrollArea::vertical().max_height(height).auto_shrink([false, true]).id_salt("steam friends").show(ui, |ui| {
                for f in shown {
                    ui.horizontal(|ui| {
                        let invited = net.invited.contains(&f.id);
                        let button = egui::Button::new(if invited { "Invite again" } else { "Invite" });
                        if ui.add_enabled(f.online, button).on_disabled_hover_text("Offline: Steam can't deliver an invite").clicked() {
                            action = Some(MenuAction::InviteFriend(f.id));
                        }
                        let name = RichText::new(&f.name).strong();
                        ui.label(if f.online { name } else { name.weak() });
                        let color = if f.in_game { UNLOCKED } else { Color32::GRAY };
                        ui.label(RichText::new(if invited { "invited" } else { f.status }).small().color(color));
                    });
                }
            });
            ui.label(
                RichText::new(
                    "They get a Steam chat invite with a Join button. They need the game open first: \
                     friends shown as \"has the game open\" can join straight away.",
                )
                .small()
                .weak(),
            );
        }
        if let Some(name) = &net.peer_name {
            ui.label(RichText::new(format!("Playing with {name}")).color(UNLOCKED));
        }
    }
    if net.active && net.is_host && !net.over_steam {
        // The one thing to send a friend.
        if let Some(code) = &net.join_code {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Join code:").size(17.0));
                ui.label(RichText::new(code).monospace().size(22.0).strong().color(GOLD));
                if ui.button("Copy").clicked() {
                    ui.ctx().copy_text(code.clone());
                }
            });
        }
        let (line, ok) = &net.port_forwarding;
        if !line.is_empty() {
            let color = match ok {
                Some(true) => UNLOCKED,
                Some(false) => WARM,
                None => Color32::LIGHT_GRAY,
            };
            ui.label(RichText::new(line).color(color));
        }
        if let Some(address) = &net.public_address {
            ui.label(RichText::new(format!("(Internet address: {address})")).small().weak());
        }
    }
    ui.add_space(4.0);
    swatch(ui, net.my_color, "Your colour");
    if net.connected {
        swatch(ui, net.other_color, "The other player");
    }
    ui.add_space(10.0);
    if net.active {
        if ui.button("Disconnect").clicked() {
            action = Some(MenuAction::Disconnect);
        }
    } else {
        // Steam first: no codes, no router setup, works from anywhere.
        ui.label(RichText::new("Play through Steam").heading());
        match &net.steam_name {
            Some(name) => {
                ui.horizontal(|ui| {
                    if ui.button(RichText::new("Host through Steam").strong()).clicked() {
                        action = Some(MenuAction::HostSteam);
                    }
                    ui.label(format!("Signed in as {name}."));
                });
                ui.label(
                    RichText::new(
                        "Your friend, with the game open, right-clicks you in their Steam friends list > Join Game \
                         (or accepts your invite). Works from anywhere, no setup. Steam shows the game as \"Spacewar\".",
                    )
                    .small()
                    .weak(),
                );
            }
            None => {
                ui.label(
                    RichText::new("Steam isn't running. Start Steam, then restart the game, for no-setup internet play.")
                        .color(WARM),
                );
            }
        }
        ui.add_space(8.0);
        ui.label(RichText::new("Or connect directly").heading());
        ui.columns(2, |cols| {
            let ui = &mut cols[0];
            ui.label(RichText::new("Host a game").heading());
            ui.label("Others join you. You're the gold player.");
            ui.horizontal(|ui| {
                ui.label("Port");
                ui.add(egui::TextEdit::singleline(&mut menu.host_port).desired_width(70.0));
            });
            let port = menu.host_port.trim().parse::<u16>().ok();
            if ui.add_enabled(port.is_some(), egui::Button::new("Host")).clicked()
                && let Some(port) = port
            {
                action = Some(MenuAction::Host(port));
            }
            ui.label(RichText::new("You'll get a join code to send your friend. Allow the game through Windows Firewall if asked.").small().weak());

            let ui = &mut cols[1];
            ui.label(RichText::new("Join a game").heading());
            ui.label("Paste the host's join code (or an address). You're the teal player.");
            ui.horizontal(|ui| {
                ui.label("Code");
                ui.add(egui::TextEdit::singleline(&mut menu.join_address).hint_text("K7M2-QX4P-AB9C-D3FH").desired_width(190.0));
            });
            let ready = !menu.join_address.trim().is_empty();
            if ui.add_enabled(ready, egui::Button::new("Join")).clicked() {
                action = Some(MenuAction::Join(menu.join_address.trim().to_owned()));
            }
        });
    }
    ui.add_space(6.0);
    egui::CollapsingHeader::new("Playing over the internet").show(ui, |ui| {
        for line in [
            "1. Host, then send your friend the join code. Most home routers let the game in automatically (UPnP): the line under the code says so.",
            "2. If it says only your network can join: turn on UPnP in your router's settings, or forward UDP port 7777 to this PC.",
            "3. If your provider shares addresses (CGNAT), or the router can't be changed: both install Tailscale (free, tailscale.com), sign in to the same account or share the machine, and join with the host's Tailscale address, like 100.101.102.103:7777.",
            "4. Windows Firewall: when it asks, allow the game (private and public networks). Both players need the same game version.",
        ] {
            ui.label(RichText::new(line).small());
        }
        if let Some(ip) = &net.lan_ip {
            ui.label(RichText::new(format!("This PC on your network: {ip}:{}", menu.host_port.trim())).small().weak());
        }
    });
    ui.separator();
    if let Some(m) = &hud.match_view {
        ui.label(RichText::new(format!("Round {} · Gold {} : {} Teal", m.round, m.scores[0], m.scores[1])).strong());
        ui.label(RichText::new("Each round: 15 s exploring from a random spot, then fight until one player falls.").small().weak());
    }
    ui.horizontal(|ui| {
        let label = if menu.confirm_reset { "Click again to reset everyone" } else { "Reset all players" };
        let button = ui.button(RichText::new(label).color(if menu.confirm_reset { Color32::from_rgb(255, 120, 100) } else { Color32::WHITE }));
        if button.clicked() {
            if menu.confirm_reset {
                action = Some(MenuAction::ResetAll);
            }
            menu.confirm_reset = !menu.confirm_reset;
        }
        ui.label(
            RichText::new("Every player's abilities, items, XP and scrap back to the start, and the score to 0-0 (a backup is kept).")
                .small()
                .weak(),
        );
    });
    ui.separator();
    match &hud.update {
        Some((status, ready)) => {
            ui.horizontal(|ui| {
                ui.label(status);
                if *ready && ui.button("Restart now").clicked() {
                    action = Some(MenuAction::Restart);
                }
            });
        }
        None => {
            ui.label(RichText::new(format!("Version {} (this copy doesn't auto-update)", crate::updater::CURRENT)).small().weak());
        }
    }
    ui.add_space(8.0);
    ui.label(
        RichText::new(
            "Two players. The host sends the join code; on one computer you can also join 127.0.0.1:7777. \
             Shared: positions, shots and the drones (the host runs them). \
             Each player keeps their own abilities and progress.",
        )
        .small()
        .weak(),
    );
    action
}

fn settings_tab(ui: &mut egui::Ui, settings: &mut Settings) {
    ui.columns(2, |cols| {
        section(&mut cols[0], "Controls");
        egui::Grid::new("controls").num_columns(2).min_col_width(LABEL_WIDTH).spacing([16.0, 8.0]).show(&mut cols[0], |ui| {
            ui.label("Mouse sensitivity");
            ui.add(Slider::new(&mut settings.mouse_sensitivity, range::MOUSE_SENSITIVITY).fixed_decimals(2));
            ui.end_row();
            ui.label("Invert mouse Y");
            ui.checkbox(&mut settings.invert_y, "");
            ui.end_row();
            ui.label("Field of view");
            ui.add(Slider::new(&mut settings.fov_degrees, range::FOV).suffix("°").fixed_decimals(0));
            ui.end_row();
            ui.label("Head bob").on_hover_text("The camera sways as you walk and dips when you land");
            ui.checkbox(&mut settings.head_bob, "");
            ui.end_row();
            ui.label("Speed widens view").on_hover_text("The field of view opens up a little at high speed");
            ui.checkbox(&mut settings.speed_fov, "");
            ui.end_row();
            ui.label("Screen shake");
            ui.add(Slider::new(&mut settings.screen_shake, range::SCREEN_SHAKE).fixed_decimals(2));
            ui.end_row();
        });
        section(&mut cols[0], "World");
        egui::Grid::new("world").num_columns(2).min_col_width(LABEL_WIDTH).spacing([16.0, 8.0]).show(&mut cols[0], |ui| {
            ui.label("Brightness");
            ui.add(Slider::new(&mut settings.brightness, range::BRIGHTNESS).fixed_decimals(2));
            ui.end_row();
            ui.label("Sun height");
            ui.add(Slider::new(&mut settings.sun_elevation, range::SUN_ELEVATION).suffix("°").fixed_decimals(0));
            ui.end_row();
            ui.label("Sun direction");
            ui.add(Slider::new(&mut settings.sun_azimuth, range::SUN_AZIMUTH).suffix("°").fixed_decimals(0));
            ui.end_row();
            ui.label("Wind");
            ui.add(Slider::new(&mut settings.wind, range::WIND).fixed_decimals(2));
            ui.end_row();
            ui.label("Drones").on_hover_text("How hard the drones fight back");
            ui.horizontal(|ui| {
                for (value, name) in [(0.0, "Peaceful"), (1.0, "Normal"), (2.0, "Hard")] {
                    ui.selectable_value(&mut settings.difficulty, value, name);
                }
            });
            ui.end_row();
        });

        section(&mut cols[1], "Graphics");
        egui::Grid::new("graphics").num_columns(2).min_col_width(LABEL_WIDTH).spacing([16.0, 8.0]).show(&mut cols[1], |ui| {
            ui.label("V-Sync");
            ui.checkbox(&mut settings.vsync, "");
            ui.end_row();
            ui.label("3D grass");
            ui.checkbox(&mut settings.grass, "");
            ui.end_row();
            ui.label("Grass distance");
            ui.add_enabled(
                settings.grass,
                Slider::new(&mut settings.grass_distance, range::GRASS_DISTANCE).suffix(" m").fixed_decimals(0),
            );
            ui.end_row();
            ui.label("Grass density");
            ui.add_enabled(settings.grass, Slider::new(&mut settings.grass_density, range::GRASS_DENSITY).fixed_decimals(2));
            ui.end_row();
            ui.label("Surface detail").on_hover_text("Parallax depth on bricks, cobbles, roof tiles and crates");
            ui.checkbox(&mut settings.surface_detail, "");
            ui.end_row();
            ui.label("View distance");
            ui.add(Slider::new(&mut settings.view_distance, range::VIEW_DISTANCE).suffix(" m").fixed_decimals(0));
            ui.end_row();
            ui.label("Show FPS");
            ui.checkbox(&mut settings.show_fps, "");
            ui.end_row();
        });
        section(&mut cols[1], "Audio");
        egui::Grid::new("audio").num_columns(2).min_col_width(LABEL_WIDTH).spacing([16.0, 8.0]).show(&mut cols[1], |ui| {
            for (label, value) in [
                ("Master volume", &mut settings.master_volume),
                ("Sound effects", &mut settings.sfx_volume),
                ("Music", &mut settings.music_volume),
                ("Ambience", &mut settings.ambient_volume),
            ] {
                ui.label(label);
                let mut percent = *value * 100.0;
                if ui.add(Slider::new(&mut percent, 0.0..=100.0).suffix("%").fixed_decimals(0)).changed() {
                    *value = percent / 100.0;
                }
                ui.end_row();
            }
        });
    });
}

fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(6.0);
    ui.label(RichText::new(title).heading());
    ui.add_space(2.0);
}

// ---------------------------------------------------------------- HUD

/// Text with a dark drop shadow, readable over any background.
fn shadow_text(painter: &egui::Painter, pos: Pos2, align: Align2, text: &str, font: FontId, color: Color32) {
    painter.text(pos + vec2(1.0, 1.0), align, text, font.clone(), Color32::from_black_alpha(200));
    painter.text(pos, align, text, font, color);
}

fn draw_hud(ctx: &egui::Context, settings: &Settings, progress: &Progress, hud: &Hud, fps: f32) {
    let painter = ctx.layer_painter(LayerId::new(Order::Foreground, Id::unique("hud")));
    let screen = ctx.content_rect();

    let fx = &hud.fx;
    // Edges: streaks at speed, red when hurt or low on health.
    if fx.speed > 0.02 {
        speed_streaks(&painter, screen, fx.speed, fx.time);
    }
    let mut hurt = fx.damage.clamp(0.0, 1.0) * 0.75;
    if fx.low_health {
        hurt = hurt.max(0.25 + 0.15 * (fx.time * 5.0).sin());
    }
    if hurt > 0.01 {
        let depth = screen.width().min(screen.height()) * 0.22;
        vignette(&painter, screen, Color32::from_rgba_unmultiplied(200, 20, 15, (hurt * 200.0) as u8), depth);
    }

    // Crosshair: a dot and four ticks with a dark outline, so it shows on
    // bright and dark surfaces. Ticks spread out briefly on a hit.
    let c = screen.center();
    let spread = 4.0 + 5.0 * fx.hit.max(fx.kill);
    for (width, color) in [(4.0, Color32::from_black_alpha(130)), (2.0, Color32::from_white_alpha(235))] {
        let stroke = Stroke::new(width, color);
        for d in [vec2(1.0, 0.0), vec2(-1.0, 0.0), vec2(0.0, 1.0), vec2(0.0, -1.0)] {
            painter.line_segment([c + d * spread, c + d * (spread + 6.0)], stroke);
        }
    }
    painter.circle_filled(c, 1.6, Color32::from_white_alpha(235));
    // Hit marker: diagonal ticks, red for a kill.
    let marker = fx.hit.max(fx.kill);
    if marker > 0.01 {
        let color = if fx.kill > 0.01 { Color32::from_rgb(255, 80, 60) } else { Color32::WHITE };
        let size = if fx.kill > 0.01 { 15.0 } else { 12.0 };
        for (width, col) in [(4.5, Color32::from_black_alpha((120.0 * marker) as u8)), (2.5, color.gamma_multiply(marker))] {
            for d in [vec2(1.0, 1.0), vec2(-1.0, 1.0), vec2(1.0, -1.0), vec2(-1.0, -1.0)] {
                painter.line_segment([c + d * 6.0, c + d * size], Stroke::new(width, col));
            }
        }
    }

    if settings.show_fps {
        // Not measured yet during the first half second.
        let text = if fps > 0.0 { format!("{fps:.0} FPS") } else { "-- FPS".to_owned() };
        shadow_text(&painter, screen.left_top() + vec2(12.0, 10.0), Align2::LEFT_TOP, &text, FontId::monospace(15.0), Color32::WHITE);
    }

    draw_progress(&painter, screen, progress);
    if hud.net.connected {
        // Who's here, under the FPS counter.
        let pos = screen.left_top() + vec2(14.0, 38.0);
        painter.circle_filled(pos + vec2(5.0, 7.0), 5.0, ui_color(hud.net.my_color));
        painter.circle_filled(pos + vec2(19.0, 7.0), 5.0, ui_color(hud.net.other_color));
        shadow_text(&painter, pos + vec2(30.0, 0.0), Align2::LEFT_TOP, "2 players", FontId::proportional(13.0), Color32::WHITE);
    } else if hud.net.active {
        let pos = screen.left_top() + vec2(14.0, 38.0);
        shadow_text(&painter, pos, Align2::LEFT_TOP, "Waiting for the other player...", FontId::proportional(13.0), WARM);
    }
    let top = draw_hotbar(&painter, screen, &hud.slots);
    let top = draw_weapons(&painter, screen, &hud.weapons, top);
    if let Some(v) = &hud.vitals {
        draw_vitals(&painter, screen, v, fx.time, top);
    }
    draw_meters(&painter, screen, &hud.meters, screen.bottom() - 16.0);

    if let Some(m) = &hud.match_view {
        draw_scoreboard(&painter, screen, m);
    }
    for b in &hud.drone_bars {
        draw_drone_bar(&painter, screen, b);
    }
    for p in &hud.popups {
        draw_popup(&painter, screen, p);
    }
    if let Some((ndc, health)) = hud.enemy {
        // A bar over the other player's head.
        let p = Pos2::new(screen.left() + (ndc.x * 0.5 + 0.5) * screen.width(), screen.top() + (0.5 - ndc.y * 0.5) * screen.height());
        let bar = Rect::from_center_size(p, vec2(70.0, 7.0));
        painter.rect_filled(bar.expand(1.5), 3.0, PANEL);
        painter.rect_filled(Rect::from_min_size(bar.min, vec2(bar.width() * health.clamp(0.0, 1.0), bar.height())), 2.0, ui_color(hud.net.other_color).gamma_multiply(1.4));
    }
    if hud.dead {
        let c = screen.center() + vec2(0.0, -70.0);
        shadow_text(&painter, c, Align2::CENTER_CENTER, "DEFEATED", FontId::proportional(40.0), Color32::from_rgb(230, 80, 70));
        shadow_text(&painter, c + vec2(0.0, 36.0), Align2::CENTER_CENTER, "Waiting for the next round...", FontId::proportional(16.0), Color32::WHITE);
    }

    // Notices, under the scoreboard: a pill that drops in and fades out.
    if let Some((toast, age, left)) = hud.toast {
        let a = (age / 0.18).clamp(0.0, 1.0).min((left / 0.4).clamp(0.0, 1.0));
        let slide = (1.0 - (age / 0.25).clamp(0.0, 1.0)).powi(2) * -12.0;
        let galley = painter.layout_no_wrap(toast.to_owned(), FontId::proportional(19.0), Color32::from_rgb(170, 230, 255).gamma_multiply(a));
        let top = if hud.match_view.is_some() { 84.0 } else { 70.0 };
        let rect = Rect::from_center_size(screen.center_top() + vec2(0.0, top + slide + galley.size().y * 0.5 + 8.0), galley.size() + vec2(36.0, 16.0));
        painter.rect_filled(rect, rect.height() * 0.5, Color32::from_black_alpha((150.0 * a) as u8));
        painter.rect_stroke(rect, rect.height() * 0.5, Stroke::new(1.0, ACCENT.gamma_multiply(0.35 * a)), StrokeKind::Inside);
        painter.galley(rect.center() - galley.size() * 0.5, galley, Color32::WHITE);
    }

    // Casting: a bar under the crosshair that fills, with a glow round it.
    if let Some(cast) = &fx.cast {
        let t = cast.progress.clamp(0.0, 1.0);
        draw_rune_sheet(&painter, screen.center(), t, fx.time);
        let c = screen.center() + vec2(0.0, 70.0);
        let bar = Rect::from_center_size(c, vec2(240.0, 12.0));
        let magic = Color32::from_rgb(255, 120, 215);
        painter.rect_filled(bar.expand(3.0), 6.0, Color32::from_black_alpha(170));
        painter.rect_filled(Rect::from_min_size(bar.min, vec2(bar.width() * t, bar.height())), 4.0, magic);
        // Overcharge: a gold bar filling over the full one.
        if cast.overcharge > 0.0 {
            let gold = Rect::from_min_size(bar.min, vec2(bar.width() * cast.overcharge, bar.height()));
            painter.rect_filled(gold, 4.0, GOLD.gamma_multiply(0.7 + 0.3 * (fx.time * 14.0).sin().abs()));
        }
        painter.rect_stroke(bar.expand(3.0), 6.0, Stroke::new(1.0, magic.gamma_multiply(0.5 + 0.5 * (fx.time * 8.0).sin().abs())), StrokeKind::Outside);
        let title = if cast.overcharge > 0.0 {
            format!("Overcharging {} · {:.0}%", cast.name, 100.0 + cast.overcharge * 100.0)
        } else {
            format!("Casting {}", cast.name)
        };
        shadow_text(&painter, c + vec2(0.0, -16.0), Align2::CENTER_BOTTOM, &title, FontId::proportional(15.0), if cast.overcharge > 0.0 { GOLD } else { Color32::WHITE });
        let hint = if cast.rune {
            "aim now · hold the key to overcharge · then press it again to cast"
        } else {
            "hold the key to overcharge"
        };
        shadow_text(&painter, c + vec2(0.0, 12.0), Align2::CENTER_TOP, hint, FontId::proportional(11.5), Color32::from_gray(190));
        painter.circle_stroke(screen.center(), 18.0 + 10.0 * t + 8.0 * cast.overcharge, Stroke::new(2.0, if cast.overcharge > 0.0 { GOLD } else { magic.gamma_multiply(0.6) }));
    }
    if !fx.runes.is_empty() {
        draw_runes(&painter, screen, &fx.runes, fx.time);
    }
    if let Some(p) = &fx.pending {
        draw_pending(&painter, screen, p, fx.time);
    }
    if let Some(b) = &fx.banner {
        draw_banner(&painter, screen, b);
    }
    if !fx.goals.is_empty() && fx.tutorial.is_none() {
        draw_goals(&painter, screen, &fx.goals, if hud.net.active { 60.0 } else { 38.0 });
    }
    if let Some(t) = &fx.tutorial {
        draw_tutorial(&painter, screen, t, fx.time);
    }
}

/// The runes waiting to go off: a chip each with its fuse burning down,
/// and how to set them off.
fn draw_runes(painter: &egui::Painter, screen: Rect, runes: &[(String, f32, f32)], time: f32) {
    let y = screen.center().y + 168.0;
    let w = 132.0;
    let total = runes.len() as f32 * (w + 8.0) - 8.0;
    let mut x = screen.center().x - total * 0.5;
    for (name, fuse, boost) in runes {
        let chip = Rect::from_min_size(Pos2::new(x, y), vec2(w, 34.0));
        let urgent = *fuse < 0.15 && (time * 10.0).sin() > 0.0;
        painter.rect_filled(chip, 6.0, Color32::from_black_alpha(170));
        painter.rect_stroke(chip, 6.0, Stroke::new(1.5, if urgent { WARM } else { ACCENT }), StrokeKind::Inside);
        let label = if *boost > 1.05 { format!("{name} {:.0}%", boost * 100.0) } else { name.clone() };
        shadow_text(painter, chip.left_top() + vec2(8.0, 9.0), Align2::LEFT_CENTER, &label, FontId::proportional(12.0), Color32::WHITE);
        let fuse_bar = Rect::from_min_size(chip.left_bottom() + vec2(8.0, -10.0), vec2((w - 16.0) * fuse.clamp(0.0, 1.0), 4.0));
        painter.rect_filled(fuse_bar, 2.0, if urgent { WARM } else { GOLD });
        x += w + 8.0;
    }
    let s = if runes.len() == 1 { "" } else { "s" };
    shadow_text(
        painter,
        Pos2::new(screen.center().x, y + 44.0),
        Align2::CENTER_TOP,
        &format!("{} spell{s} ready · press a spell's key to cast it · overcharge fades · Alt: all together", runes.len()),
        FontId::proportional(12.0),
        Color32::from_gray(210),
    );
}

/// The prompt for a half-placed ability: its name and step, what the next
/// press does, and a bar running down until it's cancelled.
fn draw_pending(painter: &egui::Painter, screen: Rect, p: &Pending, time: f32) {
    let c = screen.center() + vec2(0.0, 112.0);
    let card = Rect::from_center_size(c, vec2(320.0, 52.0));
    let pulse = 0.55 + 0.45 * (time * 4.0).sin().abs();
    painter.rect_filled(card, 8.0, Color32::from_black_alpha(175));
    painter.rect_stroke(card, 8.0, Stroke::new(1.5, ACCENT.gamma_multiply(pulse)), StrokeKind::Outside);
    // Step pips: filled for the steps done.
    let pips = p.step.1.max(1);
    for i in 0..pips {
        let at = card.left_top() + vec2(16.0 + i as f32 * 12.0, 15.0);
        if i < p.step.0 {
            painter.circle_filled(at, 4.0, ACCENT);
        } else {
            painter.circle_stroke(at, 4.0, Stroke::new(1.2, ACCENT.gamma_multiply(pulse)));
        }
    }
    let title = format!("{}  ·  step {} of {}", p.title.to_uppercase(), (p.step.0 + 1).min(pips), pips);
    let after_pips = card.left_top() + vec2(16.0 + pips as f32 * 12.0 + 2.0, 15.0);
    shadow_text(painter, after_pips, Align2::LEFT_CENTER, &title, FontId::proportional(14.0), Color32::WHITE);
    shadow_text(painter, card.left_top() + vec2(12.0, 34.0), Align2::LEFT_CENTER, &p.hint, FontId::proportional(13.0), Color32::from_gray(215));
    let warn = if p.left < 0.25 { Color32::from_rgb(255, 120, 90) } else { ACCENT };
    let bar = Rect::from_min_size(card.left_bottom() + vec2(6.0, -6.0), vec2((card.width() - 12.0) * p.left.clamp(0.0, 1.0), 3.0));
    painter.rect_filled(bar, 1.5, warn);
}

/// Eight angular runes form a small procedural sprite sheet. During a cast,
/// they light in sequence around the crosshair while the active glyph turns.
fn draw_rune_sheet(painter: &egui::Painter, center: Pos2, progress: f32, time: f32) {
    const GLYPHS: [[usize; 6]; 8] = [
        [0, 8, 4, 8, 2, 0], [6, 8, 1, 8, 5, 6], [0, 3, 6, 1, 4, 0], [2, 7, 4, 1, 6, 2],
        [0, 5, 2, 7, 4, 1], [6, 3, 0, 2, 5, 7], [1, 4, 7, 3, 0, 6], [0, 2, 5, 1, 6, 4],
    ];
    const POINTS: [egui::Vec2; 9] = [
        vec2(0.0, -1.0), vec2(0.7, -0.7), vec2(1.0, 0.0), vec2(0.7, 0.7),
        vec2(0.0, 1.0), vec2(-0.7, 0.7), vec2(-1.0, 0.0), vec2(-0.7, -0.7), vec2(0.0, 0.0),
    ];
    let active = ((progress * 8.0).floor() as usize).min(7);
    let rotation = time * 0.55;
    let pulse = 0.82 + 0.18 * (time * 7.0).sin().abs();
    let radius = 76.0 * pulse;
    let grey = Color32::from_rgba_unmultiplied(145, 145, 155, 150);
    let pink = Color32::from_rgba_unmultiplied(255, 105, 205, 225);
    let bright = Color32::from_rgba_unmultiplied(255, 185, 230, 250);
    let ring_color = grey.lerp_to_gamma(pink, progress);
    painter.circle_stroke(center, radius + 20.0, Stroke::new(1.4, ring_color));
    painter.circle_stroke(center, radius - 20.0, Stroke::new(1.2, ring_color.gamma_multiply(0.8)));

    for i in 0..8 {
        let angle = rotation + i as f32 * std::f32::consts::TAU / 8.0;
        let dir = vec2(angle.cos(), angle.sin());
        let right = vec2(-dir.y, dir.x);
        let glyph_center = center + dir * radius;
        let lit = i < active || progress >= 1.0;
        let color = if i == active { bright } else if lit { pink } else { grey };
        let scale = if i == active { 17.0 } else { 12.0 };
        for pair in GLYPHS[i].windows(2) {
            let a = POINTS[pair[0]];
            let b = POINTS[pair[1]];
            let map = |p: egui::Vec2| glyph_center + (dir * p.x + right * p.y) * scale;
            painter.line_segment([map(a), map(b)], Stroke::new(if i == active { 1.8 } else { 1.1 }, color));
        }
        painter.circle_filled(glyph_center, if i == active { 2.8 } else { 2.2 }, color);
    }

    // The centre sigil resolves as the cast approaches completion.
    let core = 16.0 + progress * 12.0;
    painter.circle_stroke(center, core, Stroke::new(2.0, pink.gamma_multiply(0.45 + progress * 0.55)));
    painter.line_segment([center + vec2(-core * 0.55, core * 0.55), center + vec2(core * 0.55, -core * 0.55)], Stroke::new(2.5, bright));
    painter.line_segment([center + vec2(-core * 0.55, -core * 0.55), center + vec2(core * 0.55, core * 0.55)], Stroke::new(1.8, pink));
}

/// Faint white streaks rushing past the edges of the screen at speed.
fn speed_streaks(painter: &egui::Painter, screen: Rect, speed: f32, time: f32) {
    let c = screen.center();
    let r = screen.width().hypot(screen.height()) * 0.5;
    for i in 0..36 {
        // A fixed scatter of angles; each streak flickers on its own phase.
        let seed = (i as f32 * 12.9898).sin() * 43758.547;
        let angle = seed.fract().abs() * std::f32::consts::TAU;
        let phase = (time * (2.0 + (seed * 0.37).fract().abs() * 3.0) + seed).fract();
        let a = (1.0 - phase) * speed;
        if a < 0.05 {
            continue;
        }
        let dir = vec2(angle.cos(), angle.sin());
        let start = 0.55 + 0.35 * phase;
        let len = 0.12 + 0.2 * speed;
        painter.line_segment(
            [c + dir * r * start, c + dir * r * (start + len)],
            Stroke::new(1.5, Color32::from_white_alpha((70.0 * a) as u8)),
        );
    }
}

fn draw_banner(painter: &egui::Painter, screen: Rect, b: &Banner) {
    let fade_in = (b.age / 0.15).clamp(0.0, 1.0);
    let fade_out = ((b.life - b.age) / 0.5).clamp(0.0, 1.0);
    let a = fade_in.min(fade_out);
    if a <= 0.0 {
        return;
    }
    // Pops in slightly larger, then settles.
    let pop = 1.0 + 0.25 * (1.0 - (b.age / 0.2).clamp(0.0, 1.0)).powi(2);
    let center = Pos2::new(screen.center().x, screen.top() + screen.height() * 0.3);
    band(painter, Rect::from_center_size(center + vec2(0.0, 8.0), vec2(720.0, 104.0)), a);
    let title = FontId::proportional(46.0 * pop);
    painter.text(center + vec2(0.0, 3.0), Align2::CENTER_CENTER, &b.title, title.clone(), Color32::from_black_alpha((160.0 * a) as u8));
    painter.text(center, Align2::CENTER_CENTER, &b.title, title, b.color.gamma_multiply(a));
    // A thin line under the title that grows outwards.
    let w = 220.0 * (b.age / 0.35).clamp(0.0, 1.0);
    painter.rect_filled(Rect::from_center_size(center + vec2(0.0, 30.0), vec2(w, 2.0)), 1.0, b.color.gamma_multiply(0.7 * a));
    if !b.subtitle.is_empty() {
        shadow_text(
            painter,
            center + vec2(0.0, 48.0),
            Align2::CENTER_CENTER,
            &b.subtitle,
            FontId::proportional(18.0),
            Color32::from_white_alpha((235.0 * a) as u8),
        );
    }
}

/// Top left, under the FPS counter: what to do next.
fn draw_goals(painter: &egui::Painter, screen: Rect, goals: &[String], top: f32) {
    let mut y = screen.top() + top;
    shadow_text(painter, Pos2::new(screen.left() + 14.0, y), Align2::LEFT_TOP, "GOALS", FontId::proportional(11.0), GOLD);
    y += 16.0;
    for goal in goals {
        painter.circle_filled(Pos2::new(screen.left() + 18.0, y + 8.0), 2.5, GOLD);
        shadow_text(painter, Pos2::new(screen.left() + 28.0, y), Align2::LEFT_TOP, goal, FontId::proportional(14.0), Color32::from_gray(235));
        y += 20.0;
    }
}

/// The tutorial card, top centre: which step, what to do and the keys for
/// it, how far along, and how to skip.
fn draw_tutorial(painter: &egui::Painter, screen: Rect, t: &crate::tutorial::TutorialView, time: f32) {
    let rect = Rect::from_center_size(Pos2::new(screen.center().x, screen.top() + 150.0), vec2(460.0, 132.0));
    painter.rect_filled(rect, 12.0, Color32::from_rgba_unmultiplied(14, 18, 26, 225));
    let done = t.step.0 == 0;
    let edge = if done { UNLOCKED } else { ACCENT };
    painter.rect_stroke(rect, 12.0, Stroke::new(1.5, edge.gamma_multiply(0.7)), StrokeKind::Inside);
    let x = rect.left() + 22.0;
    let label = if done { "TUTORIAL COMPLETE".to_owned() } else { format!("TUTORIAL  ·  STEP {} OF {}", t.step.0, t.step.1) };
    painter.text(Pos2::new(x, rect.top() + 14.0), Align2::LEFT_TOP, label, FontId::proportional(12.0), edge);
    painter.text(Pos2::new(x, rect.top() + 32.0), Align2::LEFT_TOP, t.title, FontId::proportional(24.0), Color32::WHITE);
    // The keys, as chips, on the right of the title.
    let mut kx = rect.right() - 18.0;
    for key in t.keys.iter().rev() {
        let galley = painter.layout_no_wrap((*key).to_owned(), FontId::proportional(13.0), Color32::WHITE);
        let w = galley.size().x.max(16.0) + 14.0;
        let chip = Rect::from_min_size(Pos2::new(kx - w, rect.top() + 36.0), vec2(w, 22.0));
        painter.rect_filled(chip, 5.0, Color32::from_rgb(42, 52, 70));
        painter.rect_stroke(chip, 5.0, Stroke::new(1.0, Color32::from_white_alpha(50)), StrokeKind::Inside);
        painter.galley(chip.center() - galley.size() * 0.5, galley, Color32::WHITE);
        kx -= w + 6.0;
    }
    let hint = painter.layout(t.hint.to_owned(), FontId::proportional(14.0), Color32::from_gray(210), rect.width() - 44.0);
    painter.galley(Pos2::new(x, rect.top() + 66.0), hint, Color32::from_gray(210));
    if !done {
        let bar = Rect::from_min_size(Pos2::new(x, rect.bottom() - 16.0), vec2(rect.width() - 150.0, 5.0));
        painter.rect_filled(bar, 2.5, Color32::from_white_alpha(25));
        painter.rect_filled(Rect::from_min_size(bar.min, vec2(bar.width() * t.progress, bar.height())), 2.5, ACCENT);
        let pulse = 0.55 + 0.35 * (time * 2.5).sin();
        painter.text(
            Pos2::new(rect.right() - 18.0, rect.bottom() - 13.0),
            Align2::RIGHT_CENTER,
            "Enter: skip",
            FontId::proportional(12.0),
            Color32::from_gray(180).gamma_multiply(pulse),
        );
    }
}

/// Top centre: the score, round and what's happening.
fn draw_scoreboard(painter: &egui::Painter, screen: Rect, m: &MatchView) {
    use crate::arena::Phase;
    use crate::net::PLAYER_COLORS;
    let rect = Rect::from_center_size(screen.center_top() + vec2(0.0, 40.0), vec2(300.0, 60.0));
    painter.rect_filled(rect, 8.0, PANEL);
    let score_font = FontId::proportional(28.0);
    for (i, side) in [(0usize, -1.0f32), (1, 1.0)] {
        let x = rect.center().x + side * 90.0;
        let color = ui_color(PLAYER_COLORS[i]).gamma_multiply(1.5);
        painter.text(Pos2::new(x, rect.top() + 6.0), Align2::CENTER_TOP, m.scores[i].to_string(), score_font.clone(), color);
        let who = if i == m.me { "you" } else { "them" };
        painter.text(Pos2::new(x, rect.top() + 40.0), Align2::CENTER_TOP, who, FontId::proportional(11.0), Color32::GRAY);
    }
    painter.text(rect.center_top() + vec2(0.0, 6.0), Align2::CENTER_TOP, format!("Round {}", m.round), FontId::proportional(13.0), Color32::LIGHT_GRAY);
    let (text, color) = match m.phase {
        Phase::Waiting => ("Waiting".to_owned(), Color32::GRAY),
        Phase::Explore => (format!("Explore {:.0}", m.time_left.ceil()), WARM),
        Phase::Fight => ("FIGHT!".to_owned(), Color32::from_rgb(255, 90, 80)),
        Phase::Over => match m.winner {
            Some(w) if w as usize == m.me => ("You win!".to_owned(), UNLOCKED),
            Some(_) => ("They win".to_owned(), Color32::from_rgb(255, 120, 100)),
            None => ("Round over".to_owned(), Color32::GRAY),
        },
    };
    painter.text(rect.center_top() + vec2(0.0, 26.0), Align2::CENTER_TOP, text, FontId::proportional(17.0), color);
}

/// Top right: skill points and experience.
fn draw_progress(painter: &egui::Painter, screen: Rect, progress: &Progress) {
    let rect = Rect::from_min_size(screen.right_top() + vec2(-234.0, 12.0), vec2(220.0, 44.0));
    painter.text(rect.right_bottom() + vec2(-2.0, 16.0), Align2::RIGHT_TOP, format!("{} scrap", progress.scrap), FontId::proportional(13.0), GOLD);
    painter.rect_filled(rect, 6.0, PANEL);
    let (text, color) = if progress.points > 0 {
        (format!("{} skill point{} · Esc", progress.points, if progress.points == 1 { "" } else { "s" }), WARM)
    } else {
        ("Skill points: 0".to_owned(), Color32::LIGHT_GRAY)
    };
    painter.text(rect.left_top() + vec2(10.0, 6.0), Align2::LEFT_TOP, text, FontId::proportional(14.0), color);
    let bar = Rect::from_min_size(rect.left_top() + vec2(10.0, 28.0), vec2(200.0, 7.0));
    painter.rect_filled(bar, 3.0, Color32::from_white_alpha(25));
    let fill = (progress.xp / progress.xp_needed()).clamp(0.0, 1.0);
    painter.rect_filled(Rect::from_min_size(bar.min, vec2(bar.width() * fill, bar.height())), 3.0, ACCENT);
}

const TILE: f32 = 56.0;
const GAP: f32 = 6.0;

/// Bottom centre: one tile per ability you can trigger. Returns the top
/// of the bar (for stacking the weapon bar above it).
fn draw_hotbar(painter: &egui::Painter, screen: Rect, slots: &[Slot]) -> f32 {
    let bottom = screen.bottom() - 14.0;
    if slots.is_empty() {
        return bottom;
    }
    let width = slots.len() as f32 * (TILE + GAP) - GAP;
    let mut x = screen.center().x - width * 0.5;
    let y = bottom - TILE;
    for slot in slots {
        let rect = Rect::from_min_size(Pos2::new(x, y), vec2(TILE, TILE));
        draw_tile(painter, rect, slot);
        x += TILE + GAP;
    }
    y
}

fn draw_tile(painter: &egui::Painter, rect: Rect, slot: &Slot) {
    painter.rect_filled(rect, 8.0, PANEL);
    let (border, text) = match slot.state {
        SlotState::Ready => (Stroke::new(1.0, Color32::from_white_alpha(70)), Color32::WHITE),
        SlotState::Active(_) => (Stroke::new(2.5, ACCENT), ACCENT),
        SlotState::Cooldown(_) => (Stroke::new(1.0, Color32::from_white_alpha(30)), Color32::from_gray(160)),
        SlotState::Unavailable => (Stroke::new(1.0, Color32::from_white_alpha(20)), Color32::from_gray(110)),
    };
    // Cooldown: a dark shutter that lowers as it recharges.
    if let SlotState::Cooldown(left) = slot.state {
        let h = rect.height() * left.clamp(0.0, 1.0);
        painter.rect_filled(Rect::from_min_size(rect.min, vec2(rect.width(), h)), 8.0, Color32::from_black_alpha(140));
    }
    // Active with a timer: a bar along the bottom that runs down.
    if let SlotState::Active(left) = slot.state
        && left < 1.0
    {
        let bar = Rect::from_min_size(rect.left_bottom() + vec2(4.0, -6.0), vec2((rect.width() - 8.0) * left.max(0.0), 3.0));
        painter.rect_filled(bar, 1.5, ACCENT);
    }
    painter.rect_stroke(rect, 8.0, border, StrokeKind::Inside);
    let key_font = FontId::proportional(if slot.key.len() > 2 { 13.0 } else { 18.0 });
    painter.text(rect.left_top() + vec2(7.0, 5.0), Align2::LEFT_TOP, &slot.key, key_font, text);
    painter.text(rect.center_bottom() + vec2(0.0, -9.0), Align2::CENTER_BOTTOM, &slot.name, FontId::proportional(11.0), text);
    if let Some((have, max)) = slot.charges {
        // Pips if few, a number if many.
        if max <= 5 {
            for i in 0..max {
                let c = rect.right_top() + vec2(-8.0 - i as f32 * 8.0, 9.0);
                if i < have {
                    painter.circle_filled(c, 2.8, text);
                } else {
                    painter.circle_stroke(c, 2.8, Stroke::new(1.0, Color32::from_gray(110)));
                }
            }
        } else {
            painter.text(rect.right_top() + vec2(-6.0, 5.0), Align2::RIGHT_TOP, format!("{have}/{max}"), FontId::proportional(11.0), text);
        }
    }
}

/// Above the hotbar: the weapons, the chosen one highlighted with its reload.
fn draw_weapons(painter: &egui::Painter, screen: Rect, weapons: &[WeaponSlot], above: f32) -> f32 {
    let unlocked: Vec<&WeaponSlot> = weapons.iter().filter(|w| w.unlocked).collect();
    if unlocked.is_empty() {
        return above;
    }
    let (w, h) = (if unlocked.len() > 5 { 100.0 } else { 124.0 }, 30.0);
    let width = unlocked.len() as f32 * (w + GAP) - GAP;
    let mut x = screen.center().x - width * 0.5;
    let y = above - 10.0 - h;
    for slot in unlocked {
        let rect = Rect::from_min_size(Pos2::new(x, y), vec2(w, h));
        painter.rect_filled(rect, 6.0, PANEL);
        let color = if slot.selected { WARM } else { Color32::from_gray(170) };
        if slot.selected {
            painter.rect_stroke(rect, 6.0, Stroke::new(2.0, WARM), StrokeKind::Inside);
            let bar = Rect::from_min_size(rect.left_bottom() + vec2(5.0, -5.0), vec2((w - 10.0) * slot.ready.clamp(0.0, 1.0), 2.5));
            painter.rect_filled(bar, 1.0, WARM);
        }
        painter.text(rect.left_center() + vec2(9.0, -1.0), Align2::LEFT_CENTER, slot.weapon.key(), FontId::proportional(15.0), color);
        painter.text(rect.left_center() + vec2(26.0, -1.0), Align2::LEFT_CENTER, slot.weapon.name(), FontId::proportional(12.0), color);
        x += w + GAP;
    }
    y
}

/// Bottom centre, above the weapons: the health bar. Ticks every 25
/// health, a pale trail where health was just lost, the shield as a blue
/// band over it, a pulse when low and a shimmer while it regenerates.
fn draw_vitals(painter: &egui::Painter, screen: Rect, v: &VitalsView, time: f32, bottom: f32) {
    let width = (240.0 + v.max * 0.5).min(400.0);
    let bar = Rect::from_min_size(Pos2::new(screen.center().x - width * 0.5 + 12.0, bottom - 28.0), vec2(width, 18.0));
    let frac = |x: f32| (x / v.max).clamp(0.0, 1.0);
    let low = v.health < v.max * 0.3;
    // Frame.
    painter.rect_filled(bar.expand(3.0), 6.0, Color32::from_black_alpha(170));
    painter.rect_filled(bar, 4.0, Color32::from_rgb(40, 16, 18));
    // The trail of what was just lost, then the health itself.
    let fill = |f: f32| Rect::from_min_size(bar.min, vec2(bar.width() * f, bar.height()));
    painter.rect_filled(fill(frac(v.trail)), 4.0, Color32::from_rgb(240, 225, 200));
    let mut color = if low {
        let p = 0.6 + 0.4 * (time * 7.0).sin();
        Color32::from_rgb(230, 60, 50).gamma_multiply(0.7 + 0.3 * p)
    } else if v.health < v.max * 0.6 {
        Color32::from_rgb(235, 170, 60)
    } else {
        Color32::from_rgb(90, 210, 110)
    };
    if v.heal_flash > 0.0 {
        color = color.lerp_to_gamma(Color32::from_rgb(200, 255, 200), v.heal_flash * 0.7);
    }
    let health_rect = fill(frac(v.health));
    painter.rect_filled(health_rect, 4.0, color);
    // A highlight along the top so it reads as a solid bar.
    painter.rect_filled(Rect::from_min_size(health_rect.min, vec2(health_rect.width(), 5.0)), 3.0, Color32::from_white_alpha(40));
    // Regenerating: a bright band sweeping along the filled part.
    if v.regen && health_rect.width() > 8.0 {
        let x = health_rect.left() + (time * 0.8).fract() * health_rect.width();
        let band = Rect::from_center_size(Pos2::new(x, health_rect.center().y), vec2(14.0, health_rect.height()))
            .intersect(health_rect);
        painter.rect_filled(band, 2.0, Color32::from_white_alpha(60));
    }
    // Ticks every 25 health.
    let mut tick = 25.0;
    while tick < v.max {
        let x = bar.left() + bar.width() * tick / v.max;
        let h = if (tick as u32).is_multiple_of(100) { bar.height() } else { bar.height() * 0.45 };
        painter.line_segment([Pos2::new(x, bar.bottom() - h), Pos2::new(x, bar.bottom())], Stroke::new(1.0, Color32::from_black_alpha(140)));
        tick += 25.0;
    }
    // Shield: a blue band along the top edge.
    if v.max_shield > 0.0 {
        let s = Rect::from_min_size(bar.left_top() - vec2(0.0, 8.0), vec2(bar.width() * (v.shield / v.max_shield).clamp(0.0, 1.0), 5.0));
        painter.rect_filled(Rect::from_min_size(s.min, vec2(bar.width(), 5.0)), 2.0, Color32::from_rgba_unmultiplied(60, 120, 200, 60));
        painter.rect_filled(s, 2.0, Color32::from_rgb(110, 190, 255));
    }
    // Arcane Ward: a glowing gold frame while it holds, with its points.
    if v.ward > 0.5 {
        let glow = 0.6 + 0.4 * (time * 3.0).sin();
        painter.rect_stroke(bar.expand(4.5), 7.0, Stroke::new(2.5, Color32::from_rgb(255, 120, 215).gamma_multiply(glow)), StrokeKind::Outside);
        painter.text(bar.right_center() + vec2(10.0, 0.0), Align2::LEFT_CENTER, format!("+{:.0} ward", v.ward), FontId::proportional(12.0), Color32::from_rgb(255, 150, 225));
    }
    if v.invulnerable {
        painter.rect_stroke(bar.expand(3.0), 6.0, Stroke::new(2.0, GOLD.gamma_multiply(0.6 + 0.4 * (time * 10.0).sin())), StrokeKind::Outside);
    }
    // A heart to the left, beating faster when low.
    let beat = if low { 7.0 } else { 2.2 };
    let pump = 1.0 + 0.12 * (time * beat).sin().max(0.0);
    let heart = Pos2::new(bar.left() - 20.0, bar.center().y);
    let hc = if low { Color32::from_rgb(255, 80, 70) } else { Color32::from_rgb(240, 90, 90) };
    painter.circle_filled(heart + vec2(-4.0, -3.0) * pump, 5.2 * pump, hc);
    painter.circle_filled(heart + vec2(4.0, -3.0) * pump, 5.2 * pump, hc);
    let tip = vec![heart + vec2(-9.0, -1.5) * pump, heart + vec2(9.0, -1.5) * pump, heart + vec2(0.0, 9.0) * pump];
    painter.add(egui::Shape::convex_polygon(tip, hc, Stroke::NONE));
    // Numbers.
    let text = if v.shield > 0.5 {
        format!("{:.0} / {:.0}  +{:.0}", v.health.ceil(), v.max, v.shield.ceil())
    } else {
        format!("{:.0} / {:.0}", v.health.ceil(), v.max)
    };
    shadow_text(painter, bar.center(), Align2::CENTER_CENTER, &text, FontId::proportional(13.0), Color32::WHITE);
    // States worth knowing, in a row just above the bar.
    let mut x = bar.left();
    let above = bar.top() - if v.max_shield > 0.0 { 12.0 } else { 4.0 };
    let mut chip = |label: &str, color: Color32| {
        let font = FontId::proportional(11.0);
        painter.text(Pos2::new(x + 1.0, above + 1.0), Align2::LEFT_BOTTOM, label, font.clone(), Color32::from_black_alpha(200));
        let r = painter.text(Pos2::new(x, above), Align2::LEFT_BOTTOM, label, font, color);
        x = r.right() + 12.0;
    };
    if v.adrenaline {
        chip("ADRENALINE", Color32::from_rgb(255, 110, 80));
    }
    if v.regen {
        chip("REGEN", Color32::from_rgb(140, 235, 150));
    }
    match v.second_wind {
        Some(true) => chip("SECOND WIND READY", GOLD),
        Some(false) => chip("second wind recharging", Color32::from_gray(140)),
        None => {}
    }
}

/// Bottom left: speed, momentum, energy, rockets and status chips (stacked
/// upwards from the corner).

/// A small health bar over a drone, with its effects as coloured pips.
fn draw_drone_bar(painter: &egui::Painter, screen: Rect, b: &BarView) {
    let p = Pos2::new(screen.left() + (b.ndc.x * 0.5 + 0.5) * screen.width(), screen.top() + (0.5 - b.ndc.y * 0.5) * screen.height());
    let w = 46.0 * b.scale;
    let bar = Rect::from_center_size(p, vec2(w, 5.0));
    painter.rect_filled(bar.expand(1.5), 2.0, Color32::from_black_alpha(170));
    let color = if b.health < 0.35 { Color32::from_rgb(240, 80, 60) } else { Color32::from_rgb(255, 170, 70) };
    painter.rect_filled(Rect::from_min_size(bar.min, vec2(w * b.health.clamp(0.0, 1.0), 5.0)), 2.0, color);
    let mut x = bar.left();
    for (on, c) in [
        (b.bleeding, Color32::from_rgb(230, 50, 50)),
        (b.burning, Color32::from_rgb(255, 150, 40)),
        (b.stunned, Color32::from_rgb(255, 230, 90)),
    ] {
        if on {
            painter.circle_filled(Pos2::new(x + 3.0, bar.top() - 6.0), 3.0, c);
            x += 9.0;
        }
    }
}

/// A damage number rising and fading.
fn draw_popup(painter: &egui::Painter, screen: Rect, t: &ScreenText) {
    let p = Pos2::new(screen.left() + (t.ndc.x * 0.5 + 0.5) * screen.width(), screen.top() + (0.5 - t.ndc.y * 0.5) * screen.height());
    let a = (1.0 - (t.age - 0.6).max(0.0) / 0.5).clamp(0.0, 1.0);
    let pop = 1.0 + 0.5 * (1.0 - (t.age / 0.12).min(1.0));
    let size = if t.big { 20.0 } else { 15.0 } * pop;
    painter.text(p + vec2(1.5, 1.5), Align2::CENTER_CENTER, &t.text, FontId::proportional(size), Color32::from_black_alpha((180.0 * a) as u8));
    painter.text(p, Align2::CENTER_CENTER, &t.text, FontId::proportional(size), t.color.gamma_multiply(a));
}

fn draw_meters(painter: &egui::Painter, screen: Rect, m: &Meters, bottom: f32) {
    let x = screen.left() + 16.0;
    let mut y = bottom;
    let font = FontId::proportional(14.0);
    let mut lines: Vec<(String, Color32, Option<f32>)> = Vec::new();
    if let Some((_, max)) = m.rockets
        && max >= crate::abilities::UNLIMITED
    {
        lines.push(("Rockets  unlimited".into(), GOLD, None));
    } else if let Some((left, max)) = m.rockets {
        let pips: String = (0..max.min(24)).map(|i| if i < left { '|' } else { '.' }).collect();
        lines.push((format!("Rockets  {pips}"), WARM, None));
    }
    if let Some((label, fraction)) = m.energy {
        lines.push((label.to_owned(), Color32::WHITE, Some(fraction)));
    }
    if let Some(momentum) = m.momentum {
        lines.push((format!("Momentum x{momentum:.2}"), Color32::WHITE, None));
    }
    if m.speed > 0.5 {
        lines.push((format!("{:.0} m/s", m.speed), Color32::from_gray(200), None));
    }
    for status in &m.status {
        lines.push(((*status).to_owned(), ACCENT, None));
    }
    for (text, color, bar) in lines {
        shadow_text(painter, Pos2::new(x, y), Align2::LEFT_BOTTOM, &text, font.clone(), color);
        if let Some(fraction) = bar {
            let rect = Rect::from_min_size(Pos2::new(x + 74.0, y - 12.0), vec2(120.0, 7.0));
            painter.rect_filled(rect, 3.0, Color32::from_white_alpha(30));
            painter.rect_filled(Rect::from_min_size(rect.min, vec2(120.0 * fraction.clamp(0.0, 1.0), 7.0)), 3.0, ACCENT);
        }
        y -= 22.0;
    }
}
