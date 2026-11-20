import { useEffect, useRef } from "react";
import { useStoreSelector } from "../store/useStoreSelector.js";
import { focusElement, focusFirstEnabledButton } from "../hosts/focus.js";

// Task-centered Decide surface (SPEC #7007 T-010). The store carries only
// state derived by the host from the PM reports snapshot: typed decision
// packets become numbered options, everything else keeps the existing chat
// reply path. The surface never invents operations the backend did not
// deliver — an answer is a chat reply, and a decision leaves this list only
// when a later snapshot no longer hands it to the surface. "Decide later"
// postpones without settling: the host moves the decision to the end of the
// queue, it keeps waiting, and the waiting count does not change (T-023).
const EMPTY_MODEL = Object.freeze({
  loaded: false,
  error: null,
  decisions: [],
  sending: false,
  pendingId: null,
  sendError: null,
  answered: {},
  deferNotice: null,
  deferCount: 0,
});

function selectTaskDecisionsModel(state) {
  return state ?? EMPTY_MODEL;
}

function relativeTimeLabel(iso) {
  if (!iso) return null;
  const then = Date.parse(iso);
  if (Number.isNaN(then)) return null;
  const diffMs = Date.now() - then;
  if (diffMs < 0) return iso.slice(0, 10);
  const minutes = Math.floor(diffMs / 60_000);
  if (minutes < 1) return "just now";
  if (minutes < 60) return `${minutes} minute${minutes === 1 ? "" : "s"} ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours} hour${hours === 1 ? "" : "s"} ago`;
  const days = Math.floor(hours / 24);
  if (days < 7) return `${days} day${days === 1 ? "" : "s"} ago`;
  return iso.slice(0, 10);
}

export function TaskDecisions({ store, onAnswerDecision, onReplyInChat, onDeferDecision = () => {} }) {
  const model = useStoreSelector(store, selectTaskDecisionsModel);
  const headingRef = useRef(null);
  const cardRefs = useRef(new Map());
  const actedRef = useRef(null);
  const deferredRef = useRef(null);
  const decisions = model.decisions ?? [];
  const answered = model.answered ?? {};
  const actionable = decisions.filter(item => !answered[item.id]);
  const actionableKey = actionable.map(item => item.id).join("\n");

  // Focus placement itself is imperative DOM and belongs to the host
  // (src/hosts/focus.js); this surface only chooses the target: the next
  // waiting decision's first enabled button, or the heading when no
  // waiting decision is left.
  const focusAfter = id => {
    const index = decisions.findIndex(item => item.id === id);
    const next = decisions.slice(index + 1).find(item => !answered[item.id]);
    if (next) focusFirstEnabledButton(cardRefs.current.get(next.id));
    else focusElement(headingRef.current);
  };

  // When the decision the user just acted on is answered (or leaves the
  // snapshot), focus moves to the next waiting decision or the heading so a
  // keyboard user is never left on a removed or inert row.
  useEffect(() => {
    const actedId = actedRef.current;
    if (!actedId) return;
    if (actionable.some(item => item.id === actedId)) return;
    actedRef.current = null;
    focusAfter(actedId);
  }, [actionableKey]);

  const answer = (item, optionIndex) => {
    if (model.sending || answered[item.id]) return;
    actedRef.current = item.id;
    onAnswerDecision(item, optionIndex);
  };
  const later = item => {
    deferredRef.current = item.id;
    onDeferDecision(item);
  };

  // After a deferral the queue is reordered: the next waiting decision is
  // now first, so focus (and with it the scroll position) moves there. When
  // the deferred decision is the only one waiting, focus stays on it.
  useEffect(() => {
    const deferredId = deferredRef.current;
    if (!deferredId) return;
    deferredRef.current = null;
    const next = actionable.find(item => item.id !== deferredId) ?? actionable[0];
    if (next) focusFirstEnabledButton(cardRefs.current.get(next.id));
    else focusElement(headingRef.current);
  }, [model.deferCount]);

  return (
    <section className="task-decisions" aria-label="Decide">
      <header className="task-decisions__header">
        <div>
          <p className="task-decisions__eyebrow">Your PM</p>
          <h1 ref={headingRef} tabIndex={-1}>Decide</h1>
        </div>
        {model.loaded && decisions.length > 0 ? (
          <p className="task-decisions__count" role="status">
            {actionable.length === 0
              ? "No decisions waiting"
              : `${actionable.length} decision${actionable.length === 1 ? "" : "s"} waiting`}
          </p>
        ) : null}
      </header>
      {model.deferNotice ? (
        <p className="task-decisions__notice" role="status">{model.deferNotice}</p>
      ) : null}
      {!model.loaded ? (
        <p role="status">Loading decisions…</p>
      ) : model.error && decisions.length === 0 ? (
        <div className="task-decisions__empty">
          <h2>Unable to refresh decisions</h2>
          <p>{model.error}</p>
        </div>
      ) : decisions.length === 0 ? (
        <div className="task-decisions__empty">
          <h2>No decisions waiting</h2>
          <p>When the PM needs your judgment, the decision will appear here with its reason and impact.</p>
        </div>
      ) : (
        <>
          {model.error ? (
            <p className="task-decisions__error" role="alert">
              {model.error} Showing the last known decisions.
            </p>
          ) : null}
          {decisions.map(item => {
            const answeredText = answered[item.id];
            const isPending = model.sending && model.pendingId === item.id;
            const itemError = model.sendError?.id === item.id ? model.sendError.message : null;
            const meta = [item.owner, relativeTimeLabel(item.createdAt)].filter(Boolean).join(" · ");
            return (
              <section
                className="task-decisions__card"
                key={item.id}
                data-decision-id={item.id}
                ref={element => {
                  if (element) cardRefs.current.set(item.id, element);
                  else cardRefs.current.delete(item.id);
                }}
              >
                <div className="task-decisions__card-head">
                  <h2>{item.question}</h2>
                  <span className="task-decisions__chip">
                    {answeredText
                      ? "Answer sent"
                      : `${item.deferred ? "Deferred · " : ""}${item.options ? "Choice needed" : "Reply needed"}`}
                  </span>
                </div>
                {item.result && item.result !== item.question ? (
                  <p className="task-decisions__result">{item.result}</p>
                ) : null}
                {meta ? <p className="task-decisions__meta">{meta}</p> : null}
                {item.impact ? <p className="task-decisions__impact">{`Impact: ${item.impact}`}</p> : null}
                {item.recommendation ? (
                  <p className="task-decisions__recommendation">{`Recommended: ${item.recommendation}`}</p>
                ) : null}
                {item.reason ? <p className="task-decisions__reason">{item.reason}</p> : null}
                {item.detailText ? <p className="task-decisions__detail">{item.detailText}</p> : null}
                {itemError ? (
                  <p className="task-decisions__send-error" role="alert">{itemError}</p>
                ) : null}
                {answeredText ? (
                  <p className="task-decisions__answered" role="status">
                    {`Answer sent: ${answeredText}. The PM will continue from here.`}
                  </p>
                ) : (
                  <div className="task-decisions__actions">
                    {item.options
                      ? item.options.map((option, index) => (
                          <button
                            key={index}
                            type="button"
                            className={`task-decisions__option${index === 0 ? " task-decisions__option--recommended" : ""}`}
                            disabled={model.sending}
                            onClick={() => answer(item, index)}
                          >
                            {`${index + 1}. ${option}`}
                          </button>
                        ))
                      : null}
                    <button
                      type="button"
                      className="task-decisions__reply"
                      onClick={() => onReplyInChat(item)}
                    >
                      Reply in chat
                    </button>
                    <button
                      type="button"
                      className="task-decisions__later"
                      onClick={() => later(item)}
                    >
                      Decide later
                    </button>
                    {isPending ? <span role="status">Sending…</span> : null}
                  </div>
                )}
              </section>
            );
          })}
        </>
      )}
    </section>
  );
}
