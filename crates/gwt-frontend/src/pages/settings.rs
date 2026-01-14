//! Settings page

use leptos::prelude::*;

use crate::api;
use crate::components::Loading;

/// Settings component with API data fetching
#[component]
pub fn Settings() -> impl IntoView {
    let settings = LocalResource::new(|| async move { api::fetch_settings().await.ok() });

    view! {
        <div class="space-y-6">
            <div>
                <p class="text-xs uppercase tracking-[0.2em] text-slate-500">"System"</p>
                <h2 class="brand-title text-2xl font-semibold">"Settings"</h2>
                <p class="text-sm text-slate-500">"Configuration snapshot from your local config."</p>
            </div>

            <Suspense fallback=move || view! { <Loading /> }>
                {move || {
                    settings.get().map(|data| {
                        match &*data {
                            Some(s) => {
                                view! {
                                    <div class="grid gap-4 md:grid-cols-2">
                                        <div class="panel rounded-2xl p-4">
                                            <h3 class="font-medium mb-2 text-slate-900">"Default Base Branch"</h3>
                                            <input
                                                type="text"
                                                value={s.default_base_branch.clone()}
                                                class="w-full rounded-lg border border-black/10 bg-white px-3 py-2 text-sm text-slate-700"
                                                disabled
                                            />
                                        </div>

                                        <div class="panel rounded-2xl p-4">
                                            <h3 class="font-medium mb-2 text-slate-900">"Worktree Root"</h3>
                                            <input
                                                type="text"
                                                value={s.worktree_root.clone()}
                                                class="w-full rounded-lg border border-black/10 bg-white px-3 py-2 text-sm text-slate-700"
                                                disabled
                                            />
                                        </div>

                                        <div class="panel rounded-2xl p-4 md:col-span-2">
                                            <h3 class="font-medium mb-2 text-slate-900">"Protected Branches"</h3>
                                            <div class="flex flex-wrap gap-2">
                                                {s.protected_branches.iter().map(|b: &String| {
                                                    let branch = b.clone();
                                                    view! {
                                                        <span class="bg-slate-100 text-slate-700 px-2 py-1 rounded-full text-xs font-medium">{branch}</span>
                                                    }
                                                }).collect::<Vec<_>>()}
                                            </div>
                                        </div>
                                    </div>
                                }.into_any()
                            }
                            None => view! {
                                <div class="panel rounded-2xl p-6 text-slate-500">"Unable to load settings."</div>
                            }.into_any()
                        }
                    })
                }}
            </Suspense>
        </div>
    }
}
