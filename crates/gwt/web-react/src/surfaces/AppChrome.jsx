import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useStoreSelector } from "../store/useStoreSelector.js";
import { focusElement, focusFirstEnabledButton } from "../hosts/focus.js";

// The header and footer every page shares (Issue #7740, SPEC #7007 F-1).
// The store carries the entries the host derived from its one destination
// definition; nothing about a destination is decided here. An entry with a
// `reason` cannot be chosen: it stays in place and shows that reason.
const selectModel = state => state;
const selectFooterVersion = state => state.footer.version;
const selectFooterPm = state => state.footer.pm;

function Entry({ entry, menuOpen, onActivate }) {
  const expanded = entry.kind === "menu" ? menuOpen : entry.expanded;
  return (
    <button
      type="button"
      className="app-head__item"
      id={entry.elementId}
      data-destination={entry.id}
      data-unread={entry.unread}
      data-has-error={entry.hasError === undefined ? undefined : String(entry.hasError)}
      title={entry.title}
      aria-label={entry.ariaLabel}
      aria-controls={entry.controls}
      aria-disabled={entry.reason ? "true" : undefined}
      aria-current={entry.current ? "page" : undefined}
      aria-haspopup={entry.kind === "menu" ? "menu" : undefined}
      aria-expanded={expanded === undefined ? undefined : String(expanded)}
      onClick={event => onActivate(entry, event.currentTarget)}
    >
      {entry.avatar ? <span className="app-head__avatar" aria-hidden="true">{entry.avatar}</span> : null}
      <span className="app-head__label">{entry.label}</span>
      {entry.count === undefined ? null : (
        <span className="app-head__count" hidden={!entry.count} data-has-error={String(Boolean(entry.hasError))}>{entry.count}</span>
      )}
    </button>
  );
}

// The tab row plus its one popover, which shows either the menu or the reason
// a disabled entry cannot be chosen. The popover is fixed to the viewport and
// portalled to <body>: the row scrolls sideways and the header stacks below
// the rail, so a child of the row would be clipped or covered.
export function AppHeaderNav({ store, onChoose }) {
  const model = useStoreSelector(store, selectModel);
  const [open, setOpen] = useState(null);
  const popoverRef = useRef(null);

  useEffect(() => {
    if (!open) return undefined;
    const document = open.trigger.ownerDocument;
    const view = document.defaultView;
    const row = open.trigger.closest(".app-head__row");
    const close = () => setOpen(null);
    const onClick = event => {
      if (!open.trigger.contains(event.target) && !popoverRef.current?.contains(event.target)) close();
    };
    const onKeyDown = event => {
      if (event.key !== "Escape") return;
      close();
      focusElement(open.trigger);
    };
    document.addEventListener("click", onClick);
    document.addEventListener("keydown", onKeyDown);
    view?.addEventListener("resize", close);
    row?.addEventListener("scroll", close);
    if (open.menu) focusFirstEnabledButton(popoverRef.current);
    return () => {
      document.removeEventListener("click", onClick);
      document.removeEventListener("keydown", onKeyDown);
      view?.removeEventListener("resize", close);
      row?.removeEventListener("scroll", close);
    };
  }, [open]);

  const activate = (entry, trigger) => {
    if (entry.kind !== "menu" && !entry.reason) {
      setOpen(null);
      onChoose(entry.id);
      return;
    }
    if (open?.id === entry.id) {
      setOpen(null);
      return;
    }
    const rect = trigger.getBoundingClientRect?.();
    setOpen({
      id: entry.id,
      menu: entry.kind === "menu",
      trigger,
      top: Math.round((rect?.bottom || 0) + 4),
      left: Math.round(rect?.left || 0),
    });
  };
  const choose = item => {
    if (item.reason) return;
    setOpen(null);
    onChoose(item.id);
  };

  const shown = open ? model.entries.find(entry => entry.id === open.id) : null;
  return (
    <>
      <nav className="app-head__nav" aria-label={model.navLabel}>
        {model.entries.map(entry => (
          <Entry key={entry.id} entry={entry} menuOpen={open?.id === entry.id} onActivate={activate} />
        ))}
      </nav>
      {shown ? createPortal(
        <div
          className="app-head-popover"
          ref={popoverRef}
          role={open.menu ? "menu" : "status"}
          style={{ top: `${open.top}px`, left: `clamp(8px, ${open.left}px, calc(100vw - 328px))` }}
        >
          {open.menu ? shown.items.map(item => (
            <button
              type="button"
              role="menuitem"
              className="app-head-popover__item"
              key={item.id}
              data-destination={item.id}
              title={item.title}
              aria-disabled={item.reason ? "true" : undefined}
              onClick={() => choose(item)}
            >
              <span>{item.label}</span>
              {item.reason ? <span className="app-head-popover__reason">{item.reason}</span> : null}
            </button>
          )) : <p className="app-head-popover__note">{shown.reason}</p>}
        </div>,
        open.trigger.ownerDocument.body,
      ) : null}
    </>
  );
}

// The whole header, for pages that do not ship its skeleton in their HTML.
export function AppHeader({ store, onChoose }) {
  const model = useStoreSelector(store, selectModel);
  return (
    <header className="app-head">
      <span className="app-head__brand">{model.brand}</span>
      <div className="app-head__row">
        <AppHeaderNav store={store} onChoose={onChoose} />
      </div>
    </header>
  );
}

export function AppFooterPm({ store }) {
  return <span className="app-foot__pm">{useStoreSelector(store, selectFooterPm)}</span>;
}

// The footer carries the version and the PM name only (design item 155).
export function AppFooter({ store }) {
  const version = useStoreSelector(store, selectFooterVersion);
  return (
    <footer className="app-foot">
      {version ? <span className="app-foot__version">{version}</span> : null}
      <AppFooterPm store={store} />
    </footer>
  );
}
