//! Window for choosing a Claude account and opening the desktop app.

use std::ffi::OsString;
use std::path::PathBuf;

use eframe::egui;

#[derive(Clone, Copy, PartialEq, Eq)]
enum StatusKind {
    Info,
    Success,
    Error,
}

pub fn run(
    home: PathBuf,
    preset: Option<String>,
    claude_args: Vec<OsString>,
) -> Result<(), String> {
    let mut status = String::new();
    let mut status_kind = StatusKind::Info;
    if let Some(name) = &preset {
        let dir = super::config_dir(&home, name);
        std::fs::create_dir_all(&dir)
            .map_err(|err| format!("cannot create {}: {err}", dir.display()))?;
        status = format!("Account \"{name}\" is ready.");
        status_kind = StatusKind::Success;
    }
    let accounts = super::list_accounts(&home);
    let selected = preset
        .as_ref()
        .and_then(|name| accounts.iter().position(|account| account == name))
        .unwrap_or(0);
    let app = ClaudinWindow {
        home,
        accounts,
        selected,
        new_name: String::new(),
        status,
        status_kind,
        confirm_remove: false,
        missing_app: false,
        claude_args,
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Claudin")
            .with_inner_size([380.0, 360.0])
            .with_min_inner_size([340.0, 310.0])
            .with_resizable(false)
            .with_maximize_button(false),
        centered: true,
        ..Default::default()
    };
    eframe::run_native(
        "Claudin",
        options,
        Box::new(|cc| {
            cc.egui_ctx.all_styles_mut(|style| {
                style.spacing.item_spacing = egui::vec2(8.0, 8.0);
                style.spacing.button_padding = egui::vec2(10.0, 5.0);
            });
            Ok(Box::new(app))
        }),
    )
    .map_err(|err| err.to_string())
}

struct ClaudinWindow {
    home: PathBuf,
    accounts: Vec<String>,
    selected: usize,
    new_name: String,
    status: String,
    status_kind: StatusKind,
    confirm_remove: bool,
    missing_app: bool,
    claude_args: Vec<OsString>,
}

impl ClaudinWindow {
    fn create_account(&mut self) {
        let name = self.new_name.trim().to_owned();
        if name.is_empty() {
            self.status = "Please enter an account name.".to_owned();
            self.status_kind = StatusKind::Error;
            return;
        }
        if !super::is_valid_name(&name) {
            self.status = "Use only letters, digits, '-', '_' or '.'.".to_owned();
            self.status_kind = StatusKind::Error;
            return;
        }
        let dir = super::config_dir(&self.home, &name);
        if let Err(err) = std::fs::create_dir_all(&dir) {
            self.status = format!("Cannot create {}: {err}", dir.display());
            self.status_kind = StatusKind::Error;
            return;
        }
        self.accounts = super::list_accounts(&self.home);
        self.selected = self
            .accounts
            .iter()
            .position(|account| account == &name)
            .unwrap_or(0);
        self.new_name.clear();
        self.confirm_remove = false;
        self.status = format!("Created \"{name}\".");
        self.status_kind = StatusKind::Success;
    }

    fn remove_selected(&mut self) {
        let Some(name) = self.accounts.get(self.selected).cloned() else {
            self.confirm_remove = false;
            return;
        };
        if let Err(err) = super::delete_account(&self.home, &name) {
            self.status = format!("Cannot remove {name}: {err}");
            self.status_kind = StatusKind::Error;
            return;
        }
        self.accounts = super::list_accounts(&self.home);
        if self.accounts.is_empty() {
            self.selected = 0;
        } else if self.selected >= self.accounts.len() {
            self.selected = self.accounts.len() - 1;
        }
        self.confirm_remove = false;
        self.status = format!("Removed \"{name}\".");
        self.status_kind = StatusKind::Info;
    }

    fn open_claude(&mut self, ctx: &egui::Context) {
        let Some(name) = self.accounts.get(self.selected).cloned() else {
            self.status = "Create an account first.".to_owned();
            self.status_kind = StatusKind::Error;
            return;
        };
        let dir = super::config_dir(&self.home, &name);
        if let Err(err) = std::fs::create_dir_all(&dir) {
            self.status = format!("Cannot create {}: {err}", dir.display());
            self.status_kind = StatusKind::Error;
            return;
        }
        let Some(program) = super::gui_binary(&self.home) else {
            self.missing_app = true;
            self.status = super::gui_missing_message();
            self.status_kind = StatusKind::Error;
            return;
        };
        self.missing_app = false;
        match super::spawn_in_background(&program, &dir, &self.claude_args) {
            Ok(_) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Err(err) => {
                self.status = format!("Failed to start {}: {err}", program.display());
                self.status_kind = StatusKind::Error;
            }
        }
    }
}

impl eframe::App for ClaudinWindow {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let typing = ctx.memory(|mem| mem.focused().is_some());
        if !typing {
            if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
                if self.confirm_remove {
                    self.confirm_remove = false;
                } else {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            } else if !self.accounts.is_empty() {
                if ctx.input(|input| input.key_pressed(egui::Key::ArrowDown)) {
                    self.selected = (self.selected + 1).min(self.accounts.len() - 1);
                    self.confirm_remove = false;
                }
                if ctx.input(|input| input.key_pressed(egui::Key::ArrowUp)) && self.selected > 0 {
                    self.selected -= 1;
                    self.confirm_remove = false;
                }
                if ctx.input(|input| input.key_pressed(egui::Key::Enter)) && !self.confirm_remove {
                    self.open_claude(&ctx);
                }
            }
        }

        let frame = egui::Frame::central_panel(ui.style()).inner_margin(16.0);
        egui::CentralPanel::default().frame(frame).show(ui, |ui| {
            ui.heading("Claude Accounts");
            ui.label(
                egui::RichText::new("Select an account to launch the Claude desktop app.")
                    .weak()
                    .size(12.5),
            );
            ui.add_space(6.0);

            egui::Frame::group(ui.style())
                .inner_margin(8.0)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    if self.accounts.is_empty() {
                        ui.vertical_centered(|ui| {
                            ui.add_space(28.0);
                            ui.label(
                                egui::RichText::new(
                                    "No accounts yet. Create one below to get started.",
                                )
                                .weak(),
                            );
                            ui.add_space(28.0);
                        });
                    } else {
                        let mut double_clicked = false;
                        egui::ScrollArea::vertical()
                            .max_height(115.0)
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                for (index, name) in self.accounts.iter().enumerate() {
                                    let is_selected = self.selected == index;
                                    let response = ui.add_sized(
                                        [ui.available_width(), 26.0],
                                        egui::Button::selectable(
                                            is_selected,
                                            egui::RichText::new(format!("  {name}")).size(14.0),
                                        ),
                                    );
                                    if response.clicked() {
                                        self.selected = index;
                                        self.confirm_remove = false;
                                    }
                                    if response.double_clicked() {
                                        self.selected = index;
                                        self.confirm_remove = false;
                                        double_clicked = true;
                                    }
                                }
                            });
                        if double_clicked {
                            self.open_claude(&ctx);
                        }
                    }
                });

            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let response = ui.add(
                    egui::TextEdit::singleline(&mut self.new_name)
                        .hint_text("New account name (e.g. work, personal)")
                        .desired_width(ui.available_width() - 78.0),
                );
                let can_create = !self.new_name.trim().is_empty();
                let clicked = ui
                    .add_sized([70.0, 24.0], egui::Button::new("Create"))
                    .clicked();
                let pressed_enter =
                    response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
                if clicked || (pressed_enter && can_create) {
                    self.create_account();
                }
            });

            if !self.status.is_empty() {
                let color = match self.status_kind {
                    StatusKind::Info => ui.visuals().text_color(),
                    StatusKind::Success => egui::Color32::from_rgb(46, 160, 67),
                    StatusKind::Error => ui.visuals().error_fg_color,
                };
                ui.label(egui::RichText::new(&self.status).color(color).size(12.0));
                if self.missing_app {
                    ui.hyperlink(super::GUI_INSTALL_URL);
                }
            }

            ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                if self.confirm_remove
                    && let Some(name) = self.accounts.get(self.selected).cloned()
                {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(format!("Delete \"{name}\"?"))
                                .color(ui.visuals().error_fg_color)
                                .strong(),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("Cancel").clicked() {
                                self.confirm_remove = false;
                            }
                            let delete_btn = egui::Button::new(
                                egui::RichText::new("Confirm Delete").color(egui::Color32::WHITE),
                            )
                            .fill(egui::Color32::from_rgb(200, 45, 45));
                            if ui.add(delete_btn).clicked() {
                                self.remove_selected();
                            }
                        });
                    });
                } else {
                    let has_account = !self.accounts.is_empty();
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(has_account, egui::Button::new("Remove..."))
                            .clicked()
                        {
                            self.confirm_remove = true;
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let label = match self.accounts.get(self.selected) {
                                Some(name) => format!("Open Claude ({name})"),
                                None => "Open Claude".to_owned(),
                            };
                            let open_btn = egui::Button::new(egui::RichText::new(label).strong());
                            if ui.add_enabled(has_account, open_btn).clicked() {
                                self.open_claude(&ctx);
                            }
                        });
                    });
                }
                ui.separator();
            });
        });
    }
}
