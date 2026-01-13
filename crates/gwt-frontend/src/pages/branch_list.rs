//! Branch list page

use leptos::prelude::*;

use crate::api;
use crate::components::Loading;

/// Branch list component with API data fetching
#[component]
pub fn BranchList() -> impl IntoView {
    let branches = LocalResource::new(|| async move { api::fetch_branches().await.ok() });

    view! {
        <div class="space-y-4">
            <div class="flex justify-between items-center">
                <h2 class="text-xl font-semibold">"Branches"</h2>
                <button class="bg-blue-500 text-white px-4 py-2 rounded hover:bg-blue-600">
                    "Create Branch"
                </button>
            </div>

            <Suspense fallback=move || view! { <Loading /> }>
                {move || {
                    branches.get().map(|data| {
                        match &*data {
                            Some(brs) if !brs.is_empty() => {
                                view! {
                                    <div class="space-y-2">
                                        {brs.iter().map(|b| {
                                            view! {
                                                <div class="border rounded p-4 hover:bg-gray-50 flex justify-between items-center">
                                                    <div>
                                                        <div class="font-medium flex items-center gap-2">
                                                            {b.name.clone()}
                                                            {b.is_current.then(|| view! {
                                                                <span class="text-xs bg-green-100 text-green-800 px-2 py-1 rounded">"current"</span>
                                                            })}
                                                        </div>
                                                        <div class="text-xs text-gray-400 mt-1">
                                                            {if b.has_remote { "Has remote" } else { "Local only" }}
                                                            {(b.ahead > 0 || b.behind > 0).then(|| format!(" | +{} -{}", b.ahead, b.behind))}
                                                        </div>
                                                    </div>
                                                    <div class="flex gap-2">
                                                        {(!b.has_worktree).then(|| view! {
                                                            <button class="text-sm bg-gray-100 px-3 py-1 rounded hover:bg-gray-200">
                                                                "Create Worktree"
                                                            </button>
                                                        })}
                                                    </div>
                                                </div>
                                            }
                                        }).collect::<Vec<_>>()}
                                    </div>
                                }.into_any()
                            }
                            _ => view! {
                                <p class="text-gray-500">"No branches found."</p>
                            }.into_any()
                        }
                    })
                }}
            </Suspense>
        </div>
    }
}
