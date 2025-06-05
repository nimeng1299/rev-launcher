<script lang="ts">
    import { Toaster } from "@skeletonlabs/skeleton-svelte";
    import { toaster } from "./toaster-svelte";
    import "../app.css";

    let { children } = $props();
</script>
<Toaster {toaster}></Toaster>
{@render children()}
