//! Settings page

use leptos::prelude::*;

use crate::api;
use crate::components::Loading;

/// Settings component with API data fetching
#[component]
pub fn Settings() -> impl IntoView {
    let settings = LocalResource::new(|| async move { api::fetch_settings().await.ok() });

    view! {
        <div class="space-y-4">
            <h2 class="text-xl font-semibold">"Settings"</h2>

            <Suspense fallback=move || view! { <Loading /> }>
                {move || {
                    settings.get().map(|data| {
                        match &*data {
                            Some(s) => {
                                view! {
                                    <div class="space-y-6">
                                        <div class="border rounded p-4">
                                            <h3 class="font-medium mb-2">"Default Base Branch"</h3>
                                            <input
                                                type="text"
                                                value={s.default_base_branch.clone()}
                                                class="w-full border rounded p-2"
                                                disabled
                                            />
                                        </div>

                                        <div class="border rounded p-4">
                                            <h3 class="font-medium mb-2">"Worktree Root"</h3>
                                            <input
                                                type="text"
                                                value={s.worktree_root.clone()}
                                                class="w-full border rounded p-2"
                                                disabled
                                            />
                                        </div>

                                        <div class="border rounded p-4">
                                            <h3 class="font-medium mb-2">"Protected Branches"</h3>
                                            <div class="flex flex-wrap gap-2">
                                                {s.protected_branches.iter().map(|b: &String| {
                                                    let branch = b.clone();
                                                    view! {
                                                        <span class="bg-gray-100 px-2 py-1 rounded text-sm">{branch}</span>
                                                    }
                                                }).collect::<Vec<_>>()}
                                            </div>
                                        </div>
                                    </div>
                                }.into_any()
                            }
                            None => view! {
                                <p class="text-gray-500">"Unable to load settings."</p>
                            }.into_any()
                        }
                    })
                }}
            </Suspense>
        </div>
    }
}
