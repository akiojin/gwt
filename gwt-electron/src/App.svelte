<script lang="ts">
  let status = $state('Connecting to server...');
</script>

<main>
  <h1>gwt</h1>
  <p>{status}</p>
</main>

<style>
  main {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    height: 100vh;
    font-family: var(--font-family);
    color: var(--color-text);
    background-color: var(--color-bg);
  }

  h1 {
    font-size: 2rem;
    margin-bottom: 0.5rem;
    color: var(--color-primary);
  }

  p {
    font-size: 1rem;
    color: var(--color-text-secondary);
  }
</style>
