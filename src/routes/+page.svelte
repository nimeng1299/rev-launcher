<script lang="ts">
    import { invoke } from "@tauri-apps/api/core";
    import { toaster } from "./toaster-svelte";
    import { Window } from "@tauri-apps/api/window";
    import { AppBar } from "@skeletonlabs/skeleton-svelte";

    import { Minus } from "lucide-svelte";
    import { Maximize } from "lucide-svelte";
    import { X } from "lucide-svelte";
    import { Play } from "lucide-svelte";

    const appWindow = new Window("main");
    let greetMsg = $state("");

    async function greet(event: Event) {
        event.preventDefault();
        // Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
        greetMsg = await invoke("file_dialog", {
            filters: [["Text Files", ["txt"]]],
            setDirectory: "/",
        });
    }
</script>

<main>
    <div class="background-layer"></div>
    <div data-tauri-drag-region class="titleBar">
        <h1>rev launcher</h1>
        <div>
            <button
                type="button"
                class="btn-icon preset-tonal-surface"
                onclick={() => {
                    appWindow.minimize();
                }}><Minus /></button
            >
            <button
                type="button"
                class="btn-icon preset-tonal-surface"
                onclick={() => {
                    appWindow.maximize();
                }}><Maximize /></button
            >
            <button
                type="button"
                class="btn-icon preset-tonal-surface"
                onclick={() => {
                    appWindow.close();
                }}><X /></button
            >
        </div>
    </div>
    <button
        type="button"
        class="btn preset-filled"
        onclick={() => {
            toaster.info({ title: "Toast!" });
        }}>toast</button
    >

    <div style="display: flex;">
        <div class="nav">
            <button type="button" class="btn preset-tonal-surface"
                ><Play /><span>游戏</span></button
            >
            <button type="button" class="btn preset-tonal-surface"
                ><Play /><span>游戏</span></button
            >
            <button type="button" class="btn preset-tonal-surface"
                ><Play /><span>游戏</span></button
            >
            <button type="button" class="btn preset-tonal-surface"
                ><Play /><span>游戏</span></button
            >
        </div>
        <div style="flex: 1;"></div>
    </div>
</main>

<style>
    .background-layer {
        position: fixed;
        top: 0;
        left: 0;
        right: 0;
        bottom: 0;
        z-index: -999;

        background-size: cover;
        background-position: center;
    }

    .titleBar {
        display: flex;
        justify-content: space-between;
        flex-grow: 1;
        align-items: center;
        width: 100%;
        padding: 0 1rem;
        min-width: 100%;
        max-width: 100%;
        padding-top: 5px;
        z-index: 999;
    }

    .nav {
        width: 200px;
        display: flex;
        flex-direction: column;
        padding: 2px 10px;
        gap: 10px;
    }
</style>
