const DEFAULT_IMAGE_MIME_TYPES = new Set(["image/png", "image/jpeg", "image/webp"]);

export function createTerminalContextMenuController({
  document,
  window,
  terminalRoot,
  readClipboardText = async () => "",
  readClipboardItems = async () => [],
  blobToBase64 = async () => null,
  pasteText = () => {},
  pasteImage = () => {},
  focusTerminal = () => {},
  supportedImageTypes = DEFAULT_IMAGE_MIME_TYPES,
} = {}) {
  if (!document || !window || !terminalRoot) {
    throw new Error("terminal context menu requires document, window, and terminalRoot");
  }

  const menu = document.createElement("div");
  menu.className = "terminal-context-menu";
  menu.setAttribute("role", "menu");
  menu.setAttribute("aria-label", "Terminal");
  menu.hidden = true;

  const pasteButton = document.createElement("button");
  pasteButton.type = "button";
  pasteButton.className = "terminal-context-menu__item";
  pasteButton.setAttribute("role", "menuitem");
  pasteButton.textContent = "Paste";
  menu.appendChild(pasteButton);
  document.body.appendChild(menu);

  function show(event) {
    event.preventDefault();
    event.stopPropagation();
    menu.hidden = false;
    menu.style.left = `${event.clientX || 0}px`;
    menu.style.top = `${event.clientY || 0}px`;
    pasteButton.focus?.();
  }

  function hide() {
    menu.hidden = true;
  }

  async function pasteFromClipboard() {
    hide();
    try {
      const image = await readClipboardImage({
        readClipboardItems,
        blobToBase64,
        supportedImageTypes,
      });
      if (image) {
        await pasteImage(image);
        return true;
      }

      const text = await readClipboardText();
      if (!text) {
        return false;
      }
      await pasteText(text);
      return true;
    } catch (_error) {
      return false;
    } finally {
      focusTerminal();
    }
  }

  const handleContextMenu = (event) => {
    show(event);
  };

  const handlePasteClick = (event) => {
    event.preventDefault();
    event.stopPropagation();
    void pasteFromClipboard();
  };

  const handlePointerDown = (event) => {
    if (menu.hidden || menu.contains(event.target)) {
      return;
    }
    hide();
  };

  const handleKeyDown = (event) => {
    if (event.key === "Escape") {
      hide();
    }
  };

  const handleDismiss = () => {
    hide();
  };

  terminalRoot.addEventListener("contextmenu", handleContextMenu);
  pasteButton.addEventListener("click", handlePasteClick);
  document.addEventListener("pointerdown", handlePointerDown, true);
  document.addEventListener("keydown", handleKeyDown, true);
  document.addEventListener("wheel", handleDismiss, true);
  window.addEventListener("blur", handleDismiss);

  return {
    hide,
    pasteFromClipboard,
    dispose() {
      terminalRoot.removeEventListener("contextmenu", handleContextMenu);
      pasteButton.removeEventListener("click", handlePasteClick);
      document.removeEventListener("pointerdown", handlePointerDown, true);
      document.removeEventListener("keydown", handleKeyDown, true);
      document.removeEventListener("wheel", handleDismiss, true);
      window.removeEventListener("blur", handleDismiss);
      menu.remove();
    },
  };
}

async function readClipboardImage({
  readClipboardItems,
  blobToBase64,
  supportedImageTypes,
}) {
  let items = [];
  try {
    items = await readClipboardItems();
  } catch (_error) {
    return null;
  }
  for (const item of Array.from(items || [])) {
    for (const mimeType of item?.types || []) {
      if (!supportedImageTypes.has(mimeType)) {
        continue;
      }
      const blob = await item.getType?.(mimeType);
      if (!blob) {
        continue;
      }
      const dataBase64 = await blobToBase64(blob);
      if (!dataBase64) {
        continue;
      }
      return {
        dataBase64,
        mimeType,
        filename: null,
      };
    }
  }
  return null;
}
