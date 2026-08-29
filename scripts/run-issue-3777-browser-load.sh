#!/bin/sh
set -eu

capability_file="${GWT_PLAYWRIGHT_CHECK_HOME:?}/.gwt/issue-3777-hook-capability"
hook_stdout="${GWT_PLAYWRIGHT_CHECK_HOME:?}/.gwt/issue-3777-hook.stdout"
hook_stderr="${GWT_PLAYWRIGHT_CHECK_HOME:?}/.gwt/issue-3777-hook.stderr"
hook_dispatcher_rendezvous="${GWT_PLAYWRIGHT_CHECK_HOME:?}/.gwt/issue-3777-hook-dispatcher-entered"
hook_completion_rendezvous="${GWT_PLAYWRIGHT_CHECK_HOME:?}/.gwt/issue-3777-hook-completed"
work_items_fixture="${GWT_PLAYWRIGHT_WORK_ITEMS_FIXTURE_PATH:?}"
work_items_target="${GWT_PLAYWRIGHT_WORK_ITEMS_TARGET_PATH:?}"
work_items_staging="${work_items_target}.issue-3777-staging-$$"
gwtd_path="${GWT_PLAYWRIGHT_GWTD_PATH:?}"
hook_job_pid=""
cleanup() {
  rm -f "$work_items_staging"
  if [ -n "$hook_job_pid" ]; then
    kill "$hook_job_pid" 2>/dev/null || true
    wait "$hook_job_pid" 2>/dev/null || true
  fi
}
trap cleanup EXIT HUP INT TERM

{
  IFS= read -r forward_url
  IFS= read -r forward_token
  IFS= read -r session_id
  IFS= read -r runtime_path
} < "$capability_file"
: "${forward_url:?}"
: "${forward_token:?}"
: "${session_id:?}"
: "${runtime_path:?}"

cp "$work_items_fixture" "$work_items_staging"
mv "$work_items_staging" "$work_items_target"
printf '%s\n' 'GWT_RESPONSIVENESS_LOAD_STARTED'
IFS= read -r marker
if [ "$marker" != 'GWT_RESPONSIVENESS_START_HOOK' ]; then
  exit 64
fi
rm -f "$hook_dispatcher_rendezvous" "$hook_completion_rendezvous"
(
  set +e
  env \
    HOME="$GWT_PLAYWRIGHT_CHECK_HOME" \
    USERPROFILE="$GWT_PLAYWRIGHT_CHECK_HOME" \
    GWT_HOOK_FORWARD_URL="$forward_url" \
    GWT_HOOK_FORWARD_TOKEN="$forward_token" \
    GWT_SESSION_ID="$session_id" \
    GWT_SESSION_RUNTIME_PATH="$runtime_path" \
    GWT_HOOK_PROFILE_PATH="$GWT_HOOK_PROFILE_PATH" \
    GWT_PLAYWRIGHT_HOOK_DISPATCHER_RENDEZVOUS="$hook_dispatcher_rendezvous" \
    "$gwtd_path" hook event UserPromptSubmit \
      > "$hook_stdout" 2> "$hook_stderr" <<JSON
{"prompt":"continue","cwd":"$GWT_PLAYWRIGHT_PROJECT_ROOT"}
JSON
  hook_status=$?
  printf '%s\n' "$hook_status" > "$hook_completion_rendezvous"
  printf '%s:%s\n' 'GWT_RESPONSIVENESS_HOOK_COMPLETED' "$hook_status"
  exit "$hook_status"
) &
hook_job_pid=$!
while [ ! -f "$hook_dispatcher_rendezvous" ]; do
  if ! kill -0 "$hook_job_pid" 2>/dev/null; then
    wait "$hook_job_pid"
    exit 70
  fi
  sleep 0.005
done
printf '%s\n' 'GWT_RESPONSIVENESS_HOOK_STARTED'
IFS= read -r marker
if [ "$marker" != 'GWT_RESPONSIVENESS_INTERACTIONS_COMPLETE' ]; then
  exit 64
fi
wait "$hook_job_pid"
hook_job_pid=""
