use super::*;

impl Model {
    pub(super) fn handle_ai_wizard_mouse(&mut self, mouse: MouseEvent) {
        if !self.ai_wizard.visible {
            return;
        }
        if !matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
            return;
        }

        // Check if clicking outside popup area - close wizard
        if !self.ai_wizard.is_point_in_popup(mouse.column, mouse.row) {
            self.ai_wizard.close();
            self.ai_wizard_rx = None;
            self.last_mouse_click = None;
            return;
        }

        // Only handle model selection step mouse clicks
        if !matches!(
            self.ai_wizard.step,
            crate::tui::screens::ai_wizard::AIWizardStep::ModelSelect
        ) {
            return;
        }

        // Check if clicking on model list
        if let Some(index) = self
            .ai_wizard
            .selection_index_from_point(mouse.column, mouse.row)
        {
            let now = Instant::now();
            let is_double_click = self.last_mouse_click.as_ref().is_some_and(|last| {
                last.index == index
                    && now.duration_since(last.at) <= BRANCH_LIST_DOUBLE_CLICK_WINDOW
            });

            if self.ai_wizard.select_model_index(index) {
                if is_double_click {
                    self.handle_ai_wizard_enter();
                }
            } else {
                self.ai_wizard.select_model_index(index);
            }

            self.last_mouse_click = Some(MouseClick { index, at: now });
        }
    }

    /// Handle Enter key in AI settings wizard
    pub(super) fn handle_ai_wizard_enter(&mut self) {
        use crate::tui::screens::ai_wizard::AIWizardStep;

        match self.ai_wizard.step {
            AIWizardStep::Endpoint => {
                self.ai_wizard.next_step();
            }
            AIWizardStep::ApiKey => {
                // Start fetching models
                self.ai_wizard.step = AIWizardStep::FetchingModels;
                self.ai_wizard.loading_message = Some("Fetching models...".to_string());

                let endpoint = self.ai_wizard.endpoint.trim().to_string();
                let api_key = self.ai_wizard.api_key.trim().to_string();
                let (tx, rx) = mpsc::channel();
                self.ai_wizard_rx = Some(rx);

                thread::spawn(move || {
                    let result = AIClient::new_for_list_models(&endpoint, &api_key)
                        .and_then(|client| client.list_models());
                    let _ = tx.send(AiWizardUpdate { result });
                });
            }
            AIWizardStep::FetchingModels => {
                // Do nothing while fetching
            }
            AIWizardStep::ModelSelect => {
                // Save AI settings
                self.save_ai_wizard_settings();
                self.ai_wizard.close();
                self.ai_wizard_rx = None;
                if let Some(prev_screen) = self.screen_stack.pop() {
                    self.screen = prev_screen;
                }
                self.load_profiles();
            }
        }
    }

    pub(super) fn apply_ai_wizard_updates(&mut self) {
        let Some(rx) = self.ai_wizard_rx.take() else {
            return;
        };

        loop {
            match rx.try_recv() {
                Ok(update) => {
                    if !self.ai_wizard.visible {
                        break;
                    }
                    match update.result {
                        Ok(models) => {
                            if let Err(err) = self.ai_wizard.apply_models(models) {
                                self.ai_wizard.fetch_failed(&err);
                            } else {
                                self.ai_wizard.fetch_complete();
                            }
                        }
                        Err(err) => {
                            self.ai_wizard.fetch_failed(&err);
                        }
                    }
                    break;
                }
                Err(TryRecvError::Empty) => {
                    self.ai_wizard_rx = Some(rx);
                    break;
                }
                Err(TryRecvError::Disconnected) => {
                    break;
                }
            }
        }
    }

    /// Save AI settings from wizard
    fn save_ai_wizard_settings(&mut self) {
        let model = self
            .ai_wizard
            .current_model()
            .map(|m| m.id.clone())
            .unwrap_or_default();
        let settings = AISettings {
            endpoint: self.ai_wizard.endpoint.trim().to_string(),
            api_key: self.ai_wizard.api_key.trim().to_string(),
            model,
        };

        if self.ai_wizard.is_default_ai {
            self.profiles_config.default_ai = Some(settings);
        } else if let Some(profile_name) = &self.ai_wizard.profile_name {
            if let Some(profile) = self.profiles_config.profiles.get_mut(profile_name) {
                profile.ai = Some(settings);
            }
        }
        self.save_profiles();
    }

    /// Delete AI settings from wizard
    pub(super) fn delete_ai_wizard_settings(&mut self) {
        if self.ai_wizard.is_default_ai {
            self.profiles_config.default_ai = None;
        } else if let Some(profile_name) = &self.ai_wizard.profile_name {
            if let Some(profile) = self.profiles_config.profiles.get_mut(profile_name) {
                profile.ai = None;
            }
        }
        self.save_profiles();
        self.ai_wizard.close();
        self.ai_wizard_rx = None;
        if let Some(prev_screen) = self.screen_stack.pop() {
            self.screen = prev_screen;
        }
        self.load_profiles();
    }
}
