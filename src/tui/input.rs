use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};

use super::app::{App, FormField, ProjectForm};
use crate::i18n::Msg;
use crate::platform;

impl App {
    /// Handle a keyboard event (dispatched from tick after Ctrl+C check)
    pub(crate) async fn handle_key(&mut self, key: KeyEvent) -> Result<()> {
        // Add/edit form input
        if self.form.is_some() {
            let is_selector_field = self
                .form
                .as_ref()
                .map(|f| f.is_selector_field())
                .unwrap_or(false);
            let field = self.form.as_ref().map(|f| f.field);

            match key.code {
                KeyCode::Esc => {
                    self.form = None;
                    self.status_message = Some(self.tr(Msg::Cancelled).into());
                }
                KeyCode::Enter => {
                    let val = self
                        .form
                        .as_ref()
                        .map(|f| f.current_value().to_string())
                        .unwrap_or_default();
                    if val.is_empty() && matches!(field, Some(FormField::Name | FormField::Path)) {
                        let field_msg = self.form.as_ref().unwrap().label_msg();
                        let field = self.lang.tr(field_msg).to_string();
                        self.status_message = Some(self.trf(Msg::FieldEmpty, &[("field", &field)]));
                        return Ok(());
                    }
                    if matches!(field, Some(FormField::Path))
                        && !std::path::Path::new(&val).is_dir()
                    {
                        self.status_message = Some(self.trf(Msg::DirNotFound, &[("path", &val)]));
                        return Ok(());
                    }
                    if matches!(field, Some(FormField::PhpVersion))
                        && val.trim().is_empty()
                        && self.form.as_ref().map(|f| f.is_add()).unwrap_or(false)
                    {
                        self.status_message = Some(self.tr(Msg::PhpVersionEmpty).into());
                        return Ok(());
                    }
                    let tld = self.config.tld.clone();
                    let done = {
                        let form = self.form.as_mut().unwrap();
                        form.next_field(&tld, &self.frameworks)
                    };
                    if done {
                        if let Some(completed) = self.form.take() {
                            self.finalize_form(completed).await;
                        }
                    }
                }
                KeyCode::Right | KeyCode::Tab if is_selector_field => {
                    if let Some(form) = self.form.as_mut() {
                        if matches!(form.field, FormField::Type) {
                            form.cycle_type_next();
                            form.apply_type_defaults(&self.frameworks);
                        } else {
                            form.toggle_current_selector();
                        }
                    }
                }
                KeyCode::Left | KeyCode::BackTab if is_selector_field => {
                    if let Some(form) = self.form.as_mut() {
                        if matches!(form.field, FormField::Type) {
                            form.cycle_type_prev();
                            form.apply_type_defaults(&self.frameworks);
                        } else {
                            form.toggle_current_selector();
                        }
                    }
                }
                KeyCode::Backspace if !is_selector_field => {
                    if let Some(form) = self.form.as_mut() {
                        form.current_value_mut().pop();
                    }
                }
                KeyCode::Char(c) if !is_selector_field => {
                    if let Some(form) = self.form.as_mut() {
                        form.current_value_mut().push(c);
                    }
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
                    self.status_message = Some(self.tr(Msg::Cancelled).into());
                }
            }
            return Ok(());
        }

        // Language picker
        if self.show_language_popup {
            match key.code {
                KeyCode::Esc | KeyCode::Char('l') => self.cancel_language_picker(),
                KeyCode::Down | KeyCode::Char('j') => self.select_language_next(),
                KeyCode::Up | KeyCode::Char('k') => self.select_language_prev(),
                KeyCode::Enter => self.apply_language_selection(),
                _ => {}
            }
            return Ok(());
        }

        // Theme picker
        if self.show_theme_popup {
            match key.code {
                KeyCode::Esc | KeyCode::Char('t') => self.cancel_theme_picker(),
                KeyCode::Down | KeyCode::Char('j') => self.select_theme_next(),
                KeyCode::Up | KeyCode::Char('k') => self.select_theme_prev(),
                KeyCode::Enter => self.apply_theme_selection(),
                _ => {}
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
                KeyCode::Char('D') | KeyCode::Delete => {
                    self.show_detail_popup = false;
                    self.confirm_remove_selected();
                }
                _ => {}
            }
            return Ok(());
        }

        // Unmanaged services popup
        if self.show_unmanaged_popup {
            match key.code {
                KeyCode::Esc | KeyCode::Char('u') => {
                    if self.show_unmanaged_detail {
                        self.show_unmanaged_detail = false;
                    } else {
                        self.show_unmanaged_popup = false;
                    }
                }
                KeyCode::Down | KeyCode::Char('j') => self.select_unmanaged_next(),
                KeyCode::Up | KeyCode::Char('k') => self.select_unmanaged_prev(),
                KeyCode::Enter => {
                    self.show_unmanaged_detail = !self.show_unmanaged_detail;
                }
                KeyCode::Char('r') => self.refresh_unmanaged().await,
                KeyCode::Char('f') => self.toggle_unmanaged_filter(),
                KeyCode::Char('w') => self.toggle_unmanaged_web_filter(),
                KeyCode::Char('i') => self.import_selected_unmanaged().await,
                KeyCode::Char('I') => self.ignore_selected_unmanaged().await,
                _ => {}
            }
            return Ok(());
        }

        match key.code {
            // Quit
            KeyCode::Char('q') => self.quit().await,
            KeyCode::Char('Q') => self.force_quit().await,

            // Navigation — pane-aware
            KeyCode::Down | KeyCode::Char('j') => match self.active_pane {
                super::app::ActivePane::ProjectList => self.select_next(),
                super::app::ActivePane::Logs => self.scroll_logs_down(1),
            },
            KeyCode::Up | KeyCode::Char('k') => match self.active_pane {
                super::app::ActivePane::ProjectList => self.select_prev(),
                super::app::ActivePane::Logs => self.scroll_logs_up(1),
            },

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
                self.form = Some(ProjectForm::new_add(self.frameworks.ids()));
                self.status_message = Some(self.tr(Msg::AddingProject).into());
            }

            // Edit project
            KeyCode::Char('e') => self.start_edit_selected(),

            // Remove project
            KeyCode::Char('D') | KeyCode::Delete => self.confirm_remove_selected(),

            // Language picker
            KeyCode::Char('l') => self.open_language_picker(),

            // Theme picker
            KeyCode::Char('t') => self.open_theme_picker(),

            // Help
            KeyCode::Char('?') => {
                self.show_help = true;
            }

            // Unmanaged services
            KeyCode::Char('u') => self.toggle_unmanaged_popup().await,

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
            match platform::copy_to_clipboard(&domain) {
                Ok(()) => self.status_message = Some(self.trf(Msg::Copied, &[("domain", &domain)])),
                Err(e) => {
                    self.status_message =
                        Some(self.trf(Msg::ClipboardError, &[("error", &e.to_string())]))
                }
            }
        }
    }

    fn open_in_browser(&mut self) {
        if let Some(project) = self.selected_project() {
            let scheme = if project.config.tls { "https" } else { "http" };
            let url = format!("{}://{}", scheme, project.config.domain);
            if let Err(e) = platform::open_url(&url) {
                self.status_message =
                    Some(self.trf(Msg::BrowserError, &[("error", &e.to_string())]));
            }
        }
    }
}
