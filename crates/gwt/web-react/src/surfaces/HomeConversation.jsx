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
  replyWaiting: "Project Manager is replying",
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

function PmFace({ avatar }) {
  return (
    <span className="gwt-home__face" aria-hidden="true">
      {avatar.imageUrl ? <img className="agent-avatar-image" src={avatar.imageUrl} alt="" /> : avatar.glyph}
    </span>
  );
}

function Lines({ lines }) {
  return lines.map((line, index) => <p key={index}>{line}</p>);
}

const clipboardImageTypes = new Set(["image/png", "image/jpeg", "image/webp"]);
const maxAttachments = 16;

function Attachments({ attachments, onRemove, disabled }) {
  if (!attachments?.length) return null;
  return <ul className="gwt-home__attachments" aria-label={onRemove ? "Draft attachments" : "Attachments"}>
    {attachments.map(({ name, preview, id }) => (
      <li key={id}>
        {preview ? <img src={preview} alt={name} /> : <span>{name}</span>}
        {onRemove ? <button type="button" className="wizard-button" disabled={disabled}
          aria-label={`Remove ${name}`} onClick={() => onRemove(id)}>×</button> : null}
      </li>
    ))}
  </ul>;
}

export function HomeConversation({ store, actions }) {
  const view = useStoreSelector(store, selectView);
  const [draft, setDraft] = useState("");
  const [attachments, setAttachments] = useState([]);
  const [attachmentError, setAttachmentError] = useState("");
  const [uploading, setUploading] = useState(false);
  const uploadsInFlight = useRef(false);
  const attachmentSendCount = useRef(null);
  const nextAttachmentId = useRef(0);
  const fileInputRef = useRef(null);
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
  const initialMessage = useRef(null);

  // The newest message is at the bottom: start there, and follow new messages.
  const timelineKey = view.topics.map(topic => topic.id).join("\n");
  useEffect(() => {
    const target = messageRefs.current.get(view.initialMessage);
    if (target && initialMessage.current !== view.initialMessage) {
      initialMessage.current = view.initialMessage;
      scrollIntoCenter(target);
      focusElement(target);
      setHighlight(view.initialMessage);
    } else if (!view.initialMessage) {
      scrollToEnd(logRef.current);
    }
  }, [timelineKey, view.initialMessage]);

  // A confirmed send clears exactly the draft that was sent.
  useEffect(() => {
    if (composer.sentCount) {
      setDraft("");
      if (composer.sentCount === attachmentSendCount.current) {
        setAttachments([]);
        attachmentSendCount.current = null;
      }
    }
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
  useEffect(() => {
    if (composer.replyTo?.id) focusElement(draftRef.current);
  }, [composer.replyTo?.id]);

  const reply = id => {
    actions.reply(id);
    if (!view.decisionsOnly) focusElement(draftRef.current);
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
  const canSend = Boolean(draft.trim()) && Boolean(target) && !view.busy && !noPm && !uploading;
  const canAttach = !composer.replyTo && !view.busy && !uploading;
  const addAttachments = files => {
    if (!canAttach || !files.length) return;
    if (attachments.length + files.length > maxAttachments) {
      setAttachmentError(`Choose up to ${maxAttachments} attachments.`);
      return;
    }
    setAttachmentError("");
    const added = files.map(file => ({
      file,
      name: file.name,
      id: ++nextAttachmentId.current,
      preview: clipboardImageTypes.has(file.type) ? URL.createObjectURL(file) : null,
    }));
    setAttachments(current => [...current, ...added]);
  };
  const removeAttachment = id => {
    const removed = attachments.find(attachment => attachment.id === id);
    if (removed?.preview) URL.revokeObjectURL(removed.preview);
    setAttachments(current => current.filter(attachment => attachment.id !== id));
    setAttachmentError("");
  };
  const submit = async event => {
    event?.preventDefault();
    if (!canSend || uploadsInFlight.current) return;
    attachmentSendCount.current = null;
    if (composer.replyTo || !attachments.length) {
      actions.submit(draft, target.key);
      return;
    }
    uploadsInFlight.current = true;
    setUploading(true);
    setAttachmentError("");
    try {
      const uploads = [];
      for (const { file } of attachments) {
        const uploaded = await actions.uploadAttachment(file);
        uploads.push({ ...uploaded, source: "uploaded" });
      }
      if (actions.submit(draft, target.key, attachments, uploads)) {
        attachmentSendCount.current = composer.sentCount + 1;
      } else {
        setAttachmentError("The message could not be sent. Your attachments are kept.");
      }
    } catch (error) {
      setAttachmentError(`Attachment upload failed: ${error.message}`);
    } finally {
      uploadsInFlight.current = false;
      setUploading(false);
    }
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
        <PmFace avatar={view.pm.avatar} />
        <div className="gwt-home__who">
          <h1 className="gwt-home__name">{view.decisionsOnly ? "Decisions" : view.pm.name}</h1>
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
        {view.decisionsOnly ? <a className="text-button" href="/">Back to Home</a> : null}
      </header>

      <div className="gwt-home__timeline" role="log" aria-label={COPY.conversation} ref={logRef} tabIndex={0}>
        {!view.decisionsOnly && view.resolved.length ? (
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
                    <p className="gwt-home__meta"><PmFace avatar={view.pm.avatar} /><span className="gwt-home__meta-name">{view.pm.name}</span><span>{entry.project}</span></p>
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

        {!view.decisionsOnly ? <article className="gwt-home__msg gwt-home__msg--pm" data-home-greeting>
          <p className="gwt-home__meta"><PmFace avatar={view.pm.avatar} /><span className="gwt-home__meta-name">{view.pm.name}</span></p>
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
        </article> : null}

        {view.notices.map(notice => <p className="gwt-home__note gwt-home__note--error" role="alert" key={notice}>{notice}</p>)}

        {view.topics.map(topic => topic.kind === "reply-waiting" ? (
          <article className="gwt-home__msg gwt-home__msg--pm" key={topic.id} role="status" aria-label={COPY.replyWaiting}>
            …
          </article>
        ) : topic.kind === "decision" ? (
          <article
            className={`gwt-home__msg gwt-home__msg--pm${highlight === topic.id ? " is-highlighted" : ""}`}
            key={topic.id}
            data-home-topic={topic.id}
            tabIndex={-1}
            ref={node => { if (node) messageRefs.current.set(topic.id, node); else messageRefs.current.delete(topic.id); }}
          >
            <p className="gwt-home__meta">
              <PmFace avatar={view.pm.avatar} />
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
            <Attachments attachments={topic.attachments} />
            {topic.status ? <p className="gwt-home__status" role="status">{topic.status}</p> : null}
            {topic.error ? <p className="gwt-home__status gwt-home__status--error" role="alert">{topic.error}</p> : null}
          </article>
        ) : (
          <article
            className={`gwt-home__msg gwt-home__msg--${topic.kind === "pm-message" ? "pm" : "me"}${highlight === topic.id ? " is-highlighted" : ""}`}
            key={topic.id}
            data-home-sent={topic.status ? topic.id : undefined}
            tabIndex={-1}
            ref={node => { if (node) messageRefs.current.set(topic.id, node); else messageRefs.current.delete(topic.id); }}
          >
            <p className="gwt-home__meta">
              <span className="gwt-home__meta-name">{topic.kind === "pm-message" ? view.pm.name : COPY.you}</span>
              <span>{topic.kind === "pm-message" ? topic.project : COPY.to(topic.project)}</span>
              <Time iso={topic.createdAt} />
            </p>
            <Lines lines={topic.lines} />
            <Attachments attachments={topic.attachments} />
            {topic.status ? <p className="gwt-home__status" role="status">{topic.status}</p> : null}
          </article>
        ))}
      </div>

      <form className="gwt-home__composer" hidden={view.decisionsOnly && !composer.replyTo} aria-label={COPY.draft} onSubmit={submit}>
        {composer.replyTo ? (
          <div className="gwt-home__replychip" data-home-replychip>
            <button type="button" className="gwt-home__replychip-target" onClick={() => jumpTo(composer.replyTo.id)}>
              <span>{COPY.replyingTo}</span>
              <b>{composer.replyTo.subject}</b>
            </button>
            <button type="button" className="gwt-home__replychip-clear" aria-label={COPY.clearReply} title={COPY.clearReply} onClick={actions.clearReply}>×</button>
          </div>
        ) : null}
        {!composer.replyTo ? <Attachments attachments={attachments} onRemove={removeAttachment} disabled={!canAttach} /> : null}
        <div className="gwt-home__compose-row">
          {!composer.replyTo ? <>
            <input type="file" ref={fileInputRef} multiple hidden onChange={event => {
              addAttachments(Array.from(event.target.files));
              event.target.value = "";
            }} />
            <button type="button" className="wizard-button" disabled={!canAttach}
              onClick={() => fileInputRef.current.click()}>Attach files</button>
          </> : null}
          <textarea
            className="gwt-home__draft"
            ref={draftRef}
            rows={2}
            aria-label={COPY.draft}
            placeholder={composer.replyTo ? COPY.draftReply : COPY.draftNew}
            value={draft}
            readOnly={uploading || (view.busy && Boolean(composer.status))}
            onDragOver={event => {
              if (canAttach && event.dataTransfer.types.includes("Files")) event.preventDefault();
            }}
            onDrop={event => {
              const files = Array.from(event.dataTransfer.files);
              if (!files.length) return;
              event.preventDefault();
              addAttachments(files);
            }}
            onPaste={event => {
              if (!canAttach) return;
              const images = Array.from(event.clipboardData.files).filter(file => clipboardImageTypes.has(file.type));
              if (!images.length) return;
              event.preventDefault();
              addAttachments(images);
            }}
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
          {(uploading ? "Uploading attachments…" : attachmentError) || composer.status || (composer.error ? `Not sent: ${composer.error} Your message is kept.` : needsProject ? COPY.needProject : noPm)}
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
