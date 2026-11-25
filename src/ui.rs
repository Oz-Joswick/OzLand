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

use crate::abilities::{Group, Moveset, Progress, Tree};
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
    pub key: &'static str,
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
    pub connected: bool,
    pub status: String,
    /// Your colour and the other player's (linear RGB).
    pub my_color: glam::Vec3,
    pub other_color: glam::Vec3,
    /// This computer's address on the local network.
    pub lan_ip: Option<String>,
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
    /// Your health (0..1) while in a match.
    pub health: Option<f32>,
    /// The other player's head on screen (-1..1 both ways) and health.
    pub enemy: Option<(glam::Vec2, f32)>,
    /// Out of the current round.
    pub dead: bool,
    /// Auto-update status, and whether a restart would apply it.
    pub update: Option<(String, bool)>,
    /// A short message (crystal found, ability unlocked...).
    pub toast: Option<&'a str>,
}

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
    pub selected: Tree,
    /// Multiplayer text fields.
    pub host_port: String,
    pub join_address: String,
    /// The item shown in detail on the Items tab.
    pub selected_item: usize,
    /// `Reset all players` was clicked once; a second click confirms.
    pub confirm_reset: bool,
}

impl Default for MenuState {
    fn default() -> Self {
        Self {
            tab: Tab::Abilities,
            selected: Tree::Movement,
            host_port: crate::net::DEFAULT_PORT.to_string(),
            join_address: String::new(),
            selected_item: 0,
            confirm_reset: false,
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
    /// Reset every player's progress and the score.
    ResetAll,
    /// Restart into the installed update.
    Restart,
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
    egui::Window::new("Paused")
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
        .collapsible(false)
        .resizable(false)
        .fixed_size([860.0, 0.0])
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                for (tab, name) in [
                    (Tab::Abilities, "Abilities & Weapons"),
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
                Tab::Items => items_tab(ui, settings, progress, menu),
                Tab::Multiplayer => {
                    if let Some(a) = multiplayer_tab(ui, menu, hud) {
                        action = a;
                    }
                }
                Tab::Settings => settings_tab(ui, settings),
            }
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button(RichText::new("Resume").strong()).clicked() {
                    action = MenuAction::Resume;
                }
                if menu.tab == Tab::Settings && ui.button("Reset to defaults").clicked() {
                    let (unlock, ascension) = (settings.unlock_all, settings.sandbox_ascension);
                    *settings = Settings::default();
                    (settings.unlock_all, settings.sandbox_ascension) = (unlock, ascension);
                }
                ui.label(RichText::new("Esc to resume · saved automatically").small().weak());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Quit").clicked() {
                        action = MenuAction::Quit;
                    }
                });
            });
        });
    action
}

/// The level shown for `tree` (sandbox shows everything unlocked).
fn shown_level(tree: Tree, settings: &Settings, progress: &Progress) -> u32 {
    match (settings.unlock_all, tree.max_level()) {
        (true, Some(max)) => max,
        (true, None) => settings.sandbox_ascension as u32,
        (false, _) => progress.level(tree),
    }
}

fn abilities_tab(
    ui: &mut egui::Ui,
    settings: &mut Settings,
    progress: &mut Progress,
    menu: &mut MenuState,
    hud: &Hud,
    moves: &Moveset,
) {
    // Header: points, experience, crystals, sandbox.
    ui.horizontal(|ui| {
        let points = RichText::new(format!("{} skill point{}", progress.points, if progress.points == 1 { "" } else { "s" }))
            .size(22.0)
            .color(if progress.points > 0 { WARM } else { Color32::LIGHT_GRAY });
        ui.label(points);
        ui.add_space(12.0);
        ui.vertical(|ui| {
            ui.add(
                egui::ProgressBar::new(progress.xp / progress.xp_needed())
                    .desired_width(260.0)
                    .text(format!("XP {:.0} / {:.0}", progress.xp, progress.xp_needed())),
            );
            ui.label(
                RichText::new(format!(
                    "Crystals {} / {} · drones destroyed {} · points also come from travel and air time",
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
    ui.separator();

    ui.horizontal_top(|ui| {
        // Left: every tree, grouped.
        ui.vertical(|ui| {
            ui.set_width(240.0);
            egui::ScrollArea::vertical()
                .id_salt("trees")
                .auto_shrink([false, false])
                .min_scrolled_height(450.0)
                .max_height(450.0)
                .show(ui, |ui| {
                    for group in Group::ALL {
                        ui.add_space(4.0);
                        ui.label(RichText::new(group.name().to_uppercase()).small().strong().color(Color32::GRAY));
                        for tree in Tree::ALL.into_iter().filter(|t| t.group() == group) {
                            let level = shown_level(tree, settings, progress);
                            let can = !settings.unlock_all && progress.can_upgrade(tree);
                            if tree_row(ui, tree, level, menu.selected == tree, can) {
                                menu.selected = tree;
                            }
                        }
                    }
                });
        });
        ui.separator();
        // Right: the chosen tree in detail.
        ui.vertical(|ui| {
            egui::ScrollArea::vertical()
                .id_salt("detail")
                .auto_shrink([false, false])
                .min_scrolled_height(450.0)
                .max_height(450.0)
                .show(ui, |ui| tree_detail(ui, menu.selected, settings, progress, moves));
        });
    });
}

/// A clickable row: name, level, progress pips, and a dot if it can be
/// upgraded. Returns true when clicked.
fn tree_row(ui: &mut egui::Ui, tree: Tree, level: u32, selected: bool, can_upgrade: bool) -> bool {
    let (rect, response) = ui.allocate_exact_size(vec2(230.0, 36.0), Sense::click());
    let painter = ui.painter_at(rect);
    let bg = if selected {
        Color32::from_rgb(45, 70, 100)
    } else if response.hovered() {
        Color32::from_white_alpha(14)
    } else {
        Color32::TRANSPARENT
    };
    painter.rect_filled(rect, 6.0, bg);
    let text = if selected { Color32::WHITE } else { Color32::from_gray(215) };
    painter.text(rect.left_top() + vec2(10.0, 3.0), Align2::LEFT_TOP, tree.name(), FontId::proportional(15.0), text);
    match tree.max_level() {
        Some(max) => {
            for i in 0..max {
                let c = rect.left_top() + vec2(14.0 + i as f32 * 14.0, 27.0);
                // The last pip is the capstone: bigger, gold.
                let (r, have) = if i + 1 == max { (5.0, GOLD) } else { (4.0, UNLOCKED) };
                if i < level {
                    painter.circle_filled(c, r, have);
                } else {
                    let edge = if i + 1 == max { GOLD.gamma_multiply(0.6) } else { Color32::from_gray(110) };
                    painter.circle_stroke(c, r, Stroke::new(1.0, edge));
                }
            }
            painter.text(rect.right_top() + vec2(-10.0, 5.0), Align2::RIGHT_TOP, format!("{level}/{max}"), FontId::proportional(13.0), Color32::GRAY);
        }
        None => {
            painter.text(rect.left_top() + vec2(10.0, 20.0), Align2::LEFT_TOP, format!("Level {level}"), FontId::proportional(12.0), ACCENT);
        }
    }
    if can_upgrade {
        painter.circle_filled(rect.right_center() + vec2(-14.0, 6.0), 5.0, WARM);
    }
    response.on_hover_text(tree.tagline()).clicked()
}

/// A small key label, like a keyboard key.
fn key_chip(ui: &mut egui::Ui, key: &str) {
    if !key.is_empty() {
        ui.label(RichText::new(format!(" {key} ")).monospace().small().color(Color32::WHITE).background_color(Color32::from_gray(70)));
    }
}

fn tree_detail(ui: &mut egui::Ui, tree: Tree, settings: &Settings, progress: &mut Progress, moves: &Moveset) {
    let level = shown_level(tree, settings, progress);
    ui.horizontal(|ui| {
        ui.label(RichText::new(tree.name()).size(22.0).strong());
        ui.label(RichText::new(tree.group().name()).small().weak());
    });
    ui.label(RichText::new(tree.tagline()).italics().weak());
    ui.add_space(8.0);

    if tree.max_level().is_none() {
        ascension_detail(ui, level, settings, progress, moves);
        return;
    }

    for (i, l) in tree.levels().iter().enumerate() {
        let n = i as u32 + 1;
        let unlocked = n <= level;
        let next = n == level + 1;
        let capstone = tree.max_level() == Some(n);
        egui::Frame::new()
            .fill(if capstone { Color32::from_rgba_unmultiplied(255, 200, 80, 14) } else if next { Color32::from_white_alpha(10) } else { Color32::TRANSPARENT })
            .corner_radius(6.0)
            .inner_margin(6.0)
            .show(ui, |ui| {
                ui.horizontal_top(|ui| {
                    // Status marker: filled (have it), ringed (next), dim (later).
                    let (r, _) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
                    let p = ui.painter();
                    if unlocked {
                        p.circle_filled(r.center(), 8.0, UNLOCKED);
                    } else if next {
                        p.circle_stroke(r.center(), 8.0, Stroke::new(2.0, WARM));
                    } else {
                        p.circle_stroke(r.center(), 8.0, Stroke::new(1.0, Color32::from_gray(90)));
                    }
                    p.text(r.center(), Align2::CENTER_CENTER, n.to_string(), FontId::proportional(10.0), if unlocked { Color32::BLACK } else { Color32::GRAY });
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            let name = RichText::new(l.name).strong();
                            ui.label(if unlocked { name.color(UNLOCKED) } else if next { name } else { name.weak() });
                            key_chip(ui, l.key);
                            if capstone {
                                ui.label(RichText::new(" CAPSTONE ").small().strong().color(Color32::BLACK).background_color(GOLD));
                            }
                            if next && !settings.unlock_all {
                                let cost = tree.cost(n);
                                let text = format!("Unlock · {cost} point{}", if cost == 1 { "" } else { "s" });
                                let blocker = progress.blocker(tree);
                                let button = ui.add_enabled(blocker.is_none(), egui::Button::new(text));
                                let button = match &blocker {
                                    Some(why) => button.on_disabled_hover_text(why),
                                    None => button,
                                };
                                if button.clicked() {
                                    progress.upgrade(tree);
                                }
                            }
                        });
                        let desc = RichText::new(l.description.trim_start_matches("CAPSTONE. "));
                        ui.label(if unlocked || next { desc } else { desc.weak() });
                        // The trial standing between you and the capstone.
                        if capstone
                            && !unlocked
                            && let Some(trial) = tree.trial()
                        {
                            let done = progress.trial_done(&trial);
                            let have = progress.stat(trial.stat).min(trial.goal);
                            let label = format!("Trial: {} ({:.0}/{:.0}{}{})", trial.description, have, trial.goal, if trial.unit.is_empty() { "" } else { " " }, trial.unit);
                            ui.add(
                                egui::ProgressBar::new(progress.trial_progress(&trial))
                                    .desired_width(330.0)
                                    .fill(if done { UNLOCKED } else { Color32::from_rgb(150, 110, 40) })
                                    .text(if done { format!("Trial complete: {}", trial.description) } else { label }),
                            );
                        }
                    });
                });
            });
    }

    if tree == Tree::Arsenal {
        ui.add_space(10.0);
        ui.label(RichText::new("Weapons").strong());
        egui::Grid::new("weapon stats").striped(true).spacing([14.0, 6.0]).show(ui, |ui| {
            for h in ["", "Weapon", "Damage", "Shots/s", ""] {
                ui.label(RichText::new(h).small().weak());
            }
            ui.end_row();
            for w in Weapon::ALL {
                let have = moves.weapons >= w.unlocked_at();
                key_chip(ui, w.key());
                let (damage, rate, note) = w.stats(moves);
                let text = |s: String| if have { RichText::new(s) } else { RichText::new(s).weak() };
                ui.label(text(w.name().into()));
                ui.label(text(damage));
                ui.label(text(format!("{rate:.1}")));
                ui.label(text(if have { note.into() } else { format!("Arsenal level {}", w.unlocked_at()) }));
                ui.end_row();
            }
        });
        ui.label(RichText::new("Left mouse fires · drones take 6 damage").small().weak());
    }
}

/// Ascension: the current level, its next step, and the total it adds up to.
fn ascension_detail(ui: &mut egui::Ui, level: u32, settings: &Settings, progress: &mut Progress, moves: &Moveset) {
    let info = Tree::Ascension.level_info(1);
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("Level {level}")).size(18.0).color(ACCENT));
        if !settings.unlock_all {
            let button = ui.add_enabled(progress.can_upgrade(Tree::Ascension), egui::Button::new("Ascend · 1 point"));
            if button.clicked() {
                progress.upgrade(Tree::Ascension);
            }
        }
    });
    ui.label(info.description);
    ui.add_space(10.0);
    ui.label(RichText::new("What you have now (all trees together)").strong());
    egui::Grid::new("totals").striped(true).spacing([24.0, 4.0]).show(ui, |ui| {
        let mut row = |name: &str, value: String| {
            ui.label(name);
            ui.label(RichText::new(value).color(ACCENT));
            ui.end_row();
        };
        row("Run speed", format!("x{:.2}", moves.speed));
        row("Jump height", format!("x{:.2}", moves.jump * moves.jump));
        // Only what you actually have.
        if moves.air_jumps > 0 {
            row("Air jumps", moves.air_jumps.to_string());
        }
        if moves.dash_charges > 0 {
            row("Dash charges", moves.dash_charges.to_string());
        }
        if moves.glide {
            row("Rockets per flight", if moves.boosts >= crate::abilities::UNLIMITED { "unlimited".into() } else { moves.boosts.to_string() });
            row("Rocket speed", format!("{:.0} m/s", moves.rocket_speed));
        }
        if moves.max_clones > 0 {
            row("Clones", moves.max_clones.to_string());
        }
        if moves.grapple_range > 0.0 {
            row("Grapple range", format!("{:.0} m", moves.grapple_range));
        }
        if moves.pads > 0 {
            row("Pad launch", format!("{:.0} m/s", moves.pad_launch));
        }
        if moves.weapons > 0 {
            row("Weapon damage", format!("x{:.2}", moves.weapon_damage));
        }
    });
}

/// The item tree: tiers left to right, lines from each item to the items
/// it needs, and the chosen item's details underneath.
fn items_tab(ui: &mut egui::Ui, settings: &Settings, progress: &mut Progress, menu: &mut MenuState) {
    use crate::items::ITEMS;
    let sandbox = settings.unlock_all;
    let owned = |p: &Progress, i: usize| sandbox || p.has_item(i);

    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("{} scrap", progress.scrap)).size(22.0).color(GOLD));
        ui.add_space(12.0);
        ui.vertical(|ui| {
            let have = (0..ITEMS.len()).filter(|i| owned(progress, *i)).count();
            ui.label(format!("{have} / {} items", ITEMS.len()));
            ui.label(
                RichText::new(if sandbox {
                    "Sandbox: every item is active."
                } else {
                    "Drones drop 5 scrap, crystals give 15. Most items need several others first. Items stack on top of your abilities."
                })
                .small()
                .weak(),
            );
        });
    });
    ui.separator();

    // Lay the nodes out: one column per tier, centred vertically.
    let (area, _) = ui.allocate_exact_size(vec2(840.0, 350.0), Sense::hover());
    let node = vec2(172.0, 44.0);
    let column_gap = (area.width() - 20.0 - 4.0 * node.x) / 3.0;
    let mut rects = vec![Rect::NOTHING; ITEMS.len()];
    for tier in 1..=4u32 {
        let members: Vec<usize> = (0..ITEMS.len()).filter(|i| ITEMS[*i].tier == tier).collect();
        let pitch = 56.0;
        let top = area.center().y - (members.len() as f32 * pitch - (pitch - node.y)) * 0.5;
        let x = area.left() + 10.0 + (tier - 1) as f32 * (node.x + column_gap);
        for (row, &i) in members.iter().enumerate() {
            rects[i] = Rect::from_min_size(Pos2::new(x, top + row as f32 * pitch), node);
        }
    }

    let painter = ui.painter_at(area);
    // Lines first, under the nodes: green once the needed item is owned.
    for (i, it) in ITEMS.iter().enumerate() {
        for &n in it.needs {
            let (a, b) = (rects[n].right_center(), rects[i].left_center());
            // Each item's lines bend at their own x, so they don't merge.
            let k = ((rects[i].center().y - area.top()) / area.height()).clamp(0.0, 1.0);
            let mid = a.x + (b.x - a.x) * (0.2 + 0.6 * k);
            let color = if owned(progress, n) { UNLOCKED.gamma_multiply(0.8) } else { Color32::from_gray(70) };
            painter.add(egui::Shape::line(
                vec![a, Pos2::new(mid, a.y), Pos2::new(mid, b.y), b],
                Stroke::new(if owned(progress, n) { 2.0 } else { 1.0 }, color),
            ));
        }
    }
    for (i, it) in ITEMS.iter().enumerate() {
        let rect = rects[i];
        let response = ui.interact(rect, ui.scope_id().with(("item node", i)), Sense::click());
        if response.clicked() {
            menu.selected_item = i;
        }
        let have = owned(progress, i);
        let parts_ready = it.needs.iter().all(|n| owned(progress, *n));
        let affordable = progress.scrap >= it.cost;
        let (fill, edge) = if have {
            (Color32::from_rgb(30, 60, 38), UNLOCKED)
        } else if parts_ready && affordable {
            (Color32::from_rgb(52, 44, 24), GOLD)
        } else if parts_ready {
            (Color32::from_rgb(36, 33, 28), GOLD.gamma_multiply(0.5))
        } else {
            (Color32::from_rgb(26, 26, 28), Color32::from_gray(65))
        };
        painter.rect_filled(rect, 7.0, fill);
        let width = if menu.selected_item == i { 2.5 } else if response.hovered() { 1.8 } else { 1.2 };
        let edge = if menu.selected_item == i { Color32::WHITE } else { edge };
        painter.rect_stroke(rect, 7.0, Stroke::new(width, edge), StrokeKind::Inside);
        let text = if have || parts_ready { Color32::WHITE } else { Color32::from_gray(120) };
        painter.text(rect.left_top() + vec2(10.0, 6.0), Align2::LEFT_TOP, it.name, FontId::proportional(14.0), text);
        let (status, color) = if have {
            ("Owned".to_owned(), UNLOCKED)
        } else {
            (format!("{} scrap", it.cost), if parts_ready && affordable { GOLD } else { Color32::from_gray(110) })
        };
        painter.text(rect.left_bottom() + vec2(10.0, -6.0), Align2::LEFT_BOTTOM, status, FontId::proportional(11.0), color);
        painter.text(rect.right_bottom() + vec2(-8.0, -6.0), Align2::RIGHT_BOTTOM, format!("T{}", it.tier), FontId::proportional(10.0), Color32::from_gray(100));
    }

    // The chosen item.
    ui.separator();
    let i = menu.selected_item.min(ITEMS.len() - 1);
    let it = &ITEMS[i];
    ui.horizontal(|ui| {
        ui.label(RichText::new(it.name).size(18.0).strong());
        ui.label(RichText::new(format!("Tier {} · {} scrap", it.tier, it.cost)).weak());
        if !sandbox && !progress.has_item(i) {
            let blocker = progress.item_blocker(i);
            let button = ui.add_enabled(blocker.is_none(), egui::Button::new(format!("Unlock · {} scrap", it.cost)));
            let button = match &blocker {
                Some(why) => button.on_disabled_hover_text(why),
                None => button,
            };
            if button.clicked() {
                progress.buy_item(i);
            }
        } else {
            ui.label(RichText::new("Owned").color(UNLOCKED));
        }
    });
    ui.label(it.description);
    if !it.needs.is_empty() {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Needs:").weak());
            for &n in it.needs {
                let name = RichText::new(ITEMS[n].name);
                ui.label(if owned(progress, n) { name.color(UNLOCKED) } else { name.color(WARM) });
            }
        });
    }
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
            if let Some(ip) = &net.lan_ip {
                ui.label(RichText::new(format!("Same Wi-Fi: {ip}:{}", menu.host_port.trim())).color(ACCENT));
            }
            ui.label(RichText::new("Different networks: forward UDP on this port to this PC in your router, then give your public IP:port. The host firewall must allow the game.").small().weak());

            let ui = &mut cols[1];
            ui.label(RichText::new("Join a game").heading());
            ui.label("Enter the host's LAN or public IP:port. You're the teal player.");
            ui.horizontal(|ui| {
                ui.label("Address");
            ui.add(egui::TextEdit::singleline(&mut menu.join_address).hint_text("public IP or LAN IP:7777").desired_width(170.0));
            });
            let ready = !menu.join_address.trim().is_empty();
            if ui.add_enabled(ready, egui::Button::new("Join")).clicked() {
                action = Some(MenuAction::Join(menu.join_address.trim().to_owned()));
            }
        });
    }
    ui.add_space(10.0);
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
            "Two players. On one computer, join 127.0.0.1:7777. Across the internet, the host must forward UDP \
             port 7777 (or the chosen port) to this PC and allow it through the firewall; give the joiner your public IP. \
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

    // Crosshair with a dark outline so it shows on bright and dark surfaces.
    let c = screen.center();
    for (width, color) in [(4.0, Color32::from_black_alpha(140)), (2.0, Color32::from_white_alpha(230))] {
        let stroke = Stroke::new(width, color);
        painter.line_segment([c - vec2(8.0, 0.0), c + vec2(8.0, 0.0)], stroke);
        painter.line_segment([c - vec2(0.0, 8.0), c + vec2(0.0, 8.0)], stroke);
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
    draw_weapons(&painter, screen, &hud.weapons, top);
    draw_meters(&painter, screen, &hud.meters);

    if let Some(m) = &hud.match_view {
        draw_scoreboard(&painter, screen, m);
    }
    if let Some(health) = hud.health {
        let y = weapons_top(screen, hud) - 26.0;
        let bar = Rect::from_center_size(Pos2::new(screen.center().x, y), vec2(300.0, 14.0));
        painter.rect_filled(bar.expand(2.0), 4.0, PANEL);
        let color = if health < 0.3 { Color32::from_rgb(230, 70, 60) } else { ui_color(hud.net.my_color).gamma_multiply(1.4) };
        painter.rect_filled(Rect::from_min_size(bar.min, vec2(bar.width() * health.clamp(0.0, 1.0), bar.height())), 3.0, color);
        let text = format!("{:.0} HP", health * crate::arena::MAX_HEALTH);
        shadow_text(&painter, bar.center(), Align2::CENTER_CENTER, &text, FontId::proportional(12.0), Color32::WHITE);
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

    // Notices, under the scoreboard.
    if let Some(toast) = hud.toast {
        let pos = screen.center_top() + vec2(0.0, 92.0);
        shadow_text(&painter, pos, Align2::CENTER_TOP, toast, FontId::proportional(22.0), Color32::from_rgb(140, 230, 255));
    }
}

/// Where the weapon bar's top edge is (or the hotbar's, or the bottom).
fn weapons_top(screen: Rect, hud: &Hud) -> f32 {
    let mut y = screen.bottom() - 14.0;
    if !hud.slots.is_empty() {
        y -= TILE;
    }
    if hud.weapons.iter().any(|w| w.unlocked) {
        y -= 10.0 + 30.0;
    }
    y
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
    painter.text(rect.left_top() + vec2(7.0, 5.0), Align2::LEFT_TOP, slot.key, key_font, text);
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
fn draw_weapons(painter: &egui::Painter, screen: Rect, weapons: &[WeaponSlot], above: f32) {
    let unlocked: Vec<&WeaponSlot> = weapons.iter().filter(|w| w.unlocked).collect();
    if unlocked.is_empty() {
        return;
    }
    let (w, h) = (128.0, 30.0);
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
        painter.text(rect.left_center() + vec2(26.0, -1.0), Align2::LEFT_CENTER, slot.weapon.name(), FontId::proportional(13.0), color);
        x += w + GAP;
    }
}

/// Bottom left: speed, momentum, energy, rockets and status chips (stacked
/// upwards from the corner).
fn draw_meters(painter: &egui::Painter, screen: Rect, m: &Meters) {
    let x = screen.left() + 16.0;
    let mut y = screen.bottom() - 16.0;
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
