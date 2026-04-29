#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::{egui, NativeOptions};
use egui::{Align, Color32, FontId, Frame, Layout, Pos2, Rect, RichText, Sense, Stroke, Vec2};
use egui_dock::{DockArea, DockState, NodeIndex, Style};

fn main() -> eframe::Result<()> {
    let options = NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 800.0])
            .with_title("Active Worlds — egui_dock demo"),
        ..Default::default()
    };
    eframe::run_native(
        "Active Worlds",
        options,
        Box::new(|_cc| Ok(Box::<App>::default())),
    )
}

// ---------------------------------------------------------------------------
// Tab data

enum Tab {
    Viewport3D,
    Chat,
    Objects,
    Properties,
    Materials,
    Worlds,
    Avatar,
}

// ---------------------------------------------------------------------------
// App state (mutable data that lives outside the tree)

struct AppState {
    chat_input: String,
    chat_log: Vec<(String, Color32, String)>,
    selected_world: String,
    avatar_model: String,
    frame: u64,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            chat_input: String::new(),
            chat_log: vec![
                (
                    "Flagg".into(),
                    Color32::from_rgb(255, 200, 100),
                    "Welcome to Active Worlds!".into(),
                ),
                (
                    "Roland".into(),
                    Color32::from_rgb(100, 200, 255),
                    "Hey everyone!".into(),
                ),
                (
                    "Cypress".into(),
                    Color32::from_rgb(150, 255, 150),
                    "Building a castle near 100N 100W".into(),
                ),
                (
                    "Flagg".into(),
                    Color32::from_rgb(255, 200, 100),
                    "Nice spot!".into(),
                ),
                (
                    "Roland".into(),
                    Color32::from_rgb(100, 200, 255),
                    "I'll teleport over".into(),
                ),
                (
                    "System".into(),
                    Color32::GRAY,
                    "Roland has entered the world".into(),
                ),
            ],
            selected_world: "AW".into(),
            avatar_model: "tourist".into(),
            frame: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// TabViewer

struct Viewer<'a> {
    state: &'a mut AppState,
}

impl egui_dock::TabViewer for Viewer<'_> {
    type Tab = Tab;

    fn title(&mut self, tab: &mut Self::Tab) -> egui::WidgetText {
        match tab {
            Tab::Viewport3D => "3D Viewport".into(),
            Tab::Chat => "Chat".into(),
            Tab::Objects => "Objects".into(),
            Tab::Properties => "Properties".into(),
            Tab::Materials => "Materials".into(),
            Tab::Worlds => "Worlds".into(),
            Tab::Avatar => "Avatar".into(),
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Self::Tab) {
        match tab {
            Tab::Viewport3D => draw_viewport(ui, self.state.frame),
            Tab::Chat => draw_chat(ui, &mut self.state.chat_input, &mut self.state.chat_log),
            Tab::Objects => draw_objects(ui),
            Tab::Properties => draw_properties(ui),
            Tab::Materials => draw_materials(ui),
            Tab::Worlds => draw_worlds(ui, &mut self.state.selected_world),
            Tab::Avatar => draw_avatar(ui, &mut self.state.avatar_model),
        }
    }

    /// The 3D viewport hides its tab bar when it is the only tab in its pane,
    /// so the render fills the full available area without a tab strip.
    fn solo_tab_no_bar(&self, tab: &Self::Tab) -> bool {
        matches!(tab, Tab::Viewport3D)
    }

    fn scroll_bars(&self, tab: &Self::Tab) -> [bool; 2] {
        // The viewport and chat handle scrolling internally.
        match tab {
            Tab::Viewport3D | Tab::Chat => [false, false],
            _ => [false, true],
        }
    }
}

// ---------------------------------------------------------------------------
// App

struct App {
    tree: DockState<Tab>,
    state: AppState,
}

impl Default for App {
    fn default() -> Self {
        // Right panel: info tabs
        let mut tree = DockState::new(vec![
            Tab::Objects,
            Tab::Properties,
            Tab::Materials,
            Tab::Worlds,
            Tab::Avatar,
        ]);

        // Left 30%: start with the 3D viewport.
        let [_right, left] =
            tree.main_surface_mut()
                .split_left(NodeIndex::root(), 0.30, vec![Tab::Viewport3D]);

        // Split the left pane: viewport takes top 70%, chat takes bottom 30%.
        tree.main_surface_mut()
            .split_below(left, 0.70, vec![Tab::Chat]);

        Self {
            tree,
            state: AppState::default(),
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.state.frame += 1;
        DockArea::new(&mut self.tree)
            .style(Style::from_egui(ui.style().as_ref()))
            .show_leaf_close_all_buttons(false)
            .show_leaf_collapse_buttons(false)
            .multi_row_tabs(true)
            .show_inside(
                ui,
                &mut Viewer {
                    state: &mut self.state,
                },
            );
    }
}

// ---------------------------------------------------------------------------
// Tab content renderers

fn draw_viewport(ui: &mut egui::Ui, frame: u64) {
    let rect = ui.available_rect_before_wrap();
    let painter = ui.painter_at(rect);

    // Sky
    painter.rect_filled(rect, 0.0, Color32::from_rgb(20, 30, 60));

    let horizon_y = rect.top() + rect.height() * 0.40;

    // Simple sky-to-horizon gradient via a few horizontal bands
    let sky_bands = 6;
    for i in 0..sky_bands {
        let t = i as f32 / sky_bands as f32;
        let y0 = rect.top() + (horizon_y - rect.top()) * t;
        let y1 = rect.top() + (horizon_y - rect.top()) * (t + 1.0 / sky_bands as f32);
        let c = Color32::from_rgb(
            (20.0 + 60.0 * t) as u8,
            (30.0 + 90.0 * t) as u8,
            (60.0 + 100.0 * t) as u8,
        );
        painter.rect_filled(Rect::from_x_y_ranges(rect.x_range(), y0..=y1), 0.0, c);
    }

    // Ground
    painter.rect_filled(
        Rect::from_x_y_ranges(rect.x_range(), horizon_y..=rect.bottom()),
        0.0,
        Color32::from_rgb(25, 55, 25),
    );

    // Perspective grid on the ground
    let vp = Pos2::new(rect.center().x, horizon_y);
    let grid_color = Color32::from_rgba_premultiplied(60, 110, 60, 200);
    let rows = 10u32;
    let cols = 12u32;

    for row in 1..=rows {
        let t = row as f32 / rows as f32;
        let y = horizon_y + (rect.bottom() - horizon_y) * t;
        let spread = rect.width() * 0.6 * t;
        painter.line_segment(
            [Pos2::new(vp.x - spread, y), Pos2::new(vp.x + spread, y)],
            Stroke::new(1.0, grid_color),
        );
    }
    for col in 0..=cols {
        let t = col as f32 / cols as f32 - 0.5; // -0.5 .. 0.5
        let x_far = vp.x + t * rect.width() * 1.2;
        painter.line_segment(
            [vp, Pos2::new(x_far, rect.bottom())],
            Stroke::new(1.0, grid_color),
        );
    }

    // A few stylised "buildings" on the horizon
    struct Building {
        cx: f32,
        w: f32,
        h: f32,
        color: Color32,
    }
    let buildings = [
        Building {
            cx: 0.32,
            w: 0.04,
            h: 0.18,
            color: Color32::from_rgb(110, 85, 65),
        },
        Building {
            cx: 0.46,
            w: 0.06,
            h: 0.28,
            color: Color32::from_rgb(80, 100, 140),
        },
        Building {
            cx: 0.55,
            w: 0.03,
            h: 0.14,
            color: Color32::from_rgb(130, 95, 75),
        },
        Building {
            cx: 0.63,
            w: 0.05,
            h: 0.22,
            color: Color32::from_rgb(90, 120, 90),
        },
        Building {
            cx: 0.40,
            w: 0.025,
            h: 0.10,
            color: Color32::from_rgb(100, 80, 110),
        },
    ];
    for b in &buildings {
        let bx = rect.left() + rect.width() * b.cx;
        let bw = rect.width() * b.w;
        let bh = rect.height() * b.h;
        painter.rect_filled(
            Rect::from_min_size(Pos2::new(bx - bw * 0.5, horizon_y - bh), Vec2::new(bw, bh)),
            1.0,
            b.color,
        );
        // Simple roof/roof-line
        painter.line_segment(
            [
                Pos2::new(bx - bw * 0.5, horizon_y - bh),
                Pos2::new(bx + bw * 0.5, horizon_y - bh),
            ],
            Stroke::new(1.5, b.color.gamma_multiply(1.4)),
        );
    }

    // HUD overlay
    let hud_rect = Rect::from_min_size(rect.min + Vec2::new(8.0, 8.0), Vec2::new(220.0, 56.0));
    painter.rect_filled(
        hud_rect,
        4.0,
        Color32::from_rgba_premultiplied(0, 0, 0, 140),
    );
    ui.scope_builder(egui::UiBuilder::new().max_rect(hud_rect), |ui| {
        ui.add_space(4.0);
        ui.label(
            RichText::new("Active Worlds 3D Viewport")
                .color(Color32::WHITE)
                .small()
                .strong(),
        );
        ui.label(
            RichText::new(format!(
                "Frame: {frame}  |  Pos: 0N 0W 0A  |  Facing: 0.0deg"
            ))
            .color(Color32::from_rgb(180, 200, 180))
            .small(),
        );
        ui.label(
            RichText::new("Drag the empty tab-bar space to move the whole group")
                .color(Color32::from_rgb(150, 170, 150))
                .small(),
        );
    });

    // Claim the full rect so egui doesn't add extra spacing
    ui.allocate_rect(rect, Sense::hover());
}

fn draw_chat(ui: &mut egui::Ui, input: &mut String, log: &mut Vec<(String, Color32, String)>) {
    let total = ui.available_rect_before_wrap();
    let input_h = 32.0;
    let log_h = (total.height() - input_h - 4.0).max(0.0);

    egui::ScrollArea::vertical()
        .id_salt("aw_chat_log")
        .stick_to_bottom(true)
        .max_height(log_h)
        .show(ui, |ui| {
            for (name, color, msg) in log.iter() {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(format!("{name}:")).color(*color).strong());
                    ui.label(RichText::new(msg).color(Color32::LIGHT_GRAY));
                });
            }
        });

    ui.separator();

    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
        let send_w = 52.0;
        let input_w = (ui.available_width() - send_w - 8.0).max(0.0);
        let resp = ui.add_sized(
            [input_w, input_h - 4.0],
            egui::TextEdit::singleline(input).hint_text("Say something..."),
        );
        let send = ui.button("Send");
        if send.clicked() || (resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))) {
            if !input.trim().is_empty() {
                log.push((
                    "You".into(),
                    Color32::from_rgb(220, 220, 100),
                    input.clone(),
                ));
            }
            input.clear();
            resp.request_focus();
        }
    });
}

fn draw_objects(ui: &mut egui::Ui) {
    ui.strong("World Objects");
    ui.separator();

    let objects: &[(&str, &str, &str)] = &[
        ("Castle", "100N 100W 0A", "Cypress"),
        ("Tree_Oak_01", "50N 200W 0A", "Roland"),
        ("House_Brick_03", "75N 150W 0A", "Flagg"),
        ("Lamp_Post", "0N 0W 0A", "System"),
        ("Water_Plane", "200N 50W 0A", "Roland"),
        ("Stone_Pillar", "150N 75W 0A", "Cypress"),
        ("Fountain", "100N 50W 0A", "Flagg"),
        ("Bridge_Wood", "80N 120W 0A", "Roland"),
    ];

    egui::ScrollArea::vertical()
        .id_salt("aw_objects")
        .show(ui, |ui| {
            for (name, pos, owner) in objects {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new(*name).strong());
                        ui.label(
                            RichText::new(format!("{pos}  —  {owner}"))
                                .small()
                                .color(Color32::GRAY),
                        );
                    });
                });
                ui.separator();
            }
        });
}

fn draw_properties(ui: &mut egui::Ui) {
    ui.strong("Properties");
    ui.separator();

    egui::Grid::new("aw_props")
        .num_columns(2)
        .spacing([8.0, 4.0])
        .striped(true)
        .show(ui, |ui| {
            let rows: &[(&str, &str)] = &[
                ("Name", "Castle"),
                ("Owner", "Cypress"),
                ("Position", "100N 100W 0A"),
                ("Rotation", "0.00 deg"),
                ("Scale", "1.00"),
                ("Visible", "Yes"),
                ("Solid", "Yes"),
                ("Action", "create sign msg=Welcome!"),
                ("Describe", "An ancient stone fortress."),
            ];
            for (k, v) in rows {
                ui.label(*k);
                ui.label(*v);
                ui.end_row();
            }
        });
}

fn draw_materials(ui: &mut egui::Ui) {
    ui.strong("Texture Library");
    ui.separator();

    let names = [
        "stone01",
        "wood_oak",
        "brick_red",
        "grass",
        "water_blue",
        "metal_rust",
        "sand_light",
        "marble_white",
        "glass_clear",
        "roof_tile",
    ];

    egui::ScrollArea::vertical()
        .id_salt("aw_mats")
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                for name in &names {
                    let (rect, response) =
                        ui.allocate_exact_size(Vec2::splat(72.0), Sense::hover());
                    let bg = if response.hovered() {
                        Color32::from_rgb(80, 100, 80)
                    } else {
                        Color32::from_rgb(55, 55, 55)
                    };
                    ui.painter().rect_filled(rect, 6.0, bg);
                    ui.painter().rect_stroke(
                        rect,
                        6.0,
                        Stroke::new(1.0, Color32::from_rgb(100, 100, 100)),
                        egui::StrokeKind::Inside,
                    );
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        name,
                        FontId::proportional(9.0),
                        Color32::LIGHT_GRAY,
                    );
                }
            });
        });
}

fn draw_worlds(ui: &mut egui::Ui, selected: &mut String) {
    ui.strong("World Browser");
    ui.separator();

    let worlds: &[(&str, &str, u32, bool)] = &[
        ("AW", "Active Worlds", 1203, true),
        ("AWTeen", "AW Teens", 847, true),
        ("Alphaworld", "Alpha World", 4521, false),
        ("Mars", "Mars Colony", 312, true),
        ("Atlantis", "Lost City", 89, false),
        ("Cy", "Cybergate", 551, true),
        ("Epoch", "Epoch Online", 203, false),
    ];

    egui::ScrollArea::vertical()
        .id_salt("aw_worlds")
        .show(ui, |ui| {
            for (short, name, users, public) in worlds {
                let is_sel = selected == short;
                let row_color = if is_sel {
                    Color32::from_rgb(40, 60, 90)
                } else {
                    Color32::TRANSPARENT
                };

                Frame::new().fill(row_color).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(*short)
                                .strong()
                                .color(Color32::from_rgb(100, 200, 255)),
                        );
                        ui.vertical(|ui| {
                            ui.label(*name);
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(format!("{users} online")).small());
                                let lock = if *public { "Public" } else { "Private" };
                                let lock_color = if *public {
                                    Color32::GREEN
                                } else {
                                    Color32::RED
                                };
                                ui.label(RichText::new(lock).small().color(lock_color));
                            });
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui.button("Enter").clicked() {
                                *selected = short.to_string();
                            }
                        });
                    });
                });
                ui.separator();
            }
        });
}

fn draw_avatar(ui: &mut egui::Ui, avatar_model: &mut String) {
    ui.strong("Avatar Settings");
    ui.separator();

    ui.horizontal(|ui| {
        ui.label("Citizen name:");
        ui.label(
            RichText::new("Flagg")
                .strong()
                .color(Color32::from_rgb(255, 200, 100)),
        );
    });
    ui.horizontal(|ui| {
        ui.label("Status:");
        ui.label(RichText::new("Active citizen").color(Color32::GREEN));
    });

    ui.add_space(8.0);
    ui.separator();
    ui.label(RichText::new("Movement").strong());

    egui::Grid::new("aw_avatar_move")
        .num_columns(2)
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            ui.label("Walk speed:");
            ui.label("5.0 m/s");
            ui.end_row();
            ui.label("Run speed:");
            ui.label("15.0 m/s");
            ui.end_row();
            ui.label("Fly speed:");
            ui.label("20.0 m/s");
            ui.end_row();
        });

    ui.add_space(8.0);
    ui.separator();
    ui.label(RichText::new("Appearance").strong());

    egui::ComboBox::from_label("Avatar model")
        .selected_text(avatar_model.as_str())
        .show_ui(ui, |ui| {
            for model in ["tourist", "cyborg", "wizard", "knight", "alien"] {
                ui.selectable_value(avatar_model, model.to_string(), model);
            }
        });
}
