//! Window for choosing a Claude account and opening the desktop app.

use std::ffi::OsString;
use std::path::PathBuf;

use eframe::egui;

pub fn run(
    home: PathBuf,
    preset: Option<String>,
    claude_args: Vec<OsString>,
) -> Result<(), String> {
    let mut status = String::new();
    if let Some(name) = &preset {
        let dir = super::config_dir(&home, name);
        std::fs::create_dir_all(&dir)
            .map_err(|err| format!("cannot create {}: {err}", dir.display()))?;
        status = format!("Account \"{name}\" is ready.");
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
        confirm_remove: false,
        missing_app: false,
        claude_args,
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Claudin")
            .with_inner_size([440.0, 520.0])
            .with_min_inner_size([360.0, 360.0]),
        ..Default::default()
    };
    eframe::run_native("Claudin", options, Box::new(|_cc| Ok(Box::new(app))))
        .map_err(|err| err.to_string())
}

struct ClaudinWindow {
    home: PathBuf,
    accounts: Vec<String>,
    selected: usize,
    new_name: String,
    status: String,
    confirm_remove: bool,
    missing_app: bool,
    claude_args: Vec<OsString>,
}

impl ClaudinWindow {
    fn create_account(&mut self) {
        let name = self.new_name.trim().to_owned();
        if !super::is_valid_name(&name) {
            self.status = "Use only letters, digits, '-', '_' or '.'.".to_owned();
            return;
        }
        let dir = super::config_dir(&self.home, &name);
        if let Err(err) = std::fs::create_dir_all(&dir) {
            self.status = format!("cannot create {}: {err}", dir.display());
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
    }

    fn remove_selected(&mut self) {
        let Some(name) = self.accounts.get(self.selected).cloned() else {
            self.confirm_remove = false;
            return;
        };
        if let Err(err) = super::delete_account(&self.home, &name) {
            self.status = format!("cannot remove {name}: {err}");
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
    }

    fn open_claude(&mut self, ctx: &egui::Context) {
        let Some(name) = self.accounts.get(self.selected).cloned() else {
            self.status = "Create an account first.".to_owned();
            return;
        };
        let dir = super::config_dir(&self.home, &name);
        if let Err(err) = std::fs::create_dir_all(&dir) {
            self.status = format!("cannot create {}: {err}", dir.display());
            return;
        }
        let Some(program) = super::gui_binary(&self.home) else {
            self.missing_app = true;
            self.status = super::gui_missing_message();
            return;
        };
        self.missing_app = false;
        match super::spawn_in_background(&program, &dir, &self.claude_args) {
            Ok(_) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Err(err) => {
                self.status = format!("failed to start {}: {err}", program.display());
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

        egui::CentralPanel::default().show(ui, |ui| {
            ui.heading("Claude accounts");
            ui.label("Choose an account, then open the Claude desktop app.");
            ui.add_space(8.0);

            if self.accounts.is_empty() {
                ui.label("No accounts yet.");
            } else {
                egui::ScrollArea::vertical()
                    .max_height(220.0)
                    .show(ui, |ui| {
                        for (index, name) in self.accounts.iter().enumerate() {
                            if ui.selectable_label(self.selected == index, name).clicked() {
                                self.selected = index;
                                self.confirm_remove = false;
                            }
                        }
                    });
            }

            ui.add_space(12.0);
            ui.label("New account");
            ui.horizontal(|ui| {
                let response = ui.add(
                    egui::TextEdit::singleline(&mut self.new_name)
                        .hint_text("name")
                        .desired_width(220.0),
                );
                let submit = ui.button("Create").clicked()
                    || (response.lost_focus()
                        && ui.input(|input| input.key_pressed(egui::Key::Enter)));
                if submit {
                    self.create_account();
                }
            });
            ui.label("Letters, digits, '-', '_' and '.'.");

            ui.add_space(12.0);
            ui.horizontal(|ui| {
                let has_account = !self.accounts.is_empty();
                if ui
                    .add_enabled(has_account, egui::Button::new("Remove"))
                    .clicked()
                {
                    self.confirm_remove = true;
                }
                if ui
                    .add_enabled(has_account, egui::Button::new("Open Claude"))
                    .clicked()
                {
                    self.open_claude(&ctx);
                }
            });

            if self.confirm_remove
                && let Some(name) = self.accounts.get(self.selected)
            {
                ui.add_space(8.0);
                ui.label(format!(
                    "Delete account \"{name}\" and its config directory?"
                ));
                ui.horizontal(|ui| {
                    if ui.button("Delete").clicked() {
                        self.remove_selected();
                    }
                    if ui.button("Cancel").clicked() {
                        self.confirm_remove = false;
                    }
                });
            }

            if !self.status.is_empty() {
                ui.add_space(12.0);
                ui.label(&self.status);
            }
            if self.missing_app {
                ui.hyperlink(super::GUI_INSTALL_URL);
            }
        });
    }
}
