<script lang="ts">
    import { Canvas } from "@threlte/core";
    import { World } from "@threlte/rapier";
    import Scene from "./Scene.svelte";

    let {
        wsConnection,
        isHost,
        isSinglePlayer = false,
    } = $props<{
        wsConnection: any;
        isHost: boolean;
        isSinglePlayer?: boolean;
    }>();
</script>

<div class="w-full h-full">
    <Canvas>
        <World>
            <Scene {wsConnection} {isHost} {isSinglePlayer} />
        </World>
    </Canvas>
</div>
