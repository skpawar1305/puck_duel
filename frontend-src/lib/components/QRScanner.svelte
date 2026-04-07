<script lang="ts">
    import { Html5Qrcode } from "html5-qrcode";
    import { onMount, onDestroy } from "svelte";

    let { onScan } = $props<{ onScan: (result: string) => void }>();

    const CAM_KEY = 'qr_camera_id';

    let scanner: Html5Qrcode | null = null;
    let cameras = $state<{ id: string; label: string }[]>([]);
    let activeCamId = $state('');
    let running = false;
    let errMsg = $state('');

    function shortLabel(cam: { id: string; label: string }, i: number): string {
        const l = cam.label;
        if (/back|rear|env/i.test(l))  return '📷 Back';
        if (/front|user|face/i.test(l)) return '🤳 Front';
        if (/wide/i.test(l))           return '🔭 Wide';
        if (/tele|zoom/i.test(l))      return '🔬 Tele';
        return `Cam ${i + 1}`;
    }

    async function startCam(camId: string) {
        if (!scanner) return;
        if (running) { await scanner.stop().catch(() => {}); running = false; }
        try {
            await scanner.start(
                camId,
                { fps: 10, qrbox: { width: 200, height: 200 } },
                (decoded) => { onScan(decoded); },
                () => {}
            );
            running = true;
            activeCamId = camId;
            localStorage.setItem(CAM_KEY, camId);
            errMsg = '';
        } catch {
            errMsg = 'Camera error — check permissions';
        }
    }

    onMount(async () => {
        scanner = new Html5Qrcode("qr-video");
        try {
            cameras = await Html5Qrcode.getCameras();
            if (cameras.length === 0) throw new Error('no cameras');
            const saved = localStorage.getItem(CAM_KEY);
            const savedCam = saved && cameras.find(c => c.id === saved);
            const backCam  = cameras.find(c => /back|rear|env/i.test(c.label));
            const pick     = savedCam ?? backCam ?? cameras[cameras.length - 1];
            if (!pick) throw new Error('no cameras');
            await startCam(pick.id);
        } catch {
            errMsg = 'Could not access camera — check permissions';
        }
    });

    onDestroy(async () => {
        if (scanner && running) await scanner.stop().catch(() => {});
    });
</script>

<div class="qr-root relative w-full h-full bg-black overflow-hidden rounded-xl">
    <div id="qr-video" class="w-full h-full [&_video]:w-full [&_video]:h-full [&_video]:object-cover"></div>

    {#if cameras.length > 1}
    <div class="absolute bottom-3 left-1/2 -translate-x-1/2 flex gap-2">
        {#each cameras as cam, i}
        <button
            class="px-3 py-1.5 rounded-full text-xs font-semibold transition-all active:scale-90
                   {activeCamId === cam.id
                     ? 'bg-white text-black shadow-lg'
                     : 'bg-black/60 text-white/80 border border-white/20'}"
            onclick={() => startCam(cam.id)}
        >{shortLabel(cam, i)}</button>
        {/each}
    </div>
    {/if}

    {#if errMsg}
    <p class="absolute inset-0 flex items-center justify-center text-red-400 text-sm p-6 text-center bg-black/70">{errMsg}</p>
    {/if}
</div>

<style>
    .qr-root {}
</style>
