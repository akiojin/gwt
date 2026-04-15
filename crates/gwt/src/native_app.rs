pub const APP_NAME: &str = "GWT";
pub const MACOS_BUNDLE_IDENTIFIER: &str = "io.github.akiojin.gwt";
pub const OPEN_PROJECT_MENU_ID: &str = "file.open_project";
pub const RELOAD_MENU_ID: &str = "view.reload";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeMenuCommand {
    OpenProject,
    ReloadWebView,
}

pub fn macos_bundle_identifier() -> &'static str {
    MACOS_BUNDLE_IDENTIFIER
}

pub fn macos_native_menu_titles() -> &'static [&'static str] {
    &[APP_NAME, "File", "View", "Window"]
}

pub fn native_menu_command_for_id(menu_id: &str) -> Option<NativeMenuCommand> {
    match menu_id {
        OPEN_PROJECT_MENU_ID => Some(NativeMenuCommand::OpenProject),
        RELOAD_MENU_ID => Some(NativeMenuCommand::ReloadWebView),
        _ => None,
    }
}

#[cfg(target_os = "macos")]
pub struct MacosNativeMenu {
    menu_bar: muda::Menu,
    window_menu: muda::Submenu,
}

#[cfg(target_os = "macos")]
impl Default for MacosNativeMenu {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(target_os = "macos")]
impl MacosNativeMenu {
    pub fn new() -> Self {
        use muda::{
            accelerator::{Accelerator, Code, CMD_OR_CTRL},
            AboutMetadata, Menu, MenuItem, PredefinedMenuItem, Submenu,
        };

        let menu_bar = Menu::new();
        let app_menu = Submenu::new(APP_NAME, true);
        let file_menu = Submenu::new("File", true);
        let view_menu = Submenu::new("View", true);
        let window_menu = Submenu::new("Window", true);
        let open_project_item = MenuItem::with_id(
            OPEN_PROJECT_MENU_ID,
            "Open Project...",
            true,
            Some(Accelerator::new(Some(CMD_OR_CTRL), Code::KeyO)),
        );
        let reload_item = MenuItem::with_id(
            RELOAD_MENU_ID,
            "Reload",
            true,
            Some(Accelerator::new(Some(CMD_OR_CTRL), Code::KeyR)),
        );

        let _ = menu_bar.append_items(&[&app_menu, &file_menu, &view_menu, &window_menu]);
        let _ = app_menu.append_items(&[
            &PredefinedMenuItem::about(
                None,
                Some(AboutMetadata {
                    name: Some(APP_NAME.to_string()),
                    version: Some(env!("CARGO_PKG_VERSION").to_string()),
                    comments: Some(env!("CARGO_PKG_DESCRIPTION").to_string()),
                    ..Default::default()
                }),
            ),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::hide(None),
            &PredefinedMenuItem::hide_others(None),
            &PredefinedMenuItem::show_all(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::quit(None),
        ]);
        let _ = file_menu.append_items(&[
            &open_project_item,
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::close_window(None),
        ]);
        let _ = view_menu.append_items(&[&reload_item]);
        let _ = window_menu.append_items(&[
            &PredefinedMenuItem::minimize(None),
            &PredefinedMenuItem::maximize(None),
            &PredefinedMenuItem::bring_all_to_front(None),
        ]);

        Self {
            menu_bar,
            window_menu,
        }
    }

    pub fn init_for_app(&self) {
        self.menu_bar.init_for_nsapp();
        self.window_menu.set_as_windows_menu_for_nsapp();
    }
}
