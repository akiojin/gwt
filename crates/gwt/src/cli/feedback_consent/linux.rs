use gtk::prelude::*;

use super::{bounded_window_size, DECLINE_LABEL, FOOTER_HEIGHT, PADDING, SEND_LABEL, TITLE};

pub(super) fn confirm(prompt: &str) -> Result<bool, String> {
    gtk::init().map_err(|_| "Native feedback consent requires an available desktop display")?;
    let display = gtk::gdk::Display::default().ok_or("No desktop display is available")?;
    let monitor = display
        .primary_monitor()
        .or_else(|| display.monitor(0))
        .ok_or("No desktop monitor is available")?;
    let work = monitor.workarea();
    let size = bounded_window_size(work.width(), work.height())?;
    let dialog = gtk::Dialog::builder()
        .title(TITLE)
        .modal(true)
        .resizable(false)
        .default_width(size.width)
        .default_height(size.height - 48)
        .window_position(gtk::WindowPosition::Center)
        .build();
    dialog.add_button(DECLINE_LABEL, gtk::ResponseType::Cancel);
    dialog.add_button(SEND_LABEL, gtk::ResponseType::Accept);
    dialog.set_default_response(gtk::ResponseType::Cancel);
    let body = gtk::TextView::builder()
        .editable(false)
        .cursor_visible(false)
        .wrap_mode(gtk::WrapMode::WordChar)
        .left_margin(8)
        .right_margin(8)
        .build();
    body.buffer()
        .ok_or("Feedback text buffer is unavailable")?
        .set_text(prompt);
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Always)
        .min_content_height(size.height - 48 - FOOTER_HEIGHT - 2 * PADDING)
        .hexpand(true)
        .vexpand(true)
        .build();
    scroll.add(&body);
    let content = dialog.content_area();
    content.set_margin_start(PADDING);
    content.set_margin_end(PADDING);
    content.set_margin_top(PADDING);
    content.set_margin_bottom(PADDING);
    content.pack_start(&scroll, true, true, 0);
    dialog.show_all();
    let response = dialog.run();
    dialog.close();
    Ok(response == gtk::ResponseType::Accept)
}
