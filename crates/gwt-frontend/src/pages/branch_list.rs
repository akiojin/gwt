//! Branch list page

use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::api;
use crate::components::Loading;

/// Branch list component with API data fetching
#[component]
pub fn BranchList() -> impl IntoView {
    let refresh = RwSignal::new(0u64);
    let branches = LocalResource::new(move || {
        let _ = refresh.get();
        async move { api::fetch_branches().await.ok() }
    });
    let branch_name = RwSignal::new(String::new());
    let base_branch = RwSignal::new(String::new());
    let is_working = RwSignal::new(false);
    let error = RwSignal::new(None::<String>);

    view! {
        <div class="space-y-6">
            <div class="panel rounded-2xl p-6 flex flex-col gap-4 md:flex-row md:items-center md:justify-between">
                <div>
                    <p class="text-xs uppercase tracking-[0.2em] text-slate-500">"Repository"</p>
                    <h2 class="brand-title text-2xl font-semibold">"Branches"</h2>
                    <p class="text-sm text-slate-500">"Scan ahead, jump fast, keep branches tidy."</p>
                </div>
                <div class="flex flex-col gap-2 md:items-end">
                    <div class="flex flex-col gap-2 md:flex-row md:items-center">
                        <input
                            type="text"
                            placeholder="Branch name"
                            class="w-full md:w-48 rounded-full border border-black/10 bg-white px-4 py-2 text-sm"
                            prop:value=branch_name
                            on:input=move |ev| branch_name.set(event_target_value(&ev))
                        />
                        <input
                            type="text"
                            placeholder="Base branch (optional)"
                            class="w-full md:w-48 rounded-full border border-black/10 bg-white px-4 py-2 text-sm"
                            prop:value=base_branch
                            on:input=move |ev| base_branch.set(event_target_value(&ev))
                        />
                        <button
                            class="bg-[var(--accent-2)] text-white px-4 py-2 rounded-full shadow-sm hover:opacity-90 disabled:opacity-50"
                            disabled=move || is_working.get()
                            on:click=move |_| {
                                let name = branch_name.get().trim().to_string();
                                if name.is_empty() {
                                    error.set(Some("Branch name is required.".to_string()));
                                    return;
                                }
                                error.set(None);
                                is_working.set(true);
                                let base = base_branch.get().trim().to_string();
                                let refresh = refresh.clone();
                                let branch_name = branch_name.clone();
                                let base_branch = base_branch.clone();
                                let is_working = is_working.clone();
                                let error = error.clone();
                                spawn_local(async move {
                                    let req = api::CreateBranchRequest {
                                        name,
                                        base: if base.is_empty() { None } else { Some(base) },
                                    };
                                    match api::create_branch(req).await {
                                        Ok(_) => {
                                            branch_name.set(String::new());
                                            base_branch.set(String::new());
                                            refresh.update(|v| *v += 1);
                                        }
                                        Err(_) => {
                                            error.set(Some("Failed to create branch.".to_string()));
                                        }
                                    }
                                    is_working.set(false);
                                });
                            }
                        >
                            {move || if is_working.get() { "Working..." } else { "Create Branch" }}
                        </button>
                    </div>
                    {move || {
                        error.get().map(|message| view! {
                            <p class="text-xs text-red-600">{message}</p>
                        })
                    }}
                </div>
            </div>

            <Suspense fallback=move || view! { <Loading /> }>
                {move || {
                    branches.get().map(|data| {
                        match &*data {
                            Some(brs) if !brs.is_empty() => {
                                view! {
                                    <div class="grid gap-4">
                                        {brs.iter().map(|b| {
                                            let name = b.name.clone();
                                            let name_for_worktree = name.clone();
                                            let name_for_delete = name.clone();
                                            let can_delete = !b.is_current;
                                            let can_create_worktree = !b.has_worktree;
                                            view! {
                                                <div class="panel rounded-2xl p-4 flex flex-col gap-3 md:flex-row md:items-center md:justify-between">
                                                    <div>
                                                        <div class="font-medium flex items-center gap-2 text-slate-900">
                                                            {b.name.clone()}
                                                            {b.is_current.then(|| view! {
                                                                <span class="text-xs bg-emerald-100 text-emerald-700 px-2 py-1 rounded-full">"current"</span>
                                                            })}
                                                        </div>
                                                        <div class="text-xs text-slate-500 mt-1">
                                                            {if b.has_remote { "Has remote" } else { "Local only" }}
                                                            {(b.ahead > 0 || b.behind > 0).then(|| format!(" | +{} -{}", b.ahead, b.behind))}
                                                        </div>
                                                    </div>
                                                    <div class="flex gap-2">
                                                        {can_create_worktree.then(|| view! {
                                                            <button
                                                                class="text-sm bg-slate-900 text-white px-3 py-1 rounded-full hover:opacity-90"
                                                                on:click=move |_| {
                                                                    let refresh = refresh.clone();
                                                                    let error = error.clone();
                                                                    let name = name_for_worktree.clone();
                                                                    spawn_local(async move {
                                                                        let req = api::CreateWorktreeRequest {
                                                                            branch: name,
                                                                            new_branch: false,
                                                                            base_branch: None,
                                                                        };
                                                                        if api::create_worktree(req).await.is_err() {
                                                                            error.set(Some("Failed to create worktree.".to_string()));
                                                                        } else {
                                                                            refresh.update(|v| *v += 1);
                                                                        }
                                                                    });
                                                                }
                                                            >
                                                                "Create Worktree"
                                                            </button>
                                                        })}
                                                        <button
                                                            class="text-sm border border-black/10 text-slate-600 px-3 py-1 rounded-full hover:text-slate-900 disabled:opacity-40"
                                                            disabled=!can_delete
                                                            on:click=move |_| {
                                                                if !can_delete {
                                                                    return;
                                                                }
                                                                let window = web_sys::window().expect("window");
                                                                if window.confirm_with_message("Delete this branch?").unwrap_or(false) {
                                                                    let refresh = refresh.clone();
                                                                    let error = error.clone();
                                                                    let name = name_for_delete.clone();
                                                                    spawn_local(async move {
                                                                        if api::delete_branch(&name).await.is_err() {
                                                                            error.set(Some("Failed to delete branch.".to_string()));
                                                                        } else {
                                                                            refresh.update(|v| *v += 1);
                                                                        }
                                                                    });
                                                                }
                                                            }
                                                        >
                                                            "Delete"
                                                        </button>
                                                    </div>
                                                </div>
                                            }
                                        }).collect::<Vec<_>>()}
                                    </div>
                                }.into_any()
                            }
                            _ => view! {
                                <div class="panel rounded-2xl p-6 text-slate-500">"No branches found."</div>
                            }.into_any()
                        }
                    })
                }}
            </Suspense>
        </div>
    }
}
