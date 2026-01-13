//! Header component with navigation

use leptos::prelude::*;

/// Header component with navigation links
#[component]
pub fn Header() -> impl IntoView {
    view! {
        <header class="bg-gray-800 text-white">
            <div class="container mx-auto px-4 py-3 flex justify-between items-center">
                <a href="/" class="text-xl font-bold hover:text-gray-300">"gwt"</a>
                <nav class="flex gap-4">
                    <a href="/worktrees" class="hover:text-gray-300">"Worktrees"</a>
                    <a href="/branches" class="hover:text-gray-300">"Branches"</a>
                    <a href="/settings" class="hover:text-gray-300">"Settings"</a>
                </nav>
            </div>
        </header>
    }
}
