//! 404 Not Found page

use leptos::prelude::*;

/// Not Found page component
#[component]
pub fn NotFound() -> impl IntoView {
    view! {
        <div class="flex flex-col items-center justify-center min-h-[50vh]">
            <h1 class="text-4xl font-bold text-gray-400">"404"</h1>
            <p class="text-gray-500 mt-2">"Page not found"</p>
            <a href="/" class="mt-4 text-blue-500 hover:underline">"Go to Home"</a>
        </div>
    }
}
