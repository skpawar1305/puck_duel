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
  let qrCodes = $state<{ ip: string; url: string }[]>([]);
  let remoteServerIp = $state("");

  async function startHost() {
    initAudio();
    isHost = true;
    try {
      const ips = (await invoke("get_local_ips")) as string[];

      const codes = [];
      for (const ip of ips) {
        const url = await QRCode.toDataURL(ip, { width: 200, margin: 2 });
        codes.push({ ip, url });
      }
      qrCodes = codes;

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
    <div class="flex flex-col gap-5 items-center px-8 w-full max-w-sm">
      <div class="flex flex-col items-center mb-6">
        <div class="text-5xl mb-3">🏒</div>
        <h1 class="text-4xl font-black tracking-tight bg-gradient-to-r from-cyan-400 to-blue-500 bg-clip-text text-transparent">
          Air Hockey
        </h1>
        <p class="text-neutral-500 text-sm mt-1 tracking-widest uppercase">First to 6 wins</p>
      </div>
      <button
        class="w-full py-4 bg-blue-600 text-white rounded-2xl text-lg font-bold hover:bg-blue-500 active:scale-95 shadow-[0_0_24px_rgba(37,99,235,0.4)] transition-all uppercase tracking-widest"
        onclick={startHost}
      >🌐 Host Game</button>
      <button
        class="w-full py-4 bg-emerald-600 text-white rounded-2xl text-lg font-bold hover:bg-emerald-500 active:scale-95 shadow-[0_0_24px_rgba(5,150,105,0.4)] transition-all uppercase tracking-widest"
        onclick={startJoin}
      >📷 Join Game</button>
      <button
        class="w-full py-4 bg-purple-600 text-white rounded-2xl text-lg font-bold hover:bg-purple-500 active:scale-95 shadow-[0_0_24px_rgba(147,51,234,0.4)] transition-all uppercase tracking-widest"
        onclick={startSinglePlayer}
      >🤖 vs AI</button>
    </div>
  {:else if screen === "host"}
    <div class="flex flex-col gap-5 items-center text-center p-8 w-full max-w-sm">
      <div class="text-3xl animate-pulse">⏳</div>
      <h2 class="text-2xl font-black text-cyan-400">Waiting for opponent…</h2>
      <p class="text-neutral-500 text-sm">Have them scan this QR code</p>
      {#if qrCodes.length > 0}
        <div class="flex flex-col gap-4 items-center w-full">
          {#each qrCodes as code}
            <div class="bg-white p-4 rounded-2xl shadow-xl">
              <img src={code.url} alt="QR Code" class="w-52 h-52" />
              <p class="text-xs font-mono text-black text-center mt-2 font-bold">{code.ip}</p>
            </div>
          {/each}
        </div>
      {/if}
      <button
        class="w-full py-3 bg-neutral-700 text-white rounded-xl hover:bg-neutral-600 mt-2"
        onclick={() => (screen = "menu")}
      >Cancel</button>
    </div>
  {:else if screen === "join"}
    <div class="flex flex-col gap-4 items-center w-full h-full p-6 pt-10">
      <h2 class="text-2xl font-black text-emerald-400">Scan Host QR Code</h2>
      <div class="w-full max-w-sm aspect-square rounded-3xl overflow-hidden shadow-2xl bg-black border-2 border-emerald-600/50 relative">
        <QRScanner onScan={onQrScanned} />
        <div class="absolute inset-10 border-2 border-emerald-400 rounded-xl pointer-events-none opacity-60"></div>
      </div>
      <p class="text-neutral-500 text-xs">— or enter IP manually —</p>
      <div class="flex gap-2 w-full max-w-sm">
        <input
          type="text"
          placeholder="192.168.x.x"
          bind:value={remoteServerIp}
          class="flex-1 px-4 py-3 bg-neutral-800 text-white rounded-xl border border-neutral-600 focus:border-emerald-500 outline-none font-mono text-sm"
          onkeydown={(e) => e.key === 'Enter' && onQrScanned(remoteServerIp)}
        />
        <button
          class="px-5 py-3 bg-emerald-600 text-white rounded-xl font-bold hover:bg-emerald-500 active:scale-95"
          onclick={() => onQrScanned(remoteServerIp)}
        >Connect</button>
      </div>
      <button
        class="w-full max-w-sm py-3 bg-neutral-700 text-white rounded-xl hover:bg-neutral-600"
        onclick={() => (screen = "menu")}
      >Cancel</button>
    </div>
  {:else if screen === "game"}
    <div class="absolute inset-0 w-full h-full">
      <Game {isHost} {isSinglePlayer} onBack={() => { screen = 'menu'; isSinglePlayer = false; }} />
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
