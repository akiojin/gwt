<script lang="ts">
  let {
    featureTitle = $bindable(""),
    featureDescription = $bindable(""),
    useCase = $bindable(""),
    expectedBenefit = $bindable(""),
  }: {
    featureTitle?: string;
    featureDescription?: string;
    useCase?: string;
    expectedBenefit?: string;
  } = $props();
</script>

<div class="form-section">
  <label class="form-label" for="feature-title">Title</label>
  <input id="feature-title" class="form-input" type="text" bind:value={featureTitle} placeholder="Feature summary" />
</div>

<div class="form-section">
  <label class="form-label" for="feature-desc">Description</label>
  <textarea id="feature-desc" class="form-textarea" bind:value={featureDescription} placeholder="Describe the feature you'd like" rows="3"></textarea>
</div>

<div class="form-section">
  <label class="form-label" for="feature-usecase">Use Case</label>
  <textarea id="feature-usecase" class="form-textarea" bind:value={useCase} placeholder="What problem does this solve?" rows="2"></textarea>
</div>

<div class="form-section">
  <label class="form-label" for="feature-benefit">Expected Benefit</label>
  <textarea id="feature-benefit" class="form-textarea" bind:value={expectedBenefit} placeholder="How would this improve your workflow?" rows="2"></textarea>
</div>

<style>
  .form-section { display: flex; flex-direction: column; gap: 6px; }
  .form-label { font-size: var(--ui-font-base, 14px); color: var(--text-primary); font-weight: 600; line-height: 1.4; }
  .form-input, .form-textarea {
    background: var(--bg-primary);
    border: 1px solid var(--border-color);
    border-radius: 6px;
    padding: 8px 12px;
    color: var(--text-primary);
    font-family: inherit;
    font-size: var(--ui-font-lg, 14px);
    line-height: 1.5;
  }
  .form-textarea { resize: vertical; min-height: 52px; }
  .form-input::placeholder, .form-textarea::placeholder { color: var(--text-muted); opacity: 0.9; }
  .form-input:focus, .form-textarea:focus { outline: none; border-color: var(--accent); }
</style>
