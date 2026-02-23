<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import Game from "$lib/components/Game.svelte";
  import QRScanner from "$lib/components/QRScanner.svelte";
  import { onMount, onDestroy, flushSync } from "svelte";
  import QRCode from "qrcode";
  import { initAudio } from "$lib/audio";

  let screen = $state<"menu" | "host" | "join" | "game" | "online_host" | "online_join">("menu");
  let isHost = $state(false);
  let isSinglePlayer = $state(false);
  let isOnline = $state(false);

  // Network State
  let qrCodes = $state<{ ip: string; url: string }[]>([]);
  let remoteServerIp = $state("");

  // Online State
  let roomCode = $state("");
  let joinCode = $state("");
  let onlineConnecting = $state(false);
  let onlineError = $state("");

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

  async function startOnlineHost() {
    initAudio();
    isHost = true;
    isOnline = true;
    onlineError = "";
    onlineConnecting = true;
    screen = "online_host";
    try {
      roomCode = await invoke("connect_relay_host") as string;
    } catch (e: any) {
      onlineError = e?.toString() ?? "Failed to connect";
    } finally {
      onlineConnecting = false;
    }
  }

  function startOnlineJoin() {
    initAudio();
    isHost = false;
    isOnline = true;
    isSinglePlayer = false;
    onlineError = "";
    joinCode = "";
    screen = "online_join";
  }

  async function joinOnlineRoom() {
    if (!joinCode.trim()) return;
    onlineConnecting = true;
    onlineError = "";
    try {
      await invoke("connect_relay_join", { roomCode: joinCode.trim() });
      screen = "game";
    } catch (e: any) {
      onlineError = e?.toString() ?? "Failed to join";
    } finally {
      onlineConnecting = false;
    }
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
  let unlistenRelay: UnlistenFn | null = null;
  let clientConnected = $state(false);

  onMount(async () => {
    unlisten = await listen<[string, string]>("udp-msg-received", (event) => {
      // If we are Host waiting on QR Code screen, any payload means client joined
      if (isHost && screen === "host") {
        clientConnected = true;
        initAudio();
        flushSync(() => { screen = "game"; });
      }
    });
    unlistenRelay = await listen("relay-peer-connected", () => {
      if (isHost && screen === "online_host") {
        flushSync(() => { screen = "game"; });
      }
    });
  });

  onDestroy(() => {
    if (unlisten) unlisten();
    if (unlistenRelay) unlistenRelay();
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
      >🌐 Host (Local)</button>
      <button
        class="w-full py-4 bg-emerald-600 text-white rounded-2xl text-lg font-bold hover:bg-emerald-500 active:scale-95 shadow-[0_0_24px_rgba(5,150,105,0.4)] transition-all uppercase tracking-widest"
        onclick={startJoin}
      >📷 Join (Local)</button>
      <div class="w-full border-t border-neutral-700 my-1"></div>
      <button
        class="w-full py-4 bg-orange-600 text-white rounded-2xl text-lg font-bold hover:bg-orange-500 active:scale-95 shadow-[0_0_24px_rgba(234,88,12,0.4)] transition-all uppercase tracking-widest"
        onclick={startOnlineHost}
      >🌍 Host (Online)</button>
      <button
        class="w-full py-4 bg-yellow-600 text-white rounded-2xl text-lg font-bold hover:bg-yellow-500 active:scale-95 shadow-[0_0_24px_rgba(202,138,4,0.4)] transition-all uppercase tracking-widest"
        onclick={startOnlineJoin}
      >🔑 Join (Online)</button>
      <div class="w-full border-t border-neutral-700 my-1"></div>
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
  {:else if screen === "online_host"}
    <div class="flex flex-col gap-5 items-center text-center p-8 w-full max-w-sm">
      <div class="text-3xl animate-pulse">🌍</div>
      <h2 class="text-2xl font-black text-orange-400">Online — Host</h2>
      {#if onlineConnecting}
        <p class="text-neutral-400 text-sm">Connecting to relay…</p>
      {:else if onlineError}
        <p class="text-red-400 text-sm">{onlineError}</p>
      {:else if roomCode}
        <p class="text-neutral-400 text-sm">Share this code with your friend</p>
        <div class="bg-neutral-800 border border-orange-500/40 rounded-2xl px-10 py-6">
          <p class="text-6xl font-black tracking-widest text-orange-400 font-mono">{roomCode}</p>
        </div>
        <p class="text-neutral-500 text-xs animate-pulse">Waiting for opponent to join…</p>
      {/if}
      <button
        class="w-full py-3 bg-neutral-700 text-white rounded-xl hover:bg-neutral-600 mt-2"
        onclick={() => { invoke("disconnect_relay").catch(() => {}); screen = "menu"; }}
      >Cancel</button>
    </div>
  {:else if screen === "online_join"}
    <div class="flex flex-col gap-5 items-center text-center p-8 w-full max-w-sm">
      <div class="text-3xl">🔑</div>
      <h2 class="text-2xl font-black text-yellow-400">Online — Join</h2>
      <p class="text-neutral-400 text-sm">Enter the 4-digit code from your friend</p>
      <input
        type="text"
        maxlength="4"
        placeholder="1234"
        bind:value={joinCode}
        class="w-full px-6 py-5 bg-neutral-800 text-white rounded-2xl border border-neutral-600 focus:border-yellow-500 outline-none font-mono text-4xl font-black tracking-widest text-center"
        onkeydown={(e) => e.key === 'Enter' && joinOnlineRoom()}
      />
      {#if onlineError}
        <p class="text-red-400 text-sm">{onlineError}</p>
      {/if}
      <button
        class="w-full py-4 bg-yellow-600 text-white rounded-2xl text-lg font-bold hover:bg-yellow-500 active:scale-95 disabled:opacity-40 uppercase tracking-widest"
        onclick={joinOnlineRoom}
        disabled={onlineConnecting || joinCode.length < 4}
      >{onlineConnecting ? "Connecting…" : "Join"}</button>
      <button
        class="w-full py-3 bg-neutral-700 text-white rounded-xl hover:bg-neutral-600"
        onclick={() => (screen = "menu")}
      >Cancel</button>
    </div>
  {:else if screen === "game"}
    <div class="absolute inset-0 w-full h-full">
      <Game {isHost} {isSinglePlayer} {isOnline} onBack={() => {
        screen = 'menu';
        isSinglePlayer = false;
        if (isOnline) { invoke("disconnect_relay").catch(() => {}); isOnline = false; }
      }} />
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
