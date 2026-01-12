//! Worktree list page

use leptos::prelude::*;

#[component]
pub fn WorktreeList() -> impl IntoView {
    view! {
        <div class="space-y-4">
            <h2 class="text-xl font-semibold">"Worktrees"</h2>
            <p class="text-gray-500">"No worktrees found."</p>
        </div>
    }
}
