//! Worktree list page

use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::api;
use crate::components::Loading;

/// Worktree list component with API data fetching
#[component]
pub fn WorktreeList() -> impl IntoView {
    let refresh = RwSignal::new(0u64);
    let worktrees = LocalResource::new(move || {
        let _ = refresh.get();
        async move { api::fetch_worktrees().await.ok() }
    });
    let branch_name = RwSignal::new(String::new());
    let base_branch = RwSignal::new(String::new());
    let create_new = RwSignal::new(false);
    let is_working = RwSignal::new(false);
    let error = RwSignal::new(None::<String>);

    view! {
        <div class="space-y-6">
            <div class="panel rounded-2xl p-6 flex flex-col gap-4 md:flex-row md:items-center md:justify-between">
                <div>
                    <p class="text-xs uppercase tracking-[0.2em] text-slate-500">"Workspace"</p>
                    <h2 class="brand-title text-2xl font-semibold">"Worktrees"</h2>
                    <p class="text-sm text-slate-500">"Keep branches isolated while staying fast."</p>
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
                        <label class="text-xs text-slate-500 flex items-center gap-2">
                            <input
                                type="checkbox"
                                prop:checked=create_new
                                on:change=move |ev| create_new.set(event_target_checked(&ev))
                            />
                            "Create new branch"
                        </label>
                        <button
                            class="bg-[var(--accent)] text-white px-4 py-2 rounded-full shadow-sm hover:opacity-90 disabled:opacity-50"
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
                                let new_branch = create_new.get();
                                let refresh = refresh.clone();
                                let branch_name = branch_name.clone();
                                let base_branch = base_branch.clone();
                                let is_working = is_working.clone();
                                let error = error.clone();
                                spawn_local(async move {
                                    let req = api::CreateWorktreeRequest {
                                        branch: name,
                                        new_branch,
                                        base_branch: if base.is_empty() { None } else { Some(base) },
                                    };
                                    match api::create_worktree(req).await {
                                        Ok(_) => {
                                            branch_name.set(String::new());
                                            base_branch.set(String::new());
                                            refresh.update(|v| *v += 1);
                                        }
                                        Err(_) => {
                                            error.set(Some("Failed to create worktree.".to_string()));
                                        }
                                    }
                                    is_working.set(false);
                                });
                            }
                        >
                            {move || if is_working.get() { "Working..." } else { "Create Worktree" }}
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
                    worktrees.get().map(|data| {
                        match &*data {
                            Some(wts) if !wts.is_empty() => {
                                view! {
                                    <div class="grid gap-4 md:grid-cols-2">
                                        {wts.iter().map(|wt| {
                                            let delete_disabled = wt.is_main || wt.branch.is_none();
                                            let branch_name = wt.branch.clone().unwrap_or_default();
                                            view! {
                                                <div class="panel rounded-2xl p-4 hover:-translate-y-0.5 transition">
                                                    <div class="flex items-center justify-between">
                                                        <div class="font-medium text-slate-900">{wt.branch.clone().unwrap_or_else(|| "(detached)".to_string())}</div>
                                                        <span class=format!("px-2 py-1 rounded-full text-xs font-semibold {}", match wt.status.as_str() {
                                                            "active" => "bg-emerald-100 text-emerald-700",
                                                            "locked" => "bg-amber-100 text-amber-700",
                                                            _ => "bg-slate-100 text-slate-600",
                                                        })>
                                                            {wt.status.clone()}
                                                        </span>
                                                    </div>
                                                    <div class="text-sm text-slate-500 mt-2">{wt.path.clone()}</div>
                                                    <div class="mt-4 flex justify-end">
                                                        <button
                                                            class="text-xs px-3 py-1 rounded-full border border-black/10 text-slate-600 hover:text-slate-900 disabled:opacity-40"
                                                            disabled=delete_disabled
                                                            on:click=move |_| {
                                                                if delete_disabled {
                                                                    return;
                                                                }
                                                                let window = web_sys::window().expect("window");
                                                                if window.confirm_with_message("Delete this worktree?").unwrap_or(false) {
                                                                    let refresh = refresh.clone();
                                                                    let error = error.clone();
                                                                    let branch_name = branch_name.clone();
                                                                    spawn_local(async move {
                                                                        if api::delete_worktree(&branch_name).await.is_err() {
                                                                            error.set(Some("Failed to delete worktree.".to_string()));
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
                                <div class="panel rounded-2xl p-6 text-slate-500">"No worktrees found."</div>
                            }.into_any()
                        }
                    })
                }}
            </Suspense>
        </div>
    }
}
