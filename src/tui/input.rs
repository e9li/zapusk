use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};

use crate::platform;
use super::app::{AddForm, App};

impl App {
    /// Handle a keyboard event (dispatched from tick after Ctrl+C check)
    pub(crate) async fn handle_key(&mut self, key: KeyEvent) -> Result<()> {
        // Add form input
        if let Some(ref mut form) = self.add_form {
            let is_type_field = matches!(form.field, super::app::AddField::Type);

            match key.code {
                KeyCode::Esc => {
                    self.add_form = None;
                    self.status_message = Some("Cancelled".into());
                }
                KeyCode::Enter => {
                    let val = form.current_value().to_string();
                    if val.is_empty() && matches!(form.field, super::app::AddField::Name | super::app::AddField::Path) {
                        self.status_message = Some(format!("{} cannot be empty", form.label()));
                        return Ok(());
                    }
                    if matches!(form.field, super::app::AddField::Path) && !std::path::Path::new(&val).is_dir() {
                        self.status_message = Some(format!("Directory not found: {}", val));
                        return Ok(());
                    }
                    let tld = self.config.tld.clone();
                    if form.next_field(&tld) {
                        let completed = self.add_form.take().unwrap();
                        self.finalize_add(completed).await;
                    }
                }
                // Type field: cycle with arrows/tab, no freetext
                KeyCode::Right | KeyCode::Tab if is_type_field => {
                    form.cycle_type_next();
                }
                KeyCode::Left | KeyCode::BackTab if is_type_field => {
                    form.cycle_type_prev();
                }
                // Other fields: freetext
                KeyCode::Backspace if !is_type_field => {
                    form.current_value_mut().pop();
                }
                KeyCode::Char(c) if !is_type_field => {
                    form.current_value_mut().push(c);
                }
                _ => {}
            }
            return Ok(());
        }

        // Search mode input
        if self.search_mode {
            match key.code {
                KeyCode::Esc => {
                    self.search_mode = false;
                    self.search_query.clear();
                }
                KeyCode::Enter => {
                    self.search_mode = false;
                }
                KeyCode::Backspace => {
                    self.search_query.pop();
                }
                KeyCode::Char(c) => {
                    self.search_query.push(c);
                }
                _ => {}
            }
            return Ok(());
        }

        // Confirmation dialog
        if let Some(dialog) = self.confirm_dialog.take() {
            match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    self.execute_confirm(dialog.action).await;
                }
                _ => {
                    self.status_message = Some("Cancelled".into());
                }
            }
            return Ok(());
        }

        // Help popup
        if self.show_help {
            match key.code {
                KeyCode::Esc | KeyCode::Enter | KeyCode::Char('?') | KeyCode::Char('q') => {
                    self.show_help = false;
                }
                _ => {}
            }
            return Ok(());
        }

        // Detail popup
        if self.show_detail_popup {
            match key.code {
                KeyCode::Esc | KeyCode::Enter | KeyCode::Char('d') => {
                    self.show_detail_popup = false;
                }
                _ => {}
            }
            return Ok(());
        }

        match key.code {
            // Quit
            KeyCode::Char('q') => self.quit().await,

            // Navigation
            KeyCode::Down | KeyCode::Char('j') => self.select_next(),
            KeyCode::Up | KeyCode::Char('k') => self.select_prev(),

            // Switch panes
            KeyCode::Tab => self.toggle_pane(),

            // Project actions
            KeyCode::Char('s') => self.start_selected().await,
            KeyCode::Char('x') => self.confirm_stop_selected(),
            KeyCode::Char('r') => self.restart_selected().await,

            // Caddy
            KeyCode::Char('R') => self.reload_caddy().await,

            // Log scrolling
            KeyCode::PageUp => self.scroll_logs_up(10),
            KeyCode::PageDown => self.scroll_logs_down(10),
            KeyCode::End | KeyCode::Char('G') => {
                self.log_scroll_offset = 0;
            }

            // Search
            KeyCode::Char('/') => {
                self.search_mode = true;
                self.search_query.clear();
            }

            // Add project
            KeyCode::Char('a') => {
                self.add_form = Some(AddForm::new());
                self.status_message = Some("Adding project...".into());
            }

            // Remove project
            KeyCode::Char('D') => self.confirm_remove_selected(),

            // Help
            KeyCode::Char('?') => {
                self.show_help = true;
            }

            // Detail popup
            KeyCode::Char('d') => {
                self.show_detail_popup = true;
            }

            // Open in browser
            KeyCode::Char('o') => self.open_in_browser(),

            // Copy domain to clipboard
            KeyCode::Char('c') => self.copy_domain(),

            _ => {}
        }
        Ok(())
    }

    fn copy_domain(&mut self) {
        if let Some(project) = self.selected_project() {
            let domain = project.config.domain.clone();
            let _ = platform::copy_to_clipboard(&domain);
            self.status_message = Some(format!("Copied {}", domain));
        }
    }

    fn open_in_browser(&self) {
        if let Some(project) = self.selected_project() {
            let scheme = if project.config.tls { "https" } else { "http" };
            let url = format!("{}://{}", scheme, project.config.domain);
            let _ = platform::open_url(&url);
        }
    }
}
