<script lang="ts">
    import { Html5Qrcode } from "html5-qrcode";
    import { onMount, onDestroy } from "svelte";

    let { onScan } = $props<{ onScan: (result: string) => void }>();

    let scanner: Html5Qrcode | null = null;
    let cameras: { id: string; label: string }[] = [];
    let camIdx = 0;
    let running = false;
    let errMsg = $state('');

    async function startCam(camId: string | { facingMode: string }) {
        if (!scanner) return;
        if (running) { await scanner.stop().catch(() => {}); running = false; }
        try {
            await scanner.start(
                camId as any,
                { fps: 10, qrbox: { width: 200, height: 200 } },
                (decoded) => { onScan(decoded); },
                () => {}
            );
            running = true;
            errMsg = '';
        } catch (e) {
            errMsg = 'Camera error: tap 🔄 or check permissions';
        }
    }

    async function flipCamera() {
        if (cameras.length < 2) return;
        camIdx = (camIdx + 1) % cameras.length;
        await startCam(cameras[camIdx].id);
    }

    onMount(async () => {
        scanner = new Html5Qrcode("qr-video");
        try {
            cameras = await Html5Qrcode.getCameras();
            const backIdx = cameras.findIndex(c =>
                /back|rear|env/i.test(c.label));
            camIdx = backIdx >= 0 ? backIdx : cameras.length - 1;
            await startCam(cameras.length > 0 ? cameras[camIdx].id : { facingMode: 'environment' });
        } catch {
            await startCam({ facingMode: 'environment' });
        }
    });

    onDestroy(async () => {
        if (scanner && running) await scanner.stop().catch(() => {});
    });
</script>

<div class="relative w-full h-full bg-black overflow-hidden rounded-xl">
    <div id="qr-video" class="w-full h-full [&_video]:w-full [&_video]:h-full [&_video]:object-cover"></div>
    <button
        class="absolute bottom-3 right-3 w-11 h-11 bg-black/60 text-white rounded-full text-xl flex items-center justify-center active:scale-90"
        onclick={flipCamera}
    >🔄</button>
    {#if errMsg}
    <p class="absolute inset-0 flex items-center justify-center text-red-400 text-sm p-6 text-center bg-black/70">{errMsg}</p>
    {/if}
</div>

<style>
    :global(#qr-video video) { width: 100% !important; height: 100% !important; object-fit: cover !important; }
    :global(#qr-video img) { display: none !important; }
    :global(#qr-video__scan_region) { background: transparent !important; }
</style>
