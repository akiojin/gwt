//! gwt-frontend: Web frontend for Git Worktree Manager (Leptos CSR)

use leptos::prelude::*;

mod api;
mod components;
mod pages;

/// Main application component
#[component]
pub fn App() -> impl IntoView {
    view! {
        <main class="container mx-auto p-4">
            <h1 class="text-2xl font-bold mb-4">"gwt - Git Worktree Manager"</h1>
            <pages::WorktreeList />
        </main>
    }
}

/// Mount the application
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn main() {
    console_error_panic_hook::set_once();
    leptos::mount::mount_to_body(App);
}
