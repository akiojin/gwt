//! gwt-frontend: Web frontend for Git Worktree Manager (Leptos CSR)

use leptos::prelude::*;
use leptos_router::{
    components::{Route, Router, Routes},
    path,
};

mod api;
mod components;
mod pages;

/// Main application component with routing
#[component]
pub fn App() -> impl IntoView {
    view! {
        <Router>
            <components::Header />
            <main class="container mx-auto p-4">
                <Routes fallback=|| view! { <pages::NotFound /> }>
                    <Route path=path!("/") view=pages::WorktreeList />
                    <Route path=path!("/worktrees") view=pages::WorktreeList />
                    <Route path=path!("/branches") view=pages::BranchList />
                    <Route path=path!("/settings") view=pages::Settings />
                </Routes>
            </main>
        </Router>
    }
}

/// Mount the application
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn main() {
    console_error_panic_hook::set_once();
    leptos::mount::mount_to_body(App);
}
