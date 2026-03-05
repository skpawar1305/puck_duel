<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import Game from "$lib/components/Game.svelte";
  import { onMount, onDestroy, flushSync } from "svelte";
  import { initAudio } from "$lib/audio";

  let screen = $state<"menu" | "host" | "join" | "game" | "online_host" | "online_join">("menu");
  let isHost = $state(false);
  let isSinglePlayer = $state(false);

  // LAN join — list of discovered peers { name, nodeAddrJson }
  let discoveredPeers = $state<{ name: string; nodeAddrJson: string }[]>([]);
  let connectingPeer = $state<string | null>(null);

  // Online state
  let roomCode = $state("");
  let joinCode = $state("");
  let onlineConnecting = $state(false);
  let onlineError = $state("");

  // ── LAN Host ─────────────────────────────────────────────────────────
  async function startHost() {
    initAudio();
    isHost = true;
    screen = "host";
    try {
      // Register our mDNS service and start browsing
      await invoke("discover_lan");
      // Accept exactly one incoming QUIC connection (peer-connected fires on success)
      await invoke("start_accept_loop");
    } catch (e) {
      console.error(e);
      alert("Failed to start host: " + e);
      screen = "menu";
    }
  }

  // ── LAN Join ─────────────────────────────────────────────────────────
  async function startJoin() {
    initAudio();
    isHost = false;
    discoveredPeers = [];
    screen = "join";
    try {
      // Register our mDNS service and browse for the host (emits peer-discovered events)
      await invoke("discover_lan");
    } catch (e) {
      console.error(e);
    }
  }

  async function connectToPeer(nodeAddrJson: string, label: string) {
    connectingPeer = label;
    try {
      await invoke("connect_to_peer", { nodeAddrJson });
      // peer-connected event will fire → screen = "game"
    } catch (e: any) {
      connectingPeer = null;
      alert("Failed to connect: " + e);
    }
  }

  // ── Single Player ─────────────────────────────────────────────────────
  function startSinglePlayer() {
    initAudio();
    isHost = true;
    isSinglePlayer = true;
    screen = "game";
  }

  // ── Online Host ───────────────────────────────────────────────────────
  async function startOnlineHost() {
    initAudio();
    isHost = true;
    onlineError = "";
    onlineConnecting = true;
    screen = "online_host";
    try {
      roomCode = (await invoke("host_online")) as string;
    } catch (e: any) {
      onlineError = e?.toString() ?? "Failed to connect";
    } finally {
      onlineConnecting = false;
    }
  }

  // ── Online Join ────────────────────────────────────────────────────────
  function startOnlineJoin() {
    initAudio();
    isHost = false;
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
      await invoke("join_online", { roomCode: joinCode.trim() });
      // peer-connected event fires → screen = "game"
    } catch (e: any) {
      onlineError = e?.toString() ?? "Failed to join";
      onlineConnecting = false;
    }
  }

  // ── Event listeners ────────────────────────────────────────────────────
  let unlistenPeerConnected: UnlistenFn | null = null;
  let unlistenPeerDiscovered: UnlistenFn | null = null;

  onMount(async () => {
    // Any peer-connected event navigates to game (works for LAN and online)
    unlistenPeerConnected = await listen("peer-connected", () => {
      connectingPeer = null;
      flushSync(() => { screen = "game"; });
    });

    // Discovered LAN peers — accumulate into list (avoid duplicates by nodeAddrJson)
    unlistenPeerDiscovered = await listen<string>("peer-discovered", (event) => {
      const nodeAddrJson = event.payload;
      if (!discoveredPeers.some(p => p.nodeAddrJson === nodeAddrJson)) {
        // Use a short label for display
        const label = "Peer " + (discoveredPeers.length + 1);
        discoveredPeers = [...discoveredPeers, { name: label, nodeAddrJson }];
      }
    });
  });

  onDestroy(() => {
    if (unlistenPeerConnected) unlistenPeerConnected();
    if (unlistenPeerDiscovered) unlistenPeerDiscovered();
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
      >📡 Join (Local)</button>
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
      <p class="text-neutral-500 text-sm">Make sure both devices are on the same Wi-Fi network</p>
      <p class="text-neutral-600 text-xs">Broadcasting via mDNS · Listening for peer</p>
      <button
        class="w-full py-3 bg-neutral-700 text-white rounded-xl hover:bg-neutral-600 mt-2"
        onclick={() => (screen = "menu")}
      >Cancel</button>
    </div>

  {:else if screen === "join"}
    <div class="flex flex-col gap-4 items-center w-full p-8 max-w-sm">
      <h2 class="text-2xl font-black text-emerald-400">Join (Local)</h2>
      <p class="text-neutral-500 text-sm text-center">Discovering hosts on your Wi-Fi network…</p>

      {#if discoveredPeers.length === 0}
        <div class="flex items-center gap-2 text-neutral-400 text-sm mt-4 animate-pulse">
          <span>🔍</span>
          <span>Scanning…</span>
        </div>
      {:else}
        <div class="w-full flex flex-col gap-3 mt-2">
          {#each discoveredPeers as peer}
            <button
              class="w-full py-4 bg-emerald-700 text-white rounded-2xl text-base font-bold hover:bg-emerald-600 active:scale-95 disabled:opacity-50 transition-all"
              disabled={connectingPeer !== null}
              onclick={() => connectToPeer(peer.nodeAddrJson, peer.name)}
            >
              {connectingPeer === peer.name ? "Connecting…" : "🎮 " + peer.name}
            </button>
          {/each}
        </div>
      {/if}

      <button
        class="w-full py-3 bg-neutral-700 text-white rounded-xl hover:bg-neutral-600 mt-4"
        onclick={() => { connectingPeer = null; screen = "menu"; }}
      >Cancel</button>
    </div>

  {:else if screen === "online_host"}
    <div class="flex flex-col gap-5 items-center text-center p-8 w-full max-w-sm">
      <div class="text-3xl animate-pulse">🌍</div>
      <h2 class="text-2xl font-black text-orange-400">Online — Host</h2>
      {#if onlineConnecting}
        <p class="text-neutral-400 text-sm">Registering with signaling server…</p>
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
        onclick={() => (screen = "menu")}
      >Cancel</button>
    </div>

  {:else if screen === "online_join"}
    <div class="flex flex-col gap-5 items-center text-center p-8 w-full max-w-sm">
      <div class="text-3xl">🔑</div>
      <h2 class="text-2xl font-black text-yellow-400">Online — Join</h2>
      <p class="text-neutral-400 text-sm">Enter the 6-character code from your friend</p>
      <input
        type="text"
        maxlength="6"
        placeholder="ABC123"
        bind:value={joinCode}
        class="w-full px-6 py-5 bg-neutral-800 text-white rounded-2xl border border-neutral-600 focus:border-yellow-500 outline-none font-mono text-4xl font-black tracking-widest text-center uppercase"
        onkeydown={(e) => e.key === 'Enter' && joinOnlineRoom()}
      />
      {#if onlineError}
        <p class="text-red-400 text-sm">{onlineError}</p>
      {/if}
      <button
        class="w-full py-4 bg-yellow-600 text-white rounded-2xl text-lg font-bold hover:bg-yellow-500 active:scale-95 disabled:opacity-40 uppercase tracking-widest"
        onclick={joinOnlineRoom}
        disabled={onlineConnecting || joinCode.length < 6}
      >{onlineConnecting ? "Connecting…" : "Join"}</button>
      <button
        class="w-full py-3 bg-neutral-700 text-white rounded-xl hover:bg-neutral-600"
        onclick={() => (screen = "menu")}
      >Cancel</button>
    </div>

  {:else if screen === "game"}
    <div class="absolute inset-0 w-full h-full">
      <Game {isHost} {isSinglePlayer} onBack={() => {
        screen = "menu";
        isSinglePlayer = false;
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
    touch-action: none;
  }
</style>
