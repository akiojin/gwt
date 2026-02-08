<script lang="ts">
  let { projectPath }: { projectPath: string } = $props();

  const menus = ["File", "Edit", "View", "Window", "Settings", "Help"];
  let activeMenu: string | null = $state(null);

  function toggleMenu(name: string) {
    activeMenu = activeMenu === name ? null : name;
  }

  function closeMenu() {
    activeMenu = null;
  }
</script>

<!-- svelte-ignore a11y_click_events_have_key_events -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="menubar" onclick={closeMenu}>
  {#each menus as menu}
    <button
      class="menu-item"
      class:active={activeMenu === menu}
      onclick={(e) => { e.stopPropagation(); toggleMenu(menu); }}
    >
      {menu}
    </button>
  {/each}
  <span class="project-name">{projectPath.split("/").pop()}</span>
</div>

<style>
  .menubar {
    display: flex;
    align-items: center;
    height: var(--menubar-height);
    background-color: var(--bg-secondary);
    border-bottom: 1px solid var(--border-color);
    padding: 0 8px;
    -webkit-app-region: drag;
    user-select: none;
  }

  .menu-item {
    -webkit-app-region: no-drag;
    background: none;
    border: none;
    color: var(--text-secondary);
    padding: 2px 8px;
    font-size: 12px;
    cursor: pointer;
    border-radius: 4px;
    font-family: inherit;
  }

  .menu-item:hover,
  .menu-item.active {
    background-color: var(--bg-hover);
    color: var(--text-primary);
  }

  .project-name {
    margin-left: auto;
    color: var(--text-muted);
    font-size: 12px;
    -webkit-app-region: no-drag;
  }
</style>
