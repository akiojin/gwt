//! Terminal page with WebSocket-backed PTY

use leptos::prelude::*;
use serde::Deserialize;
use serde_json::json;
use std::rc::Rc;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{CloseEvent, Event, MessageEvent, WebSocket};

#[wasm_bindgen]
extern "C" {
    type Terminal;

    #[wasm_bindgen(constructor)]
    fn new() -> Terminal;

    #[wasm_bindgen(method)]
    fn open(this: &Terminal, element: web_sys::Element);

    #[wasm_bindgen(method)]
    fn write(this: &Terminal, data: &str);

    #[wasm_bindgen(method)]
    fn onData(this: &Terminal, callback: &js_sys::Function);

    #[wasm_bindgen(method)]
    fn onResize(this: &Terminal, callback: &js_sys::Function);

    #[wasm_bindgen(method)]
    fn loadAddon(this: &Terminal, addon: &FitAddon);
}

#[wasm_bindgen]
extern "C" {
    type FitAddon;

    #[wasm_bindgen(constructor)]
    fn new() -> FitAddon;

    #[wasm_bindgen(method)]
    fn fit(this: &FitAddon);
}

#[wasm_bindgen]
extern "C" {
    type ResizeEvent;

    #[wasm_bindgen(method, getter)]
    fn cols(this: &ResizeEvent) -> u16;

    #[wasm_bindgen(method, getter)]
    fn rows(this: &ResizeEvent) -> u16;
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ServerMessage {
    Ready { session_id: String },
    Output { data: String },
    Error { message: String },
}

#[component]
pub fn Terminal() -> impl IntoView {
    let container = NodeRef::<leptos::html::Div>::new();
    let status = RwSignal::new("Connecting...".to_string());
    let last_error = RwSignal::new(None::<String>);

    Effect::new(move |_| {
        let Some(element) = container.get() else {
            return;
        };

        let terminal = Rc::new(Terminal::new());
        let fit_addon = Rc::new(FitAddon::new());
        terminal.loadAddon(fit_addon.as_ref());
        terminal.open(element.unchecked_into());
        fit_addon.fit();

        let window = web_sys::window().expect("window");
        let location = window.location();
        let host = location
            .host()
            .unwrap_or_else(|_| "127.0.0.1:3000".to_string());
        let protocol = location.protocol().unwrap_or_else(|_| "http:".to_string());
        let ws_scheme = if protocol == "https:" { "wss" } else { "ws" };
        let ws_url = format!("{ws_scheme}://{host}/ws/terminal");

        let ws = Rc::new(WebSocket::new(&ws_url).expect("websocket"));

        let on_open_status = status;
        let on_open = Closure::wrap(Box::new(move |_event: Event| {
            on_open_status.set("Connected".to_string());
        }) as Box<dyn FnMut(_)>);
        ws.set_onopen(Some(on_open.as_ref().unchecked_ref()));
        on_open.forget();

        let on_message_terminal = Rc::clone(&terminal);
        let on_message_status = status;
        let on_message_error = last_error;
        let on_message = Closure::wrap(Box::new(move |event: MessageEvent| {
            let Some(text) = event.data().as_string() else {
                return;
            };

            match serde_json::from_str::<ServerMessage>(&text) {
                Ok(ServerMessage::Ready {
                    session_id: _session_id,
                }) => {
                    on_message_status.set("Connected".to_string());
                }
                Ok(ServerMessage::Output { data }) => {
                    on_message_terminal.write(&data);
                }
                Ok(ServerMessage::Error { message }) => {
                    on_message_error.set(Some(message));
                }
                Err(_) => {
                    on_message_error.set(Some("Invalid server message.".to_string()));
                }
            }
        }) as Box<dyn FnMut(_)>);
        ws.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
        on_message.forget();

        let on_close_status = status;
        let on_close = Closure::wrap(Box::new(move |_event: CloseEvent| {
            on_close_status.set("Disconnected".to_string());
        }) as Box<dyn FnMut(_)>);
        ws.set_onclose(Some(on_close.as_ref().unchecked_ref()));
        on_close.forget();

        let input_ws = Rc::clone(&ws);
        let on_data = Closure::wrap(Box::new(move |data: String| {
            let payload = json!({ "type": "input", "data": data });
            let _ = input_ws.send_with_str(&payload.to_string());
        }) as Box<dyn FnMut(_)>);
        terminal.onData(on_data.as_ref().unchecked_ref());
        on_data.forget();

        let resize_ws = Rc::clone(&ws);
        let on_resize = Closure::wrap(Box::new(move |event: ResizeEvent| {
            let payload = json!({
                "type": "resize",
                "cols": event.cols(),
                "rows": event.rows(),
            });
            let _ = resize_ws.send_with_str(&payload.to_string());
        }) as Box<dyn FnMut(_)>);
        terminal.onResize(on_resize.as_ref().unchecked_ref());
        on_resize.forget();

        let resize_fit = Rc::clone(&fit_addon);
        let on_window_resize = Closure::wrap(Box::new(move |_event: Event| {
            resize_fit.fit();
        }) as Box<dyn FnMut(_)>);
        let _ = window
            .add_event_listener_with_callback("resize", on_window_resize.as_ref().unchecked_ref());
        on_window_resize.forget();
    });

    view! {
        <div class="space-y-6">
            <div class="panel rounded-2xl p-6 flex flex-col gap-2 md:flex-row md:items-center md:justify-between">
                <div>
                    <p class="text-xs uppercase tracking-[0.2em] text-slate-500">"Session"</p>
                    <h2 class="brand-title text-2xl font-semibold">"Terminal"</h2>
                </div>
                <span class="text-sm text-slate-500">{move || status.get()}</span>
            </div>
            {move || {
                last_error.get().map(|err| view! {
                    <div class="rounded-2xl bg-red-50 border border-red-200 text-red-700 px-4 py-2 text-sm">
                        {err}
                    </div>
                })
            }}
            <div
                node_ref=container
                class="panel rounded-2xl bg-black text-white h-[70vh] w-full overflow-hidden"
            ></div>
        </div>
    }
}
