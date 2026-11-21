import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useStoreSelector } from "../store/useStoreSelector.js";
import { focusElement } from "../hosts/focus.js";
import { scrollIntoCenter, scrollToEnd } from "../hosts/scroll.js";
import { localTimeLabel, relativeTimeLabel } from "./time-labels.js";

// Home: the conversation with the Project Manager (Issue #7639, SPEC #7007
// T-021). The store carries the view the host derived; this surface keeps
// only what belongs to the screen itself: the draft, the chosen project, the
// search text, the open confirmation and the message a jump highlights.
//
// One timeline, oldest at the top and newest at the bottom (design items 229,
// 230, 250): settled topics first, then what is still open. A decision is a
// PM message with its choices inside it (item 226), and every reply goes
// through the one composer at the bottom (item 227).
const selectView = state => state;

// Every string this surface shows.
const COPY = {
  conversation: "Conversation with the Project Manager",
  you: "You",
  to: project => `to ${project}`,
  decisionWaiting: "Decision waiting",
  waitingFor: label => `waiting since ${label}`,
  reply: "Reply to this topic",
  later: "Decide later",
  recommended: "Recommended",
  resolvedSearch: "Search resolved topics",
  resolvedSearchPlaceholder: "Search resolved topics…",
  resolved: "Resolved",
  resolvedNone: "No resolved topic matches.",
  yourAnswer: "Your answer",
  remove: "Delete this topic",
  removeTitle: "Delete this topic?",
  removeBody: subject => `"${subject}" is removed from this conversation. The answer you sent stays with the project.`,
  removeConfirm: "Delete topic",
  cancel: "Cancel",
  replyingTo: "Replying to",
  clearReply: "Stop replying to this topic",
  draft: "Message to the Project Manager",
  draftNew: "Start a new topic… (name a project with @Project name)",
  draftReply: "Write your answer…",
  send: "Send",
  sendReply: "Reply",
  project: "Send to project",
  chooseProject: "Choose a project…",
  needProject: "Choose the project this is for, or name it with @Project name.",
  noPm: project => `No Project Manager is running for ${project}. Open the project to start one.`,
  guide: {
    github: { label: "Import from GitHub", hint: "Pick a repository and it becomes a project." },
    folder: { label: "Open a folder", hint: "Use a folder on this computer as a project." },
    clone: { label: "Clone from a URL", hint: "Paste a Git URL to clone and open it." },
  },
  projects: "Open Projects",
};

function Time({ iso }) {
  const label = relativeTimeLabel(iso);
  if (!label) return null;
  return <time dateTime={iso} title={localTimeLabel(iso) ?? undefined}>{label}</time>;
}

function PmFace() {
  return <span className="gwt-home__face" aria-hidden="true">P</span>;
}

function Lines({ lines }) {
  return lines.map((line, index) => <p key={index}>{line}</p>);
}

export function HomeConversation({ store, actions }) {
  const view = useStoreSelector(store, selectView);
  const [draft, setDraft] = useState("");
  const [chosenProject, setChosenProject] = useState("");
  const [query, setQuery] = useState("");
  const [removing, setRemoving] = useState(null);
  const [highlight, setHighlight] = useState(null);
  const logRef = useRef(null);
  const draftRef = useRef(null);
  const dialogRef = useRef(null);
  const removeTriggerRef = useRef(null);
  const messageRefs = useRef(new Map());
  const { composer } = view;

  // The newest message is at the bottom: start there, and follow new messages.
  const timelineKey = view.topics.map(topic => topic.id).join("\n");
  useEffect(() => {
    scrollToEnd(logRef.current);
  }, [timelineKey]);

  // A confirmed send clears exactly the draft that was sent.
  useEffect(() => {
    if (composer.sentCount) setDraft("");
  }, [composer.sentCount]);

  useEffect(() => {
    if (!removing) return undefined;
    focusElement(dialogRef.current?.querySelector("[data-home-remove-cancel]"));
    const onKeyDown = event => {
      if (event.key === "Escape") closeRemove();
    };
    const document = dialogRef.current.ownerDocument;
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [removing]);

  const jumpTo = id => {
    const node = messageRefs.current.get(id);
    if (!node) return;
    scrollIntoCenter(node);
    focusElement(node);
    setHighlight(id);
  };
  const reply = id => {
    actions.reply(id);
    focusElement(draftRef.current);
  };
  const closeRemove = () => {
    setRemoving(null);
    focusElement(removeTriggerRef.current);
  };

  // Where a new message goes: the project it names, else the one chosen, else
  // the only open project. Never a silent pick among several.
  const projects = composer.projects;
  const mentioned = mentionProject(draft, projects);
  const target = composer.replyTo
    ? projects.find(project => project.key === composer.replyTo.projectKey) ?? null
    : mentioned ?? projects.find(project => project.key === chosenProject) ?? (projects.length === 1 ? projects[0] : null);
  const needsProject = !composer.replyTo && !target && projects.length > 1 && draft.trim() !== "";
  const noPm = target && !target.pmAvailable ? COPY.noPm(target.title) : "";
  const canSend = Boolean(draft.trim()) && Boolean(target) && !view.busy && !noPm;
  const submit = event => {
    event?.preventDefault();
    if (canSend) actions.submit(draft, target.key);
  };
  const onDraftKeyDown = event => {
    if (event.key !== "Enter" || event.shiftKey || event.nativeEvent?.isComposing || event.keyCode === 229) return;
    submit(event);
  };

  const needle = query.trim().toLowerCase();
  const resolved = view.resolved.filter(entry => !needle
    || [entry.subject, entry.project, entry.answer].some(text => String(text ?? "").toLowerCase().includes(needle)));

  return (
    <section className="gwt-home" data-home aria-label={COPY.conversation}>
      <header className="gwt-home__head">
        <PmFace />
        <div className="gwt-home__who">
          <h1 className="gwt-home__name">{view.pm.name}</h1>
          <p className="gwt-home__sub">
            {view.pm.role}
            {" · "}
            <button
              type="button"
              className="gwt-home__waitjump"
              data-home-waitjump
              disabled={!view.waiting}
              onClick={() => jumpTo(view.firstWaitingId)}
            >
              {view.waitingLabel}
            </button>
          </p>
        </div>
      </header>

      <div className="gwt-home__timeline" role="log" aria-label={COPY.conversation} ref={logRef} tabIndex={0}>
        {view.resolved.length ? (
          <div className="gwt-home__resolved" data-home-resolved>
            <input
              type="search"
              className="gwt-home__search"
              aria-label={COPY.resolvedSearch}
              placeholder={COPY.resolvedSearchPlaceholder}
              value={query}
              onChange={event => setQuery(event.target.value)}
            />
            {resolved.length ? resolved.map(entry => (
              <details className="gwt-home__settled" key={entry.id} data-home-topic={entry.id}>
                <summary>
                  <span className="gwt-home__chip gwt-home__chip--done">{COPY.resolved}</span>
                  <span className="gwt-home__settled-subject">{entry.subject}</span>
                  <span className="gwt-home__settled-answer">{entry.answer}</span>
                </summary>
                <div className="gwt-home__settled-body">
                  <article className="gwt-home__msg gwt-home__msg--pm">
                    <p className="gwt-home__meta"><PmFace /><span className="gwt-home__meta-name">{view.pm.name}</span><span>{entry.project}</span></p>
                    <Lines lines={entry.lines} />
                  </article>
                  <article className="gwt-home__msg gwt-home__msg--me" aria-label={COPY.yourAnswer}>
                    <p className="gwt-home__meta"><span className="gwt-home__meta-name">{COPY.you}</span><Time iso={entry.answeredAt} /></p>
                    <p>{entry.answer}</p>
                  </article>
                  <button
                    type="button"
                    className="text-button"
                    data-home-remove={entry.id}
                    onClick={event => { removeTriggerRef.current = event.currentTarget; setRemoving(entry); }}
                  >
                    {COPY.remove}
                  </button>
                </div>
              </details>
            )) : <p className="gwt-home__note">{COPY.resolvedNone}</p>}
          </div>
        ) : null}

        <article className="gwt-home__msg gwt-home__msg--pm" data-home-greeting>
          <p className="gwt-home__meta"><PmFace /><span className="gwt-home__meta-name">{view.pm.name}</span></p>
          <p>{view.greeting}</p>
          {view.guide ? (
            <div className="gwt-home__guide">
              {[["github", actions.importFromGitHub], ["folder", actions.openFolder], ["clone", actions.clone]].map(([id, run]) => (
                <button type="button" className="gwt-home__entry" key={id} data-home-entry={id} onClick={run}>
                  <span>{COPY.guide[id].label}</span>
                  <small>{COPY.guide[id].hint}</small>
                </button>
              ))}
            </div>
          ) : null}
          {view.noneOpen ? (
            <div className="gwt-home__options">
              <button type="button" className="wizard-button" data-home-entry="projects" onClick={actions.openProjects}>{COPY.projects}</button>
            </div>
          ) : null}
        </article>

        {view.notices.map(notice => <p className="gwt-home__note gwt-home__note--error" role="alert" key={notice}>{notice}</p>)}

        {view.topics.map(topic => topic.kind === "decision" ? (
          <article
            className={`gwt-home__msg gwt-home__msg--pm${highlight === topic.id ? " is-highlighted" : ""}`}
            key={topic.id}
            data-home-topic={topic.id}
            tabIndex={-1}
            ref={node => { if (node) messageRefs.current.set(topic.id, node); else messageRefs.current.delete(topic.id); }}
          >
            <p className="gwt-home__meta">
              <PmFace />
              <span className="gwt-home__meta-name">{view.pm.name}</span>
              <span>{topic.project}</span>
              <span className="gwt-home__chip gwt-home__chip--waiting">{COPY.decisionWaiting}</span>
              <button type="button" className="gwt-home__reply" data-home-reply={topic.id} title={COPY.reply} aria-label={COPY.reply} onClick={() => reply(topic.id)}>↩</button>
              <span className="gwt-home__subject">
                {topic.subject}
                {relativeTimeLabel(topic.createdAt) ? <>{" · "}<span title={localTimeLabel(topic.createdAt) ?? undefined}>{COPY.waitingFor(relativeTimeLabel(topic.createdAt))}</span></> : null}
              </span>
            </p>
            <Lines lines={topic.lines} />
            <div className="gwt-home__options">
              {topic.options.map((option, index) => (
                <button
                  type="button"
                  className={`wizard-button${index === topic.recommended ? " primary" : ""}`}
                  key={index}
                  data-home-option={index}
                  title={index === topic.recommended ? COPY.recommended : undefined}
                  disabled={view.busy}
                  onClick={() => actions.answer(topic.id, index)}
                >
                  {option}
                </button>
              ))}
              <button type="button" className="text-button" data-home-defer disabled={view.busy} onClick={() => actions.defer(topic.id)}>{COPY.later}</button>
            </div>
            {topic.status ? <p className="gwt-home__status" role="status">{topic.status}</p> : null}
            {topic.error ? <p className="gwt-home__status gwt-home__status--error" role="alert">{topic.error}</p> : null}
          </article>
        ) : (
          <article className="gwt-home__msg gwt-home__msg--me" key={topic.id} data-home-sent={topic.id}>
            <p className="gwt-home__meta">
              <span className="gwt-home__meta-name">{COPY.you}</span>
              <span>{COPY.to(topic.project)}</span>
              <Time iso={topic.createdAt} />
            </p>
            <Lines lines={topic.lines} />
            <p className="gwt-home__status" role="status">{topic.status}</p>
          </article>
        ))}
      </div>

      <form className="gwt-home__composer" aria-label={COPY.draft} onSubmit={submit}>
        {composer.replyTo ? (
          <div className="gwt-home__replychip" data-home-replychip>
            <button type="button" className="gwt-home__replychip-target" onClick={() => jumpTo(composer.replyTo.id)}>
              <span>{COPY.replyingTo}</span>
              <b>{composer.replyTo.subject}</b>
            </button>
            <button type="button" className="gwt-home__replychip-clear" aria-label={COPY.clearReply} title={COPY.clearReply} onClick={actions.clearReply}>×</button>
          </div>
        ) : null}
        <div className="gwt-home__compose-row">
          <textarea
            className="gwt-home__draft"
            ref={draftRef}
            rows={2}
            aria-label={COPY.draft}
            placeholder={composer.replyTo ? COPY.draftReply : COPY.draftNew}
            value={draft}
            readOnly={view.busy && Boolean(composer.status)}
            onChange={event => setDraft(event.target.value)}
            onKeyDown={onDraftKeyDown}
          />
          {!composer.replyTo && projects.length > 1 ? (
            <select
              className="gwt-home__project"
              aria-label={COPY.project}
              data-home-project
              value={mentioned ? mentioned.key : chosenProject}
              disabled={Boolean(mentioned)}
              onChange={event => setChosenProject(event.target.value)}
            >
              <option value="">{COPY.chooseProject}</option>
              {projects.map(project => <option key={project.key} value={project.key}>{project.title}</option>)}
            </select>
          ) : null}
          <button type="submit" className="wizard-button primary" data-home-send disabled={!canSend}>
            {composer.replyTo ? COPY.sendReply : COPY.send}
          </button>
        </div>
        <p className="gwt-home__composer-status" aria-live="polite">
          {composer.status || (composer.error ? `Not sent: ${composer.error} Your message is kept.` : needsProject ? COPY.needProject : noPm)}
        </p>
      </form>

      {removing ? createPortal(
        <div className="modal-backdrop open" data-home-remove-dialog>
          <div className="modal-shell" role="dialog" aria-modal="true" aria-labelledby="gwt-home-remove-title" ref={dialogRef}>
            <div className="modal-header"><h2 id="gwt-home-remove-title">{COPY.removeTitle}</h2></div>
            <div className="modal-body"><p>{COPY.removeBody(removing.subject)}</p></div>
            <div className="modal-footer">
              <button type="button" className="wizard-button" data-home-remove-cancel onClick={closeRemove}>{COPY.cancel}</button>
              <button
                type="button"
                className="wizard-button primary"
                data-home-remove-confirm
                onClick={() => { actions.deleteTopic(removing.id); setRemoving(null); focusElement(logRef.current); }}
              >
                {COPY.removeConfirm}
              </button>
            </div>
          </div>
        </div>,
        logRef.current.ownerDocument.body,
      ) : null}
    </section>
  );
}

// `@Project name` in the draft names the project (design item 123). The
// longest title wins so "@Alpha Two" never resolves to "Alpha".
function mentionProject(text, projects) {
  const lower = String(text ?? "").toLowerCase();
  return [...projects].sort((a, b) => b.title.length - a.title.length).find(project => {
    const needle = `@${project.title.toLowerCase()}`;
    const at = lower.indexOf(needle);
    if (at < 0) return false;
    const next = lower[at + needle.length];
    return next === undefined || !/[\p{L}\p{N}_-]/u.test(next);
  }) ?? null;
}
