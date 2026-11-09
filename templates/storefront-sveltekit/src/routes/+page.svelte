<script lang="ts">
  let { data } = $props();
</script>

<main>
  <h1>{data.message}</h1>
  <p data-testid="context">{data.store ?? "no-store"}:{data.deployment ?? "no-deployment"}</p>
</main>
