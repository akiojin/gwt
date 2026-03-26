1. `cd gwt-gui && pnpm install`
2. `cd gwt-gui && pnpm test src/lib/components/AgentLaunchForm.test.ts src/lib/agentLaunchDefaults.test.ts`
3. `cargo test -p gwt-core agent::codex::tests:: -- --test-threads=1`
4. `cargo test -p gwt-tauri build_agent_args_codex -- --test-threads=1`
5. `cd gwt-gui && pnpm exec svelte-check --tsconfig ./tsconfig.json`
