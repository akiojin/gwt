<script lang="ts">
  import MenuBar from "./lib/components/MenuBar.svelte";
  import Sidebar from "./lib/components/Sidebar.svelte";
  import MainArea from "./lib/components/MainArea.svelte";
  import StatusBar from "./lib/components/StatusBar.svelte";
  import OpenProject from "./lib/components/OpenProject.svelte";

  let projectPath: string | null = $state(null);

  function handleProjectOpen(path: string) {
    projectPath = path;
  }
</script>

{#if projectPath === null}
  <OpenProject onOpen={handleProjectOpen} />
{:else}
  <div class="app-layout">
    <MenuBar {projectPath} />
    <div class="app-body">
      <Sidebar />
      <MainArea />
    </div>
    <StatusBar {projectPath} />
  </div>
{/if}

<style>
  .app-layout {
    display: flex;
    flex-direction: column;
    height: 100vh;
    width: 100vw;
  }

  .app-body {
    display: flex;
    flex: 1;
    overflow: hidden;
  }
</style>
