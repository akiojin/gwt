import { useEffect, useRef } from "react";
import { useStoreSelector } from "../store/useStoreSelector.js";
import { focusElement } from "../hosts/focus.js";

// First-run setup (Issue #7731, SPEC #7007 slice N-4). The host
// (crates/gwt/web/first-run-setup.js) derives everything shown here — the
// step, the gates and every string — and this surface only draws it.
const selectModel = state => state;

function Rows({ rows }) {
  return rows.map(row => (
    <div className="gwt-setup__row" key={row.title}>
      <div className="gwt-setup__row-text">
        <span className="gwt-setup__row-title">{row.title}</span>
        <span className="gwt-setup__row-detail">{row.detail}</span>
      </div>
      {row.badge ? <span className="gwt-setup__badge" data-state={row.state}>{row.badge}</span> : null}
    </div>
  ));
}

function Buttons({ back, onBack, children }) {
  return (
    <div className="gwt-setup__buttons">
      {onBack ? <button type="button" className="wizard-button" data-setup-back onClick={onBack}>{back}</button> : null}
      {children}
    </div>
  );
}

export function FirstRunSetup({ store, actions }) {
  const view = useStoreSelector(store, selectModel);
  const titleRef = useRef(null);
  const mounted = useRef(false);

  // Moving to another step moves the reader to its title.
  useEffect(() => {
    if (mounted.current) focusElement(titleRef.current);
    mounted.current = true;
  }, [view.step]);

  const title = text => <h2 className="gwt-setup__title" tabIndex={-1} ref={titleRef}>{text}</h2>;
  let body = null;
  if (!view.loaded) {
    body = null;
  } else if (view.step === "welcome") {
    body = (
      <>
        {title(view.welcome.title)}
        <div className="gwt-setup__group">
          <h3 className="gwt-setup__group-title">{view.welcome.group}</h3>
          <Rows rows={view.welcome.rows} />
        </div>
        <Buttons>
          <button type="button" className="wizard-button primary" data-setup-next onClick={() => actions.goTo("tools")}>
            {view.welcome.next}
          </button>
        </Buttons>
      </>
    );
  } else if (view.step === "tools") {
    const tools = view.tools;
    body = (
      <>
        {title(tools.title)}
        <p className="gwt-setup__lead">{tools.lead}</p>
        <div className="gwt-setup__group" data-setup-tools>
          {tools.items.map(item => (
            <div className="gwt-setup__row" key={item.name} data-setup-tool data-installed={String(item.installed)}>
              <div className="gwt-setup__row-text"><span className="gwt-setup__row-title">{item.name}</span></div>
              <span className="gwt-setup__badge" data-state={item.installed ? "done" : "blocked"}>{item.badge}</span>
            </div>
          ))}
          {tools.registered ? null : (
            <div className="gwt-setup__row" data-setup-tools-empty>
              <div className="gwt-setup__row-text">
                <span className="gwt-setup__row-title">{tools.noneTitle}</span>
                <span className="gwt-setup__row-detail">{tools.noneDetail}</span>
              </div>
              <button type="button" className="wizard-button" data-setup-retry onClick={actions.retry}>{tools.retry}</button>
            </div>
          )}
        </div>
        <Buttons back={view.back} onBack={() => actions.goTo("welcome")}>
          <button
            type="button"
            className="wizard-button primary"
            data-setup-next
            disabled={!tools.registered}
            aria-describedby={tools.reason ? "gwt-setup-reason" : undefined}
            onClick={() => actions.goTo("pm")}
          >
            {tools.next}
          </button>
        </Buttons>
        {tools.reason ? <p className="gwt-setup__reason" id="gwt-setup-reason" data-setup-reason>{tools.reason}</p> : null}
      </>
    );
  } else if (view.step === "pm") {
    const pm = view.pm;
    body = (
      <form onSubmit={event => { event.preventDefault(); actions.save(); }}>
        {title(pm.title)}
        <p className="gwt-setup__lead">{pm.lead}</p>
        <div className="gwt-setup__group">
          <div className="gwt-setup__row gwt-setup__row--choice" data-setup-pm>
            <div className="gwt-setup__row-text">
              <span className="gwt-setup__row-title" id="gwt-setup-pm-label">{pm.row}</span>
              <span className="gwt-setup__row-detail">{pm.rowDetail}</span>
            </div>
            <div className="gwt-setup__choices" role="radiogroup" aria-labelledby="gwt-setup-pm-label">
              {pm.options.map(option => (
                <label className="gwt-setup__choice" key={option.value} data-checked={String(pm.choice === option.value)}>
                  <input
                    type="radio"
                    name="gwt-setup-pm"
                    value={option.value}
                    checked={pm.choice === option.value}
                    onChange={() => actions.choose(option.value)}
                  />
                  <span>{option.label}</span>
                </label>
              ))}
              {pm.naming ? (
                // Uncontrolled, so an IME composition is never interrupted.
                <input
                  type="text"
                  className="gwt-setup__name"
                  data-setup-pm-name
                  aria-label={pm.name}
                  placeholder={pm.namePlaceholder}
                  maxLength={pm.nameMax}
                  autoComplete="off"
                  defaultValue={pm.nameValue}
                  onInput={event => actions.rename(event.currentTarget.value)}
                />
              ) : null}
            </div>
          </div>
        </div>
        {pm.note ? <p className="gwt-setup__note">{pm.note}</p> : null}
        <Buttons back={view.back} onBack={() => actions.goTo("tools")}>
          <button
            type="submit"
            className="wizard-button primary"
            data-setup-next
            disabled={!pm.canSave}
            aria-describedby={pm.reason ? "gwt-setup-reason" : undefined}
          >
            {pm.next}
          </button>
        </Buttons>
        {pm.reason ? <p className="gwt-setup__reason" id="gwt-setup-reason" data-setup-reason>{pm.reason}</p> : null}
      </form>
    );
  } else if (view.step === "project") {
    const project = view.project;
    body = (
      <>
        {title(project.title)}
        <p className="gwt-setup__lead">{project.lead}</p>
        <div className="gwt-setup__entries">
          <button type="button" className="gwt-setup__entry" data-setup-open-folder onClick={actions.openFolder}>
            {project.openFolder.label}<small>{project.openFolder.detail}</small>
          </button>
          <button type="button" className="gwt-setup__entry" data-setup-clone onClick={actions.clone}>
            {project.clone.label}<small>{project.clone.detail}</small>
          </button>
        </div>
        <Buttons back={view.back} onBack={() => actions.goTo("pm")}>
          <button type="button" className="wizard-button primary" data-setup-next onClick={() => actions.goTo("done")}>
            {project.next}
          </button>
        </Buttons>
        <p className="gwt-setup__note">{project.note}</p>
      </>
    );
  } else {
    body = (
      <>
        {title(view.done.title)}
        <p className="gwt-setup__lead">{view.done.lead}</p>
        <div className="gwt-setup__group"><Rows rows={view.done.rows} /></div>
        <Buttons>
          <button type="button" className="wizard-button primary" data-setup-home onClick={actions.home}>{view.done.home}</button>
        </Buttons>
      </>
    );
  }

  return (
    <section className="gwt-setup" data-setup data-setup-step={view.step} aria-labelledby="gwt-setup-heading">
      <header className="gwt-setup__head">
        <h1 className="gwt-setup__heading" id="gwt-setup-heading">{view.heading}</h1>
        <p className="gwt-setup__lead">{view.lead}</p>
      </header>
      <ol className="gwt-setup__steps">
        {view.steps.map(step => (
          <li key={step.id} data-state={step.state} aria-current={step.state === "current" ? "step" : undefined}>{step.label}</li>
        ))}
      </ol>
      <p className="gwt-setup__status" role="status">{view.status}</p>
      {view.error ? <p className="gwt-setup__error" role="alert" data-setup-error>{view.error}</p> : null}
      {body}
    </section>
  );
}
