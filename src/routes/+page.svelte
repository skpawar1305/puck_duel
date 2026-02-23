<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import Game from "$lib/components/Game.svelte";
  import QRScanner from "$lib/components/QRScanner.svelte";
  import { onMount, onDestroy } from "svelte";
  import QRCode from "qrcode";
  import { initAudio } from "$lib/audio";

  let screen = $state<"menu" | "host" | "join" | "game">("menu");
  let isHost = $state(false);
  let isSinglePlayer = $state(false);

  // Network State
  let localIp = $state("");
  let qrCodeDataUrl = $state("");
  let remoteServerIp = $state("");

  async function startHost() {
    initAudio();
    isHost = true;
    try {
      const ip = (await invoke("get_local_ip")) as string;
      localIp = ip;
      const url = `${ip}`;

      qrCodeDataUrl = await QRCode.toDataURL(url, { width: 300, margin: 2 });

      await invoke("start_udp_host");

      screen = "host";
    } catch (e) {
      console.error(e);
      alert("Failed to start host");
    }
  }

  function startSinglePlayer() {
    initAudio();
    isHost = true;
    isSinglePlayer = true;
    screen = "game";
  }

  function startJoin() {
    initAudio();
    isHost = false;
    isSinglePlayer = false;
    screen = "join";
  }

  async function onQrScanned(ip: string) {
    if (ip && ip.includes(".")) {
      remoteServerIp = ip;

      // Connect UDP
      try {
        await invoke("connect_udp_client", { hostIp: ip });
        console.log("Connected to UDP Host at", ip);
        screen = "game";
      } catch (e) {
        console.error("Failed to connect UDP client", e);
      }
    }
  }

  // Once host receives an event, it switches screen
  let unlisten: UnlistenFn | null = null;
  onMount(async () => {
    unlisten = await listen<[string, string]>("udp-msg-received", (event) => {
      // If we are Host waiting on QR Code screen, any ping payload means client joined
      if (isHost && screen === "host") {
        screen = "game";
      }
    });
  });

  onDestroy(() => {
    if (unlisten) unlisten();
  });
</script>

<main
  class="w-screen h-screen bg-neutral-900 text-white flex flex-col items-center justify-center overflow-hidden"
>
  {#if screen === "menu"}
    <div class="flex flex-col gap-6 items-center">
      <h1
        class="text-4xl font-bold bg-gradient-to-r from-cyan-400 to-blue-500 bg-clip-text text-transparent mb-8"
      >
        Air Hockey VR
      </h1>
      <button
        class="px-8 py-4 bg-blue-600 text-white rounded-2xl text-xl font-bold hover:bg-blue-500 shadow-[0_0_20px_rgba(37,99,235,0.5)] transition-all uppercase tracking-widest min-w-48"
        onclick={startHost}
      >
        Host Game
      </button>
      <button
        class="px-8 py-4 bg-purple-600 text-white rounded-2xl text-xl font-bold hover:bg-purple-500 shadow-[0_0_20px_rgba(147,51,234,0.5)] transition-all uppercase tracking-widest min-w-48"
        onclick={startSinglePlayer}
      >
        Single Player
      </button>
      <button
        class="px-8 py-4 bg-emerald-600 text-white rounded-2xl text-xl font-bold hover:bg-emerald-500 shadow-[0_0_20px_rgba(5,150,105,0.5)] transition-all uppercase tracking-widest min-w-48"
        onclick={startJoin}
      >
        Join Game
      </button>
    </div>
  {:else if screen === "host"}
    <div
      class="flex flex-col gap-6 items-center text-center p-8 bg-neutral-800 rounded-3xl shadow-2xl"
    >
      <h2 class="text-2xl font-bold text-cyan-400">Waiting for Opponent...</h2>
      <p class="text-neutral-400">Scan this QR code from the other device</p>
      {#if qrCodeDataUrl}
        <div class="bg-white p-4 rounded-xl">
          <img src={qrCodeDataUrl} alt="QR Code" class="w-64 h-64" />
        </div>
      {/if}
      <button
        class="mt-4 px-6 py-3 bg-neutral-700 text-white rounded-xl hover:bg-neutral-600"
        onclick={() => (screen = "menu")}
      >
        Cancel
      </button>
    </div>
  {:else if screen === "join"}
    <div class="flex flex-col gap-6 items-center w-full h-full p-4">
      <h2 class="text-2xl font-bold text-emerald-400 mt-8 mb-4">
        Scan Host QR
      </h2>
      <div
        class="w-full max-w-md aspect-square rounded-3xl overflow-hidden shadow-2xl bg-black border-4 border-neutral-700 relative"
      >
        <QRScanner onScan={onQrScanned} />
        <div
          class="absolute inset-0 border-2 border-emerald-500 block m-12 opacity-50 rounded-xl pointer-events-none"
        ></div>
      </div>
      <button
        class="mt-8 px-6 py-3 bg-neutral-700 text-white rounded-xl hover:bg-neutral-600"
        onclick={() => (screen = "menu")}
      >
        Cancel
      </button>
    </div>
  {:else if screen === "game"}
    <div class="absolute inset-0 w-full h-full">
      <Game {isHost} {isSinglePlayer} wsConnection={null as any} />
    </div>
  {/if}
</main>

<style>
  :global(body) {
    margin: 0;
    padding: 0;
    font-family: "Inter", system-ui, sans-serif;
    user-select: none;
    -webkit-user-select: none;
    touch-action: none; /* Crucial for games to prevent zooming/scrolling */
  }
</style>
