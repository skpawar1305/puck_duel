<script lang="ts">
    import { Html5QrcodeScanner } from "html5-qrcode";
    import { onMount, onDestroy } from "svelte";

    let { onScan } = $props<{ onScan: (result: string) => void }>();
    let scannerId = "reader";
    let scanner: Html5QrcodeScanner | null = null;

    onMount(() => {
        scanner = new Html5QrcodeScanner(
            scannerId,
            { fps: 10, qrbox: { width: 250, height: 250 } },
            false,
        );
        scanner.render(
            (decodedText: string) => {
                onScan(decodedText);
                if (scanner) scanner.clear().catch(() => {});
            },
            () => {},
        );
    });

    onDestroy(() => {
        if (scanner) {
            scanner.clear().catch(() => {});
        }
    });
</script>

<div
    id={scannerId}
    class="w-full h-full bg-black text-white [&_video]:object-cover overflow-hidden rounded-xl"
></div>

<style>
    :global(#reader__scan_region) {
        background: transparent !important;
    }
    :global(#reader video) {
        width: 100% !important;
        height: 100% !important;
        object-fit: cover !important;
    }
    :global(#reader__dashboard_section_csr span) {
        color: white !important;
    }
</style>
