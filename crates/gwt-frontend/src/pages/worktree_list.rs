//! Worktree list page

use leptos::prelude::*;

use crate::api;
use crate::components::Loading;

/// Worktree list component with API data fetching
#[component]
pub fn WorktreeList() -> impl IntoView {
    let worktrees = LocalResource::new(|| async move { api::fetch_worktrees().await.ok() });

    view! {
        <div class="space-y-4">
            <div class="flex justify-between items-center">
                <h2 class="text-xl font-semibold">"Worktrees"</h2>
                <button class="bg-blue-500 text-white px-4 py-2 rounded hover:bg-blue-600">
                    "Create Worktree"
                </button>
            </div>

            <Suspense fallback=move || view! { <Loading /> }>
                {move || {
                    worktrees.get().map(|data| {
                        match &*data {
                            Some(wts) if !wts.is_empty() => {
                                view! {
                                    <div class="space-y-2">
                                        {wts.iter().map(|wt| {
                                            view! {
                                                <div class="border rounded p-4 hover:bg-gray-50">
                                                    <div class="font-medium">{wt.branch.clone().unwrap_or_else(|| "(detached)".to_string())}</div>
                                                    <div class="text-sm text-gray-500">{wt.path.clone()}</div>
                                                    <div class="text-xs text-gray-400 mt-1">
                                                        <span class=format!("px-2 py-1 rounded {}", match wt.status.as_str() {
                                                            "active" => "bg-green-100 text-green-800",
                                                            "locked" => "bg-yellow-100 text-yellow-800",
                                                            _ => "bg-gray-100 text-gray-800",
                                                        })>
                                                            {wt.status.clone()}
                                                        </span>
                                                    </div>
                                                </div>
                                            }
                                        }).collect::<Vec<_>>()}
                                    </div>
                                }.into_any()
                            }
                            _ => view! {
                                <p class="text-gray-500">"No worktrees found."</p>
                            }.into_any()
                        }
                    })
                }}
            </Suspense>
        </div>
    }
}
