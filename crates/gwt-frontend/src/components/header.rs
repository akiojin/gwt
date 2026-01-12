//! Header component

use leptos::prelude::*;

#[component]
pub fn Header() -> impl IntoView {
    view! {
        <header class="bg-gray-800 text-white p-4">
            <h1 class="text-xl font-bold">"gwt"</h1>
        </header>
    }
}
