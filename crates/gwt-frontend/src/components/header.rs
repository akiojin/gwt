//! Header component with navigation

use leptos::prelude::*;

/// Header component with navigation links
#[component]
pub fn Header() -> impl IntoView {
    view! {
        <header class="sticky top-0 z-10 bg-white/80 backdrop-blur border-b border-black/10">
            <div class="container mx-auto px-4 py-4 flex flex-col gap-3 md:flex-row md:justify-between md:items-center">
                <a href="/" class="brand-title text-2xl font-semibold tracking-wide text-slate-900 hover:text-slate-700">"gwt"</a>
                <nav class="flex flex-wrap gap-4 text-sm font-medium text-slate-700">
                    <a href="/worktrees" class="hover:text-slate-900">"Worktrees"</a>
                    <a href="/branches" class="hover:text-slate-900">"Branches"</a>
                    <a href="/terminal" class="hover:text-slate-900">"Terminal"</a>
                    <a href="/settings" class="hover:text-slate-900">"Settings"</a>
                </nav>
            </div>
        </header>
    }
}
