<script lang="ts">
  interface MenuItem {
    label: string;
    action?: string;
    separator?: boolean;
    shortcut?: string;
  }

  interface MenuDef {
    label: string;
    items: MenuItem[];
  }

  let {
    projectPath,
    onAction,
  }: {
    projectPath: string;
    onAction: (action: string) => void;
  } = $props();

  let activeMenu: string | null = $state(null);

  const menus: MenuDef[] = [
    {
      label: "File",
      items: [
        { label: "Open Project...", action: "open-project", shortcut: "Cmd+O" },
        { label: "", separator: true },
        { label: "Close Project", action: "close-project" },
      ],
    },
    {
      label: "Edit",
      items: [
        { label: "Copy", action: "copy", shortcut: "Cmd+C" },
        { label: "Paste", action: "paste", shortcut: "Cmd+V" },
      ],
    },
    {
      label: "View",
      items: [
        { label: "Toggle Sidebar", action: "toggle-sidebar", shortcut: "Cmd+B" },
      ],
    },
    {
      label: "Agent",
      items: [
        { label: "Launch Agent...", action: "launch-agent" },
        { label: "", separator: true },
        { label: "List Terminals", action: "list-terminals" },
      ],
    },
    {
      label: "Settings",
      items: [{ label: "Preferences...", action: "open-settings" }],
    },
    {
      label: "Help",
      items: [
        { label: "About gwt", action: "about" },
      ],
    },
  ];

  function toggleMenu(name: string) {
    activeMenu = activeMenu === name ? null : name;
  }

  function closeMenu() {
    activeMenu = null;
  }

  function handleItemClick(action: string | undefined) {
    if (action) {
      onAction(action);
    }
    closeMenu();
  }

  function handleMouseEnter(name: string) {
    if (activeMenu !== null) {
      activeMenu = name;
    }
  }
</script>

<!-- svelte-ignore a11y_click_events_have_key_events -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="menubar-wrapper">
  <div class="menubar">
    {#each menus as menu}
      <div class="menu-wrapper">
        <button
          class="menu-item"
          class:active={activeMenu === menu.label}
          onclick={(e) => {
            e.stopPropagation();
            toggleMenu(menu.label);
          }}
          onmouseenter={() => handleMouseEnter(menu.label)}
        >
          {menu.label}
        </button>
        {#if activeMenu === menu.label}
          <div class="dropdown">
            {#each menu.items as item}
              {#if item.separator}
                <div class="dropdown-separator"></div>
              {:else}
                <button
                  class="dropdown-item"
                  onclick={() => handleItemClick(item.action)}
                >
                  <span class="dropdown-label">{item.label}</span>
                  {#if item.shortcut}
                    <span class="dropdown-shortcut">{item.shortcut}</span>
                  {/if}
                </button>
              {/if}
            {/each}
          </div>
        {/if}
      </div>
    {/each}
    <span class="project-name">{projectPath.split("/").pop()}</span>
  </div>
  {#if activeMenu !== null}
    <div class="backdrop" onclick={closeMenu}></div>
  {/if}
</div>

<style>
  .menubar-wrapper {
    position: relative;
  }

  .menubar {
    display: flex;
    align-items: center;
    height: var(--menubar-height);
    background-color: var(--bg-secondary);
    border-bottom: 1px solid var(--border-color);
    padding: 0 8px;
    -webkit-app-region: drag;
    user-select: none;
    position: relative;
    z-index: 101;
  }

  .menu-wrapper {
    position: relative;
    -webkit-app-region: no-drag;
  }

  .menu-item {
    -webkit-app-region: no-drag;
    background: none;
    border: none;
    color: var(--text-secondary);
    padding: 2px 8px;
    font-size: var(--ui-font-md);
    cursor: pointer;
    border-radius: 4px;
    font-family: inherit;
  }

  .menu-item:hover,
  .menu-item.active {
    background-color: var(--bg-hover);
    color: var(--text-primary);
  }

  .dropdown {
    position: absolute;
    top: 100%;
    left: 0;
    min-width: 200px;
    background: var(--bg-secondary);
    border: 1px solid var(--border-color);
    border-radius: 6px;
    padding: 4px 0;
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.4);
    z-index: 102;
  }

  .dropdown-item {
    display: flex;
    align-items: center;
    justify-content: space-between;
    width: 100%;
    padding: 6px 12px;
    background: none;
    border: none;
    color: var(--text-primary);
    font-size: var(--ui-font-md);
    cursor: pointer;
    font-family: inherit;
    text-align: left;
  }

  .dropdown-item:hover {
    background-color: var(--accent);
    color: var(--bg-primary);
  }

  .dropdown-item:hover .dropdown-shortcut {
    color: var(--bg-primary);
  }

  .dropdown-shortcut {
    font-size: var(--ui-font-sm);
    color: var(--text-muted);
    margin-left: 24px;
  }

  .dropdown-separator {
    height: 1px;
    background: var(--border-color);
    margin: 4px 8px;
  }

  .backdrop {
    position: fixed;
    top: 0;
    left: 0;
    right: 0;
    bottom: 0;
    z-index: 100;
  }

  .project-name {
    margin-left: auto;
    color: var(--text-muted);
    font-size: var(--ui-font-md);
    -webkit-app-region: no-drag;
  }
</style>
